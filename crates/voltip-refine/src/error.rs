//! What can go wrong talking to the chat-completions service.

/// Refinement failures, sorted by what the caller can do about them. None of them should stop
/// a dictation: the caller falls back to the raw transcript.
///
/// The interface shows the opening of each message in its own language and the service's words
/// after it (`serviceReasonText` in packages/shared/src/labels.ts, docs/dictation.md §3.12): a
/// changed `#[error]` text changes that table too.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RefineError {
    /// The configuration cannot produce a request (bad URL, empty model, zero timeout).
    #[error("invalid refine configuration: {0}")]
    InvalidConfig(String),
    /// The service answered 401 or 403: the key is missing, wrong or expired.
    #[error("refine service rejected the credentials{}", said(.message))]
    Unauthorized {
        /// What the service said ([`voltip_reply::summary`]); empty when it said nothing.
        message: String,
    },
    /// The service answered 429.
    #[error("refine service rate limited{}{}", retry_suffix(.retry_after_ms), said(.message))]
    RateLimited {
        /// `Retry-After` in milliseconds when the service sent one as a delay in seconds.
        retry_after_ms: Option<u64>,
        /// What the service said ([`voltip_reply::summary`]); empty when it said nothing.
        message: String,
    },
    /// The service has more requests than it can serve right now (HTTP 503): Gemini answers
    /// `UNAVAILABLE` "This model is currently experiencing high demand" (Google AI Studio,
    /// 2026-10-24, for 40 minutes on end). Like a rate limit it passes, and it is not a used-up
    /// quota (docs/dictation.md §3.6).
    #[error("refine service overloaded (HTTP 503){}", said(.message))]
    Overloaded {
        /// What the service said ([`voltip_reply::summary`]); empty when it said nothing.
        message: String,
    },
    /// Any other non-2xx answer.
    #[error("refine server error {status}{}", said(.body))]
    Server {
        /// HTTP status code.
        status: u16,
        /// What the service said ([`voltip_reply::summary`]): its error's message and code, a
        /// page's title or the text, cut to [`crate::MAX_ERROR_BODY_CHARS`] characters.
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
    /// The service says the model's quota is used up ([`is_quota_exhausted`]): Model Studio's
    /// 免费额度用完即停 (`AllocationQuota.FreeTierOnly`) or OpenAI's `insufficient_quota`. The one
    /// error a fallback model list moves on for (docs/dictation.md §3.5).
    #[error("refine quota used up ({code}): {message}")]
    QuotaExhausted {
        /// The service's error code.
        code: String,
        /// Its message, cut to [`crate::MAX_ERROR_BODY_CHARS`] characters.
        message: String,
        /// When the quota is back (milliseconds since the epoch), when the service said: the
        /// built-in service's report ([`crate::QUOTA_HEADER`], docs/dictation.md §27).
        retry_at_ms: Option<u64>,
    },
}

/// Whether a service's error `code` or `message` says the model's quota is used up
/// (docs/dictation.md §3.5): Model Studio's 免费额度用完即停 names it `AllocationQuota.FreeTierOnly`,
/// OpenAI and the services copying its errors `insufficient_quota`. Rate limits are not: Model
/// Studio's `Throttling…` and OpenAI's `rate_limit_exceeded` stay [`RefineError::RateLimited`].
/// The same rule as `voltip_asr::is_quota_exhausted`.
pub fn is_quota_exhausted(code: &str, message: &str) -> bool {
    code.contains("FreeTierOnly") || message.contains("FreeTierOnly") || code == "insufficient_quota"
}

/// The `code` and `message` of an error body in every shape the providers send
/// ([`voltip_reply::error_fields`]: OpenAI's, Model Studio's, Google AI Studio's array, …); empty
/// strings when the body has neither.
pub(crate) fn error_fields(body: &str) -> (String, String) {
    let fields = voltip_reply::error_fields(body);
    (fields.code, fields.message)
}

impl RefineError {
    /// Whether the same request may succeed if sent again: rate limits, an overloaded service, 5xx,
    /// network and timeout. Configuration, credential, malformed-response and empty-answer errors
    /// are not.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::RateLimited { .. } | Self::Overloaded { .. } | Self::Network(_) | Self::Timeout => true,
            Self::Server { status, .. } => *status >= 500,
            Self::InvalidConfig(_) | Self::Unauthorized { .. } | Self::BadResponse(_) | Self::EmptyAnswer | Self::Truncated | Self::QuotaExhausted { .. } => {
                false
            }
        }
    }
}

fn retry_suffix(retry_after_ms: &Option<u64>) -> String {
    retry_after_ms.map_or_else(String::new, |ms| format!(" (retry after {ms} ms)"))
}

/// `: <what the service said>`, or nothing when it said nothing.
fn said(message: &str) -> String {
    if message.is_empty() { String::new() } else { format!(": {message}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_are_plain() {
        assert_eq!(RefineError::InvalidConfig("x".into()).to_string(), "invalid refine configuration: x");
        assert_eq!(RefineError::Unauthorized { message: String::new() }.to_string(), "refine service rejected the credentials");
        assert_eq!(RefineError::Unauthorized { message: "bad key".into() }.to_string(), "refine service rejected the credentials: bad key");
        assert_eq!(RefineError::RateLimited { retry_after_ms: None, message: String::new() }.to_string(), "refine service rate limited");
        assert_eq!(
            RefineError::RateLimited { retry_after_ms: Some(1500), message: "slow down".into() }.to_string(),
            "refine service rate limited (retry after 1500 ms): slow down"
        );
        assert_eq!(RefineError::Server { status: 502, body: "down".into() }.to_string(), "refine server error 502: down");
        assert_eq!(RefineError::Overloaded { message: "busy".into() }.to_string(), "refine service overloaded (HTTP 503): busy");
        assert_eq!(RefineError::Network("reset".into()).to_string(), "refine network error: reset");
        assert_eq!(RefineError::Timeout.to_string(), "refine request timed out");
        assert_eq!(RefineError::BadResponse("no choices".into()).to_string(), "refine service returned an unexpected response: no choices");
        assert_eq!(RefineError::EmptyAnswer.to_string(), "refine service returned an empty answer");
        assert_eq!(RefineError::Truncated.to_string(), "refine answer was cut off at the output limit");
        assert_eq!(
            RefineError::QuotaExhausted { code: "insufficient_quota".into(), message: "You exceeded your current quota".into(), retry_at_ms: None }.to_string(),
            "refine quota used up (insufficient_quota): You exceeded your current quota"
        );
    }

    #[test]
    fn retryable_classification() {
        assert!(RefineError::RateLimited { retry_after_ms: Some(1), message: String::new() }.is_retryable());
        assert!(RefineError::Network(String::new()).is_retryable());
        assert!(RefineError::Timeout.is_retryable());
        assert!(RefineError::Server { status: 502, body: String::new() }.is_retryable());
        assert!(RefineError::Overloaded { message: String::new() }.is_retryable());
        assert!(!RefineError::Server { status: 422, body: String::new() }.is_retryable());
        assert!(!RefineError::Unauthorized { message: String::new() }.is_retryable());
        assert!(!RefineError::InvalidConfig(String::new()).is_retryable());
        assert!(!RefineError::BadResponse(String::new()).is_retryable());
        assert!(!RefineError::EmptyAnswer.is_retryable());
        assert!(!RefineError::Truncated.is_retryable(), "the same request would be cut off again");
        assert!(!RefineError::QuotaExhausted { code: String::new(), message: String::new(), retry_at_ms: None }.is_retryable());
    }

    #[test]
    fn a_used_up_quota_is_told_from_a_rate_limit() {
        assert!(is_quota_exhausted("AllocationQuota.FreeTierOnly", ""));
        assert!(is_quota_exhausted("", "AllocationQuota.FreeTierOnly"));
        assert!(is_quota_exhausted("insufficient_quota", ""));
        assert!(!is_quota_exhausted("Throttling.AllocationQuota", "Allocated quota exceeded"));
        assert!(!is_quota_exhausted("rate_limit_exceeded", ""));
        assert_eq!(error_fields(r#"{"error":{"code":"insufficient_quota","message":"m"}}"#), ("insufficient_quota".into(), "m".into()));
        assert_eq!(error_fields(r#"{"code":"AllocationQuota.FreeTierOnly","message":"m"}"#), ("AllocationQuota.FreeTierOnly".into(), "m".into()));
        // Google AI Studio wraps its error in an array, with a numeric code and a status (2026-10-24).
        assert_eq!(error_fields(r#"[{"error":{"code":503,"message":"high demand","status":"UNAVAILABLE"}}]"#), ("UNAVAILABLE".into(), "high demand".into()));
        assert_eq!(error_fields("[]"), (String::new(), String::new()));
        assert_eq!(error_fields(r#"{"code":7}"#), ("7".into(), String::new()));
        assert_eq!(error_fields("<html>"), (String::new(), String::new()));
    }
}
