//! What can go wrong talking to the ASR service.

/// ASR failures, sorted by what the caller can do about them.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AsrError {
    /// The configuration cannot produce a request (bad URL, empty model, zero timeout).
    #[error("invalid ASR configuration: {0}")]
    InvalidConfig(String),
    /// The service answered 401 or 403: the token is missing, wrong or expired.
    #[error("ASR rejected the credentials")]
    Unauthorized,
    /// The service answered 429.
    #[error("ASR rate limited{}", retry_suffix(.retry_after_ms))]
    RateLimited {
        /// `Retry-After` in milliseconds when the service sent one as a delay in seconds.
        retry_after_ms: Option<u64>,
    },
    /// Any other non-2xx answer.
    #[error("ASR server error {status}: {body}")]
    Server {
        /// HTTP status code.
        status: u16,
        /// Response body, cut to [`crate::MAX_ERROR_BODY_CHARS`] characters.
        body: String,
    },
    /// DNS, TCP, TLS or a connection dropped mid-request.
    #[error("ASR network error: {0}")]
    Network(String),
    /// The request did not complete within the configured timeout.
    #[error("ASR request timed out")]
    Timeout,
    /// A 2xx answer whose body is not the expected JSON.
    #[error("ASR returned an unexpected response: {0}")]
    BadResponse(String),
    /// The service refused the request with its own error code (Model Studio's `InvalidParameter`,
    /// `Model.AccessDenied`, a realtime task's `task-failed`).
    #[error("ASR service error {code}: {message}")]
    Service {
        /// The service's error code.
        code: String,
        /// Its message, cut to [`crate::MAX_ERROR_BODY_CHARS`] characters.
        message: String,
    },
    /// Model Studio's 免费额度用完即停 stopped the model: its free quota is used up and the account
    /// allows it nothing more (`AllocationQuota.FreeTierOnly`).
    #[error("ASR free quota used up: Model Studio stops this model once its free quota ends (免费额度用完即停)")]
    FreeQuotaExhausted,
    /// The recording is not audio the protocol can send (not a 16-bit PCM WAV).
    #[error("ASR audio: {0}")]
    Audio(String),
}

impl AsrError {
    /// Whether the same request may succeed if sent again: rate limits, 5xx, network and
    /// timeout. Configuration, credential and malformed-response errors are not retryable.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::RateLimited { .. } | Self::Network(_) | Self::Timeout => true,
            Self::Server { status, .. } => *status >= 500,
            Self::InvalidConfig(_) | Self::Unauthorized | Self::BadResponse(_) | Self::Service { .. } | Self::FreeQuotaExhausted | Self::Audio(_) => false,
        }
    }
}

fn retry_suffix(retry_after_ms: &Option<u64>) -> String {
    retry_after_ms.map_or_else(String::new, |ms| format!(" (retry after {ms} ms)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_are_plain() {
        assert_eq!(AsrError::InvalidConfig("base_url is empty".into()).to_string(), "invalid ASR configuration: base_url is empty");
        assert_eq!(AsrError::Unauthorized.to_string(), "ASR rejected the credentials");
        assert_eq!(AsrError::RateLimited { retry_after_ms: None }.to_string(), "ASR rate limited");
        assert_eq!(AsrError::RateLimited { retry_after_ms: Some(3000) }.to_string(), "ASR rate limited (retry after 3000 ms)");
        assert_eq!(AsrError::Server { status: 502, body: "bad gateway".into() }.to_string(), "ASR server error 502: bad gateway");
        assert_eq!(AsrError::Network("connection refused".into()).to_string(), "ASR network error: connection refused");
        assert_eq!(AsrError::Timeout.to_string(), "ASR request timed out");
        assert_eq!(AsrError::BadResponse("not json".into()).to_string(), "ASR returned an unexpected response: not json");
        assert_eq!(
            AsrError::Service { code: "InvalidParameter".into(), message: "Model not exist.".into() }.to_string(),
            "ASR service error InvalidParameter: Model not exist."
        );
        assert!(AsrError::FreeQuotaExhausted.to_string().contains("免费额度用完即停"));
        assert_eq!(AsrError::Audio("not a WAV file".into()).to_string(), "ASR audio: not a WAV file");
    }

    #[test]
    fn retryable_classification() {
        assert!(AsrError::RateLimited { retry_after_ms: None }.is_retryable());
        assert!(AsrError::Network("x".into()).is_retryable());
        assert!(AsrError::Timeout.is_retryable());
        assert!(AsrError::Server { status: 500, body: String::new() }.is_retryable());
        assert!(AsrError::Server { status: 503, body: String::new() }.is_retryable());
        assert!(!AsrError::Server { status: 400, body: String::new() }.is_retryable());
        assert!(!AsrError::Server { status: 404, body: String::new() }.is_retryable());
        assert!(!AsrError::Unauthorized.is_retryable());
        assert!(!AsrError::InvalidConfig(String::new()).is_retryable());
        assert!(!AsrError::BadResponse(String::new()).is_retryable());
        assert!(!AsrError::Service { code: String::new(), message: String::new() }.is_retryable());
        assert!(!AsrError::FreeQuotaExhausted.is_retryable());
        assert!(!AsrError::Audio(String::new()).is_retryable());
    }
}
