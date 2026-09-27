//! Client configuration and base-URL normalisation.

use std::fmt;
use std::time::Duration;

use url::Url;

use crate::AsrError;

/// Default [`AsrConfig::timeout`]: a 120 s recording through a busy GPU still fits.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
/// Longest error body kept in [`AsrError::Server`].
pub const MAX_ERROR_BODY_CHARS: usize = 200;

/// How to reach the transcription service.
#[derive(Clone, PartialEq, Eq)]
pub struct AsrConfig {
    /// Service root, with or without `/v1` and a trailing slash: `https://host`, `https://host/`,
    /// `https://host/v1/` all become `https://host/v1`. See [`normalize_base_url`].
    pub base_url: String,
    /// Bearer token sent as `Authorization`, if the service wants one. Never printed by `Debug`.
    pub token: Option<String>,
    /// Model name sent in the `model` field (e.g. `Qwen/Qwen3-ASR-1.7B`, `whisper-1`).
    pub model: String,
    /// Whole-request timeout (connect, upload, inference, download).
    pub timeout: Duration,
}

impl AsrConfig {
    /// A configuration without a token, with the default timeout.
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self { base_url: base_url.into(), token: None, model: model.into(), timeout: DEFAULT_TIMEOUT }
    }

    /// Set the bearer token.
    pub fn with_token(mut self, token: Option<String>) -> Self {
        self.token = token;
        self
    }

    /// Set the whole-request timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

impl fmt::Debug for AsrConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AsrConfig")
            .field("base_url", &self.base_url)
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .field("model", &self.model)
            .field("timeout", &self.timeout)
            .finish()
    }
}

/// Normalise a service root: trim whitespace, require `http`/`https`, drop query and fragment,
/// strip trailing slashes and make sure the path ends in `/v1`. Returns the URL without a
/// trailing slash so endpoints can be appended with `format!("{base}/audio/transcriptions")`.
pub fn normalize_base_url(raw: &str) -> Result<String, AsrError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(AsrError::InvalidConfig("base_url is empty".into()));
    }
    let mut url = Url::parse(raw).map_err(|e| AsrError::InvalidConfig(format!("base_url {raw:?}: {e}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(AsrError::InvalidConfig(format!("base_url {raw:?}: scheme must be http or https")));
    }
    if url.host_str().is_none() {
        return Err(AsrError::InvalidConfig(format!("base_url {raw:?}: missing host")));
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
            ("https://host", "https://host/v1"),
            ("https://host/", "https://host/v1"),
            ("https://host//", "https://host/v1"),
            ("https://host/v1", "https://host/v1"),
            ("https://host/v1/", "https://host/v1"),
            ("https://host/openai/v1", "https://host/openai/v1"),
            ("https://host/api", "https://host/api/v1"),
            ("http://127.0.0.1:8000", "http://127.0.0.1:8000/v1"),
            ("  https://host/v1  ", "https://host/v1"),
            ("https://host/v1?x=1#frag", "https://host/v1"),
            ("HTTPS://Host.Example/V1", "https://host.example/V1/v1"),
        ];
        for (input, want) in cases {
            assert_eq!(normalize_base_url(input).as_deref(), Ok(want), "input {input:?}");
        }
    }

    #[test]
    fn normalisation_rejects_garbage() {
        for bad in ["", "   ", "host", "host/v1", "ftp://host/v1", "file:///tmp/x", "https://", "not a url"] {
            let err = normalize_base_url(bad).unwrap_err();
            assert!(matches!(err, AsrError::InvalidConfig(_)), "{bad:?} -> {err:?}");
        }
        assert_eq!(normalize_base_url("").unwrap_err(), AsrError::InvalidConfig("base_url is empty".into()));
        assert!(normalize_base_url("ftp://host").unwrap_err().to_string().contains("scheme must be http or https"));
    }

    #[test]
    fn config_builder_and_redacted_debug() {
        let config = AsrConfig::new("https://host", "whisper-1");
        assert_eq!(config.token, None);
        assert_eq!(config.timeout, DEFAULT_TIMEOUT);
        let config = config.with_token(Some("sk-very-secret".into())).with_timeout(Duration::from_secs(5));
        assert_eq!(config.timeout, Duration::from_secs(5));
        let debug = format!("{config:?}");
        assert!(!debug.contains("very-secret"), "{debug}");
        assert!(debug.contains("token: Some(\"<redacted>\")"), "{debug}");
        assert!(debug.contains("whisper-1"));
        let none = format!("{:?}", AsrConfig::new("https://host", "m"));
        assert!(none.contains("token: None"), "{none}");
        assert_eq!(config.clone(), config);
    }
}
