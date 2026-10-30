//! Importing audio and video files (docs/dictation.md §31). A file is decoded to the app's
//! recording format by the shell's [`MediaDecoder`] (the desktop's `voltip-media`), cut into
//! segments of [`SEGMENT_BOUNDS`], recognised one segment after the other and, when asked,
//! polished segment by segment ([`run`]); it lands in the history as one entry whose segments keep
//! their times, so its subtitles line up with the file. The original file is only ever read.
//!
//! The jobs wait in a queue in memory and run one at a time (`crate::runtime`'s import side); a
//! dictation always goes first: a job waits at the next segment while one runs. Imports never use
//! the built-in service: only an installed local model, a cloud provider with the user's own key,
//! or Ollama and the custom endpoint as the user set them up, key or not (user decision D20).

pub mod run;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::audio_cache::RecognitionChoice;
use crate::dictation::SegmentBounds;
use crate::dictation::long::RATE;
use crate::models::CancelToken;
use crate::providers::ProviderId;

/// An import's segments run 3–15 s (docs/dictation.md §31): shorter than a long take's 20–45 s,
/// so each subtitle stays close to the speech it shows.
pub const SEGMENT_BOUNDS: SegmentBounds = SegmentBounds { min: 3 * RATE, max: 15 * RATE };
/// Where the decoded files wait, under the data directory; emptied at every start.
pub const IMPORTS_DIR: &str = "imports";

/// What a media file holds, as [`MediaDecoder::probe`] reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MediaInfo {
    /// The container (`wav`, `mp3`, `mp4`, `mov`, `mkv`, `webm`, `ogg`, `flac`, …).
    pub container: String,
    /// The codec of the audio track that is read (`pcm`, `mp3`, `aac`, `opus`, …).
    pub codec: String,
    /// That track's channels.
    pub channels: u16,
    /// That track's sample rate.
    pub sample_rate: u32,
    /// Its length, as the file says or the reader estimates it (an MP3 without an encoder
    /// header, a raw AAC stream).
    pub duration_ms: Option<u64>,
    /// How many audio tracks the file has (only one is read).
    pub audio_tracks: u32,
}

/// What [`MediaDecoder::decode_to_pcm`] wrote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PcmInfo {
    /// 16 kHz mono samples in the output file.
    pub samples: u64,
    /// The file they came from.
    pub info: MediaInfo,
}

/// Why a file could not be decoded.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MediaError {
    /// The file could not be opened or read (it is gone, or not readable by this user).
    #[error("the file cannot be read: {0}")]
    Unreadable(String),
    /// Not an audio or video file the decoder knows.
    #[error("not an audio or video file this app reads")]
    UnsupportedFormat,
    /// The file has no audio track.
    #[error("the file has no audio track")]
    NoAudio,
    /// The audio track's codec cannot be decoded; its name.
    #[error("unsupported audio codec: {0}")]
    UnsupportedCodec(String),
    /// The file is damaged or cut off.
    #[error("the file is damaged: {0}")]
    Corrupt(String),
    /// [`MediaDecoder::decode_to_pcm`] was cancelled.
    #[error("cancelled")]
    Cancelled,
    /// The decoded audio could not be written.
    #[error("the decoded audio cannot be written: {0}")]
    Write(String),
}

/// Decodes audio and video files (docs/dictation.md §31): the desktop plugs `voltip-media` in.
/// Both calls block; the import runs them on a blocking thread.
pub trait MediaDecoder: Send + Sync + std::fmt::Debug {
    /// The file extensions of what it reads, lowercase and without the dot: the history page's
    /// open dialog and its drop take files with these.
    fn extensions(&self) -> &'static [&'static str];

    /// What `path` holds, without decoding it.
    fn probe(&self, path: &Path) -> Result<MediaInfo, MediaError>;

    /// Decode the audio of `path` into `out` as 16 kHz mono 16-bit little-endian samples (the
    /// recording format, `crate::dictation::long`), written next to it as `<out>.part` and renamed
    /// at the end: `out` exists only complete. Video tracks are skipped; of several audio tracks
    /// the default one is read. `progress(done_ms, total_ms)` follows the file's time; `cancel`
    /// stops it soon after, with [`MediaError::Cancelled`] and nothing left behind.
    fn decode_to_pcm(&self, path: &Path, out: &Path, progress: &dyn Fn(u64, Option<u64>), cancel: &CancelToken) -> Result<PcmInfo, MediaError>;
}

/// The polish of an import: a provider's model (a cloud provider's with the user's own key, or
/// Ollama's or the custom endpoint's as the user set it up).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolishModel {
    /// Whose model.
    pub provider: ProviderId,
    /// The model.
    pub model: String,
}

/// One file to import and what with (docs/dictation.md §31). Only that import uses the models:
/// the settings stay as they are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportRequest {
    /// The file.
    pub path: PathBuf,
    /// The recognition: an installed local model, or a provider's model (a cloud provider's with
    /// the user's key, or the custom endpoint's).
    pub asr: RecognitionChoice,
    /// The polish, when the text is polished; `None`: it is not.
    pub refine: Option<PolishModel>,
    /// The language hint; `None`: detected.
    pub language: Option<String>,
}

/// Why an import is refused before it is queued.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ImportError {
    /// The built-in service was chosen, or is what the choice resolves to.
    #[error("导入只能使用本机模型或你自己设置的服务，不能使用内置服务")]
    BuiltinNotAllowed,
    /// A cloud provider that needs a key, without the user's own (Ollama and the custom endpoint
    /// import without one, user decision D20).
    #[error("这个云端服务需要填写你自己的密钥，才能用于导入")]
    NoOwnKey,
    /// This app imports nothing (the phone).
    #[error("这台设备不能导入音视频文件")]
    Unavailable,
    /// The history is off: an import's text has nowhere to go.
    #[error("历史记录已关闭：导入的文字保存在历史记录里，请先打开历史记录")]
    HistoryOff,
    /// Not a file that can be read.
    #[error("无法读取文件：{0}")]
    NotAFile(String),
    /// The chosen models cannot run (the core's sentence).
    #[error("{0}")]
    Engines(String),
}

/// Where an import job is (docs/dictation.md §31).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ImportState {
    /// Waiting for the jobs before it.
    Queued,
    /// The file is being decoded; `pct` once its length is known.
    Decoding {
        /// How far, 0–100.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pct: Option<u8>,
    },
    /// Its segments are being recognised.
    Recognizing {
        /// Segments done.
        done: u32,
        /// Segments in all.
        total: u32,
    },
    /// Its segments are being polished.
    Polishing {
        /// Segments done.
        done: u32,
        /// Segments with text.
        total: u32,
    },
    /// In the history.
    Done {
        /// The entry.
        entry: Uuid,
        /// Segments whose polish failed: they kept their recognised text.
        #[serde(default)]
        polish_failed: u32,
    },
    /// It stopped.
    Failed {
        /// Why.
        failure: ImportFailure,
        /// 重试 goes on from the first segment not recognised; otherwise it starts again.
        resumable: bool,
    },
}

impl ImportState {
    /// Decoding, recognising or polishing.
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Decoding { .. } | Self::Recognizing { .. } | Self::Polishing { .. })
    }
}

/// Why an import job stopped (docs/dictation.md §31), by kind: the interface words it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum ImportFailure {
    /// The file could not be read (gone, or not readable).
    Unreadable {
        /// The system's words.
        detail: String,
    },
    /// Not an audio or video file the import reads.
    UnsupportedFormat,
    /// The file has no audio track.
    NoAudio,
    /// Its audio's codec is not supported.
    UnsupportedCodec {
        /// The codec's name (`AC-3`, `DTS`, …).
        codec: String,
    },
    /// The file is damaged or cut off.
    Corrupt {
        /// What is wrong.
        detail: String,
    },
    /// The decoded audio could not be kept in the data directory, or the entry not saved.
    Storage {
        /// The system's words.
        detail: String,
    },
    /// A segment could not be recognised in two tries.
    Recognition {
        /// The recogniser's words (the interface translates the clients' phrases).
        detail: String,
    },
    /// Nothing was said in the whole file.
    NoSpeech,
}

impl From<MediaError> for ImportFailure {
    fn from(e: MediaError) -> Self {
        match e {
            MediaError::Unreadable(detail) => Self::Unreadable { detail },
            MediaError::UnsupportedFormat => Self::UnsupportedFormat,
            MediaError::NoAudio => Self::NoAudio,
            MediaError::UnsupportedCodec(codec) => Self::UnsupportedCodec { codec },
            MediaError::Corrupt(detail) => Self::Corrupt { detail },
            MediaError::Write(detail) => Self::Storage { detail },
            // A cancel removes its job: this failure is never shown.
            MediaError::Cancelled => Self::Storage { detail: "cancelled".into() },
        }
    }
}

/// One file in the queue, as the interface shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportJob {
    /// Its id for 取消, 重试, 移除.
    pub id: Uuid,
    /// The file's name (never its path).
    pub name: String,
    /// Its size in bytes.
    pub size: u64,
    /// Its length, once known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Waiting for a dictation to end before its next segment.
    #[serde(default)]
    pub paused: bool,
    /// Where it is.
    pub state: ImportState,
}

/// `UiState.imports`: the queue (docs/dictation.md §31), in the order the files run.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportsView {
    /// The app imports files (the desktop); the phone shows none of it.
    pub available: bool,
    /// The file extensions the import reads ([`MediaDecoder::extensions`]): what a drop takes.
    pub extensions: Vec<String>,
    /// The jobs: running, waiting, done and failed, until removed or the app quits.
    pub jobs: Vec<ImportJob>,
}

/// The name an import shows and its entry keeps: the file name alone.
pub fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

/// `<data_dir>/imports`.
pub fn imports_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(IMPORTS_DIR)
}
