//! Client configuration and base-URL normalisation.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use url::Url;
use voltip_net::{Network, NetworkRef};

use crate::AsrError;

/// Default [`AsrConfig::timeout`]: a 120 s recording through a busy GPU still fits.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
/// Longest error body kept in [`AsrError::Server`].
pub const MAX_ERROR_BODY_CHARS: usize = 200;
/// The answer header in which the built-in service reports the device's usage and quota
/// (docs/dictation.md §27).
pub const QUOTA_HEADER: &str = "x-voltip-quota";

/// The built-in service's metering (docs/dictation.md §27): the headers every request carries
/// (the device and the client), and who hears the usage report of an answer ([`QUOTA_HEADER`]),
/// refusals included. Only the built-in service is metered; `Debug` names the headers only.
#[derive(Clone)]
pub struct Metering {
    /// Sent with every request.
    pub headers: Vec<(String, String)>,
    /// Called with the [`QUOTA_HEADER`] of every answer that carries one.
    pub on_report: Arc<dyn Fn(&str) + Send + Sync>,
}

impl PartialEq for Metering {
    /// The same headers: a callback cannot be compared.
    fn eq(&self, other: &Self) -> bool {
        self.headers == other.headers
    }
}

impl Eq for Metering {}

impl fmt::Debug for Metering {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Metering").field("headers", &self.headers.iter().map(|(name, _)| name).collect::<Vec<_>>()).finish_non_exhaustive()
    }
}

/// When the quota a report ([`QUOTA_HEADER`]) is about is reset: its `reset=` in Unix seconds, as
/// milliseconds.
pub(crate) fn report_reset_ms(report: &str) -> Option<u64> {
    report.split(';').find_map(|part| part.trim().strip_prefix("reset="))?.trim().parse::<u64>().ok()?.checked_mul(1000)
}

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
    /// What the OpenAI-compatible multipart's `file` carries (Model Studio's protocols always send
    /// the WAV).
    pub upload: AudioUpload,
    /// The proxy plan the requests and the realtime WebSocket follow (docs/dictation.md §24); the
    /// process's system-following network unless one is handed in.
    pub network: NetworkRef,
    /// The built-in service's metering; `None` for every other service.
    pub metering: Option<Metering>,
}

/// The audio a transcription request uploads (docs/dictation.md §3.9).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AudioUpload {
    /// The WAV as it is (`audio.wav`): every service takes it.
    #[default]
    Wav,
    /// Ogg Opus at 24 kbit/s (`audio.ogg`), about a tenth of the WAV, for a service known to decode
    /// it. A WAV Opus cannot take as it is (not 16-bit mono, another rate) still goes as the WAV.
    OggOpus,
}

impl AsrConfig {
    /// A configuration without a token, with the default timeout, uploading the WAV.
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            token: None,
            model: model.into(),
            timeout: DEFAULT_TIMEOUT,
            upload: AudioUpload::Wav,
            network: NetworkRef::default(),
            metering: None,
        }
    }

    /// Meter the requests ([`Metering`]).
    pub fn with_metering(mut self, metering: Option<Metering>) -> Self {
        self.metering = metering;
        self
    }

    /// Go through `network`'s proxy plan.
    pub fn with_network(mut self, network: &Arc<Network>) -> Self {
        self.network = NetworkRef::new(network);
        self
    }

    /// Set what the requests upload.
    pub fn with_upload(mut self, upload: AudioUpload) -> Self {
        self.upload = upload;
        self
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
            .field("upload", &self.upload)
            .field("network", &self.network)
            .field("metering", &self.metering)
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
        assert_eq!(config.upload, AudioUpload::Wav, "the format every service takes");
        let config = config.with_token(Some("sk-very-secret".into())).with_timeout(Duration::from_secs(5)).with_upload(AudioUpload::OggOpus);
        assert_eq!((config.timeout, config.upload), (Duration::from_secs(5), AudioUpload::OggOpus));
        let debug = format!("{config:?}");
        assert!(!debug.contains("very-secret"), "{debug}");
        assert!(debug.contains("token: Some(\"<redacted>\")"), "{debug}");
        assert!(debug.contains("whisper-1"));
        let none = format!("{:?}", AsrConfig::new("https://host", "m"));
        assert!(none.contains("token: None"), "{none}");
        assert_eq!(config.clone(), config);
    }

    #[test]
    fn metering_compares_its_headers_and_debug_names_them_only() {
        let metering = |device: &str| Metering { headers: vec![("x-voltip-device".into(), device.into())], on_report: Arc::new(|_| {}) };
        assert_eq!(metering("a"), metering("a"), "the callback is not compared");
        assert_ne!(metering("a"), metering("b"));
        let debug = format!("{:?}", AsrConfig::new("https://host", "m").with_metering(Some(metering("6f1c…"))));
        assert!(debug.contains("x-voltip-device") && !debug.contains("6f1c"), "{debug}");
    }

    #[test]
    fn the_reset_of_a_report_is_read_in_milliseconds() {
        let report = "mode=limit; basis=device; day=2026-10-27; reset=1793116800; asr=3600/3600; refine=12/100";
        assert_eq!(report_reset_ms(report), Some(1_793_116_800_000));
        assert_eq!(report_reset_ms("reset = 5"), None, "the key is `reset=`");
        assert_eq!(report_reset_ms("mode=count"), None);
        assert_eq!(report_reset_ms("reset=soon"), None);
    }
}
