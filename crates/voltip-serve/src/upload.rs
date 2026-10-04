//! Reading the multipart request of `POST /v1/audio/transcriptions` (docs/dictation.md §23.4): the
//! `file` part is decoded as it arrives ([`crate::wav`]), the other parts are short text fields.
//! At most [`MAX_PARTS`] parts, each text field at most [`MAX_FIELD_BYTES`], one file; a body that
//! stops arriving for [`IDLE`] is given up.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::extract::Multipart;
use axum::extract::multipart::Field;
use voltip_core::serve::ServeRequest;

use crate::openai::{ApiError, ResponseFormat};
use crate::wav::{self, DecodeError};

/// Most parts a request may have.
pub const MAX_PARTS: usize = 16;
/// Longest text field, in bytes.
pub const MAX_FIELD_BYTES: usize = 4096;
/// Bytes a WAV may carry after its samples (trailing chunks).
pub const MAX_TRAILING_BYTES: u64 = 1024 * 1024;
/// How long the body may stop arriving.
pub const IDLE: Duration = Duration::from_secs(60);

/// The decoded audio: a 16 kHz mono 16-bit PCM file, removed when this is dropped.
#[derive(Debug)]
pub struct TempPcm {
    path: PathBuf,
    samples: u64,
}

impl TempPcm {
    /// The file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Samples in it.
    pub fn samples(&self) -> u64 {
        self.samples
    }
}

impl Drop for TempPcm {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Removes a file unless it was handed on.
struct Pending(Option<PathBuf>);

impl Drop for Pending {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// What a request carried.
#[derive(Debug)]
pub struct Upload {
    /// The audio.
    pub pcm: TempPcm,
    /// `model` and `language`.
    pub request: ServeRequest,
    /// `response_format`.
    pub format: ResponseFormat,
}

async fn next_chunk(field: &mut Field<'_>) -> Result<Option<axum::body::Bytes>, ApiError> {
    match tokio::time::timeout(IDLE, field.chunk()).await {
        Err(_) => Err(ApiError::timeout()),
        Ok(Err(e)) => Err(ApiError::invalid("invalid_multipart", format!("请求体无法解析：{e}"))),
        Ok(Ok(chunk)) => Ok(chunk),
    }
}

async fn text_field(field: &mut Field<'_>, name: &str) -> Result<String, ApiError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = next_chunk(field).await? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > MAX_FIELD_BYTES {
            return Err(ApiError::invalid("field_too_long", format!("字段 {name} 超过 {MAX_FIELD_BYTES} 字节")));
        }
    }
    String::from_utf8(bytes).map_err(|_| ApiError::invalid("invalid_field", format!("字段 {name} 不是 UTF-8 文本")))
}

fn decode_error(error: DecodeError) -> ApiError {
    match error {
        DecodeError::Unsupported(message) => ApiError::unsupported(message),
        DecodeError::TooLong(message) => ApiError::too_large(message),
        DecodeError::Broken(message) => ApiError::invalid("invalid_audio", message),
        DecodeError::Io(message) => ApiError::internal(message),
    }
}

/// Decode the `file` part into `uploads/<uuid>.pcm`, refusing more than `max_samples`.
async fn file_field(field: &mut Field<'_>, uploads: &Path, max_samples: u64) -> Result<TempPcm, ApiError> {
    let path = uploads.join(format!("{}.pcm", uuid::Uuid::new_v4()));
    let pending = Pending(Some(path.clone()));
    let decoder = wav::start(path.clone(), max_samples);
    let mut sent = 0u64;
    let mut tx = Some(decoder.tx);
    while let Some(chunk) = next_chunk(field).await? {
        sent += chunk.len() as u64;
        if let Some(sender) = &tx
            && sender.send(chunk).await.is_err()
        {
            // The decoder stopped: its result says why (or the WAV ended and this is what follows it).
            tx = None;
            if decoder.task.is_finished() {
                break;
            }
        }
        if tx.is_none() && sent.saturating_sub(decoder.consumed.load(Ordering::SeqCst)) > MAX_TRAILING_BYTES {
            break;
        }
    }
    drop(tx);
    let decoded = decoder.task.await.map_err(|e| ApiError::internal(format!("解码任务失败：{e}")))?.map_err(decode_error)?;
    // The file's bytes after the samples: a WAV carries little there.
    while let Some(chunk) = next_chunk(field).await? {
        sent += chunk.len() as u64;
        if sent.saturating_sub(decoder.consumed.load(Ordering::SeqCst)) > MAX_TRAILING_BYTES {
            return Err(ApiError::too_large("音频数据之后的附加内容过多"));
        }
    }
    if sent.saturating_sub(decoder.consumed.load(Ordering::SeqCst)) > MAX_TRAILING_BYTES {
        return Err(ApiError::too_large("音频数据之后的附加内容过多"));
    }
    tracing::debug!(channels = decoded.format.0, rate = decoded.format.1, bits = decoded.format.2, samples = decoded.samples, "upload decoded");
    let mut pending = pending;
    let path = pending.0.take().unwrap_or(path);
    Ok(TempPcm { path, samples: decoded.samples })
}

/// Read the whole request. `stream=true` is refused: the service answers once, with the text.
pub async fn read(mut multipart: Multipart, uploads: &Path, max_samples: u64) -> Result<Upload, ApiError> {
    let (mut pcm, mut request, mut format, mut parts) = (None, ServeRequest::default(), ResponseFormat::default(), 0usize);
    loop {
        let next = match tokio::time::timeout(IDLE, multipart.next_field()).await {
            Err(_) => return Err(ApiError::timeout()),
            Ok(Err(e)) => return Err(ApiError::invalid("invalid_multipart", format!("请求体无法解析：{e}"))),
            Ok(Ok(next)) => next,
        };
        let Some(mut field) = next else { break };
        parts += 1;
        if parts > MAX_PARTS {
            return Err(ApiError::invalid("too_many_fields", format!("请求最多 {MAX_PARTS} 个字段")));
        }
        let name = field.name().unwrap_or_default().to_owned();
        match name.as_str() {
            "file" => {
                if pcm.is_some() {
                    return Err(ApiError::invalid("duplicate_file", "请求只能带一个 file"));
                }
                pcm = Some(file_field(&mut field, uploads, max_samples).await?);
            }
            "model" => request.model = Some(text_field(&mut field, &name).await?),
            "language" => request.language = Some(text_field(&mut field, &name).await?).filter(|l| !l.trim().is_empty()),
            "response_format" => format = ResponseFormat::parse(&text_field(&mut field, &name).await?)?,
            "stream" => {
                if text_field(&mut field, &name).await?.trim().eq_ignore_ascii_case("true") {
                    return Err(ApiError::invalid("stream_unsupported", "不支持 stream=true：本服务在处理完成后一次返回文字"));
                }
            }
            // `prompt` (Paseo always sends a fixed English instruction; it is not a clean-up
            // instruction), `temperature`, `include[]`, `timestamp_granularities[]` …: read and ignored.
            _ => {
                text_field(&mut field, &name).await?;
            }
        }
    }
    let pcm = pcm.ok_or_else(|| ApiError::invalid("missing_file", "缺少 file 字段"))?;
    Ok(Upload { pcm, request, format })
}
