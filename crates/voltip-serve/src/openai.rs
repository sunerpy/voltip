//! The OpenAI shapes the service answers in (docs/dictation.md §23.4): the transcription
//! responses (`json`, `text`, `verbose_json`), the model list and the error body
//! `{"error": {"message", "type", "code"}}`.

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use voltip_core::serve::{ModelInfo, RATE, ServeError, ServeOutcome};

/// An error as the client receives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiError {
    /// The HTTP status.
    pub status: StatusCode,
    /// OpenAI's `type`.
    pub kind: &'static str,
    /// A stable machine-readable `code`.
    pub code: &'static str,
    /// What went wrong, for a person.
    pub message: String,
    /// Seconds a client should wait before trying again (`Retry-After`).
    pub retry_after: Option<u32>,
}

impl ApiError {
    fn new(status: StatusCode, kind: &'static str, code: &'static str, message: impl Into<String>) -> Self {
        Self { status, kind, code, message: message.into(), retry_after: None }
    }

    /// 400: the request is not valid.
    pub fn invalid(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_request_error", code, message)
    }

    /// 401: no token, or not this one.
    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "invalid_request_error", "invalid_api_key", "缺少令牌或令牌不正确")
    }

    /// 408: the upload stopped arriving.
    pub fn timeout() -> Self {
        Self::new(StatusCode::REQUEST_TIMEOUT, "invalid_request_error", "upload_timeout", "上传超过 60 秒没有新数据")
    }

    /// 413: too long or too large.
    pub fn too_large(message: impl Into<String>) -> Self {
        Self::new(StatusCode::PAYLOAD_TOO_LARGE, "invalid_request_error", "audio_too_long", message)
    }

    /// 415: not a format the service reads.
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNSUPPORTED_MEDIA_TYPE, "invalid_request_error", "unsupported_audio_format", message)
    }

    /// 503: recognition cannot run now, or every slot and place in the queue is taken.
    pub fn unavailable(code: &'static str, message: impl Into<String>) -> Self {
        Self { retry_after: Some(5), ..Self::new(StatusCode::SERVICE_UNAVAILABLE, "server_error", code, message) }
    }

    /// 500: something on this computer failed.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "server_error", "internal_error", message)
    }
}

impl From<ServeError> for ApiError {
    fn from(error: ServeError) -> Self {
        match error {
            ServeError::Invalid(message) => Self::invalid("invalid_model", message),
            ServeError::NotReady(message) => Self::unavailable("not_ready", message),
            ServeError::Quota(message) => Self::new(StatusCode::TOO_MANY_REQUESTS, "insufficient_quota", "insufficient_quota", message),
            ServeError::Upstream(message) => Self::new(StatusCode::BAD_GATEWAY, "server_error", "upstream_error", message),
            ServeError::Internal(message) => Self::internal(message),
            ServeError::Cancelled => Self::new(StatusCode::REQUEST_TIMEOUT, "invalid_request_error", "cancelled", "请求已取消"),
        }
    }
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    error: ErrorDetail<'a>,
}

#[derive(Serialize)]
struct ErrorDetail<'a> {
    message: &'a str,
    #[serde(rename = "type")]
    kind: &'a str,
    code: &'a str,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ErrorBody { error: ErrorDetail { message: &self.message, kind: self.kind, code: self.code } };
        let mut response = (self.status, Json(body)).into_response();
        if self.status == StatusCode::UNAUTHORIZED {
            response.headers_mut().insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        if let Some(seconds) = self.retry_after {
            response.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from(seconds));
        }
        response
    }
}

/// `response_format`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResponseFormat {
    /// `{"text"}` (the default).
    #[default]
    Json,
    /// The text alone.
    Text,
    /// The text with the take's details.
    VerboseJson,
}

impl ResponseFormat {
    /// Parse a request's `response_format`; `srt` and `vtt` need timestamps the service does not have.
    pub fn parse(value: &str) -> Result<Self, ApiError> {
        match value.trim() {
            "" | "json" => Ok(Self::Json),
            "text" => Ok(Self::Text),
            "verbose_json" => Ok(Self::VerboseJson),
            "srt" | "vtt" => Err(ApiError::invalid("unsupported_response_format", format!("不支持 response_format={value}：本服务不提供时间戳"))),
            other => Err(ApiError::invalid("unsupported_response_format", format!("不支持 response_format={other}；可用 json、text、verbose_json"))),
        }
    }
}

#[derive(Serialize)]
struct JsonBody<'a> {
    text: &'a str,
}

#[derive(Serialize)]
struct VerboseBody<'a> {
    task: &'static str,
    language: Option<&'a str>,
    duration: f64,
    text: &'a str,
    segments: [(); 0],
    voltip: &'a ServeOutcome,
}

/// The response for `outcome` in `format`.
pub fn transcription(outcome: &ServeOutcome, format: ResponseFormat) -> Response {
    match format {
        ResponseFormat::Json => Json(JsonBody { text: &outcome.text }).into_response(),
        ResponseFormat::Text => ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], outcome.text.clone()).into_response(),
        ResponseFormat::VerboseJson => Json(VerboseBody {
            task: "transcribe",
            language: outcome.language.as_deref(),
            duration: outcome.duration_ms as f64 / 1000.0,
            text: &outcome.text,
            segments: [],
            voltip: outcome,
        })
        .into_response(),
    }
}

#[derive(Serialize)]
struct ModelEntry<'a> {
    id: &'a str,
    object: &'static str,
    created: u64,
    owned_by: &'static str,
    name: &'a str,
}

#[derive(Serialize)]
struct ModelsBody<'a> {
    object: &'static str,
    data: Vec<ModelEntry<'a>>,
}

/// `GET /v1/models`.
pub fn models(models: &[ModelInfo]) -> Response {
    let data = models.iter().map(|m| ModelEntry { id: &m.id, object: "model", created: 0, owned_by: "voltip", name: &m.name }).collect();
    Json(ModelsBody { object: "list", data }).into_response()
}

/// Samples of the 16 kHz audio in `minutes`.
pub fn samples_for(minutes: u16) -> u64 {
    u64::from(minutes) * 60 * RATE
}
