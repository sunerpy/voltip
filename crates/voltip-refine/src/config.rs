//! Client configuration and base-URL normalisation.

use std::fmt;
use std::time::Duration;

use url::Url;

use crate::RefineError;

/// Default [`RefineConfig::timeout`]. Refinement is short text in, short text out.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// Longest error body kept in [`RefineError::Server`].
pub const MAX_ERROR_BODY_CHARS: usize = 200;

/// How aggressively the model may rewrite.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RefineStyle {
    /// Punctuation, obvious mis-recognitions and spoken fillers. The everyday setting.
    #[default]
    Default,
    /// Punctuation and sentence breaks only; every word stays as dictated.
    Punctuation,
    /// Default plus a light shift towards written register (no slang, complete sentences).
    Formal,
}

/// How to reach the chat-completions service.
#[derive(Clone, PartialEq, Eq)]
pub struct RefineConfig {
    /// Service root; `https://api.groq.com/openai/v1`, `https://api.openai.com/v1` and
    /// `https://host` (which becomes `https://host/v1`) are all fine. See [`normalize_base_url`].
    pub base_url: String,
    /// Bearer key sent as `Authorization`, if the service wants one. Never printed by `Debug`.
    pub api_key: Option<String>,
    /// Model name sent in the `model` field.
    pub model: String,
    /// Whole-request timeout.
    pub timeout: Duration,
    /// Rewrite style.
    pub style: RefineStyle,
}

impl RefineConfig {
    /// A configuration without a key, default timeout and [`RefineStyle::Default`].
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self { base_url: base_url.into(), api_key: None, model: model.into(), timeout: DEFAULT_TIMEOUT, style: RefineStyle::Default }
    }

    /// Set the bearer key.
    pub fn with_api_key(mut self, api_key: Option<String>) -> Self {
        self.api_key = api_key;
        self
    }

    /// Set the whole-request timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set the rewrite style.
    pub fn with_style(mut self, style: RefineStyle) -> Self {
        self.style = style;
        self
    }
}

impl fmt::Debug for RefineConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RefineConfig")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("model", &self.model)
            .field("timeout", &self.timeout)
            .field("style", &self.style)
            .finish()
    }
}

/// Normalise a service root: trim whitespace, require `http`/`https`, drop query and fragment,
/// strip trailing slashes and make sure the path ends in `/v1`. Returns the URL without a
/// trailing slash so endpoints can be appended with `format!("{base}/chat/completions")`.
pub fn normalize_base_url(raw: &str) -> Result<String, RefineError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(RefineError::InvalidConfig("base_url is empty".into()));
    }
    let mut url = Url::parse(raw).map_err(|e| RefineError::InvalidConfig(format!("base_url {raw:?}: {e}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(RefineError::InvalidConfig(format!("base_url {raw:?}: scheme must be http or https")));
    }
    if url.host_str().is_none() {
        return Err(RefineError::InvalidConfig(format!("base_url {raw:?}: missing host")));
    }
    let mut path = url.path().trim_end_matches('/').to_string();
    if !path.ends_with("/v1") {
        path.push_str("/v1");
    }
    url.set_path(&path);
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalisation_table() {
        let cases = [
            ("https://api.groq.com/openai/v1", "https://api.groq.com/openai/v1"),
            ("https://api.groq.com/openai/v1/", "https://api.groq.com/openai/v1"),
            ("https://api.groq.com/openai", "https://api.groq.com/openai/v1"),
            ("https://api.openai.com", "https://api.openai.com/v1"),
            ("https://host/", "https://host/v1"),
            ("http://localhost:11434", "http://localhost:11434/v1"),
            ("  https://host/v1?k=v#x ", "https://host/v1"),
        ];
        for (input, want) in cases {
            assert_eq!(normalize_base_url(input).as_deref(), Ok(want), "input {input:?}");
        }
        for bad in ["", "host", "ws://host", "https://"] {
            assert!(matches!(normalize_base_url(bad), Err(RefineError::InvalidConfig(_))), "{bad:?}");
        }
    }

    #[test]
    fn config_builder_and_redacted_debug() {
        let config = RefineConfig::new("https://host", "llama-3.3-70b-versatile");
        assert_eq!((config.api_key.as_deref(), config.timeout, config.style), (None, DEFAULT_TIMEOUT, RefineStyle::Default));
        let config = config.with_api_key(Some("gsk_secret_key".into())).with_timeout(Duration::from_secs(3)).with_style(RefineStyle::Formal);
        assert_eq!(config.style, RefineStyle::Formal);
        assert_eq!(config.timeout, Duration::from_secs(3));
        let debug = format!("{config:?}");
        assert!(!debug.contains("gsk_secret_key"), "{debug}");
        assert!(debug.contains("api_key: Some(\"<redacted>\")"), "{debug}");
        assert!(debug.contains("style: Formal"), "{debug}");
        assert!(format!("{:?}", RefineConfig::new("https://host", "m")).contains("api_key: None"));
        assert_eq!(RefineStyle::default(), RefineStyle::Default);
        assert_eq!(config.clone(), config);
    }
}
