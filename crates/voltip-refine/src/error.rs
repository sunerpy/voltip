//! What can go wrong talking to the chat-completions service.

/// Refinement failures, sorted by what the caller can do about them. None of them should stop
/// a dictation: the caller falls back to the raw transcript.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RefineError {
    /// The configuration cannot produce a request (bad URL, empty model, zero timeout).
    #[error("invalid refine configuration: {0}")]
    InvalidConfig(String),
    /// The service answered 401 or 403: the key is missing, wrong or expired.
    #[error("refine service rejected the credentials")]
    Unauthorized,
    /// The service answered 429.
    #[error("refine service rate limited{}", retry_suffix(.retry_after_ms))]
    RateLimited {
        /// `Retry-After` in milliseconds when the service sent one as a delay in seconds.
        retry_after_ms: Option<u64>,
    },
    /// Any other non-2xx answer.
    #[error("refine server error {status}: {body}")]
    Server {
        /// HTTP status code.
        status: u16,
        /// Response body, cut to [`crate::MAX_ERROR_BODY_CHARS`] characters.
        body: String,
    },
    /// DNS, TCP, TLS or a connection dropped mid-request.
    #[error("refine network error: {0}")]
    Network(String),
    /// The request did not complete within the configured timeout.
    #[error("refine request timed out")]
    Timeout,
    /// A 2xx answer whose body is not the expected JSON.
    #[error("refine service returned an unexpected response: {0}")]
    BadResponse(String),
    /// The model answered with nothing usable (or there was nothing to refine).
    #[error("refine service returned an empty answer")]
    EmptyAnswer,
    /// The answer stopped at the output limit (`finish_reason: length`): an edit refuses it rather
    /// than paste a cut-off rewrite over the whole selection (docs/dictation.md §19).
    #[error("refine answer was cut off at the output limit")]
    Truncated,
}

impl RefineError {
    /// Whether the same request may succeed if sent again: rate limits, 5xx, network and
    /// timeout. Configuration, credential, malformed-response and empty-answer errors are not.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::RateLimited { .. } | Self::Network(_) | Self::Timeout => true,
            Self::Server { status, .. } => *status >= 500,
            Self::InvalidConfig(_) | Self::Unauthorized | Self::BadResponse(_) | Self::EmptyAnswer | Self::Truncated => false,
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
        assert_eq!(RefineError::InvalidConfig("x".into()).to_string(), "invalid refine configuration: x");
        assert_eq!(RefineError::Unauthorized.to_string(), "refine service rejected the credentials");
        assert_eq!(RefineError::RateLimited { retry_after_ms: None }.to_string(), "refine service rate limited");
        assert_eq!(RefineError::RateLimited { retry_after_ms: Some(1500) }.to_string(), "refine service rate limited (retry after 1500 ms)");
        assert_eq!(RefineError::Server { status: 503, body: "down".into() }.to_string(), "refine server error 503: down");
        assert_eq!(RefineError::Network("reset".into()).to_string(), "refine network error: reset");
        assert_eq!(RefineError::Timeout.to_string(), "refine request timed out");
        assert_eq!(RefineError::BadResponse("no choices".into()).to_string(), "refine service returned an unexpected response: no choices");
        assert_eq!(RefineError::EmptyAnswer.to_string(), "refine service returned an empty answer");
        assert_eq!(RefineError::Truncated.to_string(), "refine answer was cut off at the output limit");
    }

    #[test]
    fn retryable_classification() {
        assert!(RefineError::RateLimited { retry_after_ms: Some(1) }.is_retryable());
        assert!(RefineError::Network(String::new()).is_retryable());
        assert!(RefineError::Timeout.is_retryable());
        assert!(RefineError::Server { status: 502, body: String::new() }.is_retryable());
        assert!(!RefineError::Server { status: 422, body: String::new() }.is_retryable());
        assert!(!RefineError::Unauthorized.is_retryable());
        assert!(!RefineError::InvalidConfig(String::new()).is_retryable());
        assert!(!RefineError::BadResponse(String::new()).is_retryable());
        assert!(!RefineError::EmptyAnswer.is_retryable());
        assert!(!RefineError::Truncated.is_retryable(), "the same request would be cut off again");
    }
}
