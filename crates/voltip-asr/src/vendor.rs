//! The recognition services with protocols of their own (docs/dictation.md §3.15): 豆包语音,
//! Tencent Cloud, iFlytek's dictation and realtime transcription, Baidu, StepFun, GLM-ASR and
//! MiMo-ASR. Each is a [`VendorAsr`]: whole takes, the 测试连接 check of its credential, and, for
//! the services that recognise while the take is spoken, realtime sessions.
//!
//! The settings hold every address as `http(s)` (the engines pane checks that); a client derives
//! its WebSocket endpoint from it ([`endpoint`]), so a proxy address in the URL field keeps the
//! vendor's protocol. The credential arrives in [`AsrConfig::token`]: the plain key, or for a
//! vendor with several fields the JSON object the core stored ([`crate::credentials`]).

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;
use url::Url;

use crate::client::Transcript;
use crate::config::AsrConfig;
use crate::error::AsrError;

/// Which protocol a [`client`] speaks (`voltip_core::AsrProtocol`'s vendor variants).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Vendor {
    /// 豆包语音's binary WebSocket protocol.
    Volcengine,
    /// Tencent Cloud's realtime WebSocket and its flash file API.
    Tencent,
    /// iFlytek's dictation (`iat`).
    IflytekIat,
    /// iFlytek's realtime transcription (`rtasr`).
    IflytekRtasr,
    /// Baidu's realtime WebSocket.
    Baidu,
    /// StepFun's `/audio/transcriptions`.
    StepfunTranscriptions,
    /// StepFun's realtime WebSocket (`…-stream` models).
    StepfunRealtime,
    /// GLM-ASR's `/audio/transcriptions`.
    Zhipu,
    /// MiMo-ASR through `/chat/completions`.
    Mimo,
}

/// What a realtime session's caller sends it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RealtimeInput {
    /// 16-bit little-endian PCM, 16 kHz mono, of any length: the client frames it for its service.
    Audio(Vec<u8>),
    /// No more audio: the service finishes the last sentence and says so.
    Finish,
}

/// What a realtime session says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RealtimeEvent {
    /// The sentence being spoken, whole so far (it may still change).
    Partial {
        /// Its text.
        text: String,
    },
    /// A finished sentence. Its times are milliseconds of the audio sent since the session
    /// started, whichever connection of it carried the sentence (`voltip_core::dictation::StreamEvent`).
    Sentence {
        /// Its text, punctuation included.
        text: String,
        /// Start.
        begin_ms: u64,
        /// End.
        end_ms: u64,
    },
    /// Everything was said: the last event of a session that ended well.
    Finished,
}

/// Where a realtime session sends what it hears.
pub type RealtimeEvents = mpsc::UnboundedSender<RealtimeEvent>;

/// One vendor's recognition client.
#[async_trait]
pub trait VendorAsr: Send + Sync {
    /// The model its requests name: the take's receipt.
    fn model(&self) -> &str;

    /// Whether a request with `glossary` sends terms to the service (`Transcript.glossary_sent`):
    /// `false` for a service that takes none, or when the client's limits leave none of them.
    fn sends_glossary(&self, glossary: &[String]) -> bool;

    /// Recognise a whole take (a 16-bit mono PCM WAV), the glossary as the service takes it.
    async fn transcribe(&self, wav: &[u8], language: Option<&str>, glossary: &[String]) -> Result<Transcript, AsrError>;

    /// 测试连接: send the smallest valid request and return once the service said it succeeded;
    /// a completed handshake alone is not success (docs/dictation.md §3.15).
    async fn probe(&self) -> Result<(), AsrError>;

    /// The service recognises while the take is spoken: [`VendorAsr::realtime`] runs sessions.
    fn streams(&self) -> bool;

    /// Run one realtime session: connect, send what `input` brings, and pass what the service says
    /// to `events` until it has said everything, ending with [`RealtimeEvent::Finished`]. An error
    /// ends the session with it. When `input` closes before [`RealtimeInput::Finish`] (a cancelled
    /// take) the connection closes at once and nothing more is sent.
    async fn realtime(
        &self,
        language: Option<&str>,
        glossary: &[String],
        input: mpsc::UnboundedReceiver<RealtimeInput>,
        events: RealtimeEvents,
    ) -> Result<(), AsrError>;
}

/// The client of `vendor` for `config`: the stored base URL, the model, the credential in
/// [`AsrConfig::token`]. Fails on a configuration it cannot use (a bad URL, an empty model, a
/// credential that is missing or misses a field).
pub fn client(vendor: Vendor, config: AsrConfig) -> Result<Arc<dyn VendorAsr>, AsrError> {
    if config.model.trim().is_empty() {
        return Err(AsrError::InvalidConfig("model is empty".into()));
    }
    Ok(match vendor {
        Vendor::Volcengine => Arc::new(crate::volcengine::VolcengineAsr::new(config)?),
        Vendor::Tencent => Arc::new(crate::tencent::TencentAsr::new(config)?),
        Vendor::IflytekIat => Arc::new(crate::iflytek::IatAsr::new(config)?),
        Vendor::IflytekRtasr => Arc::new(crate::iflytek::RtasrAsr::new(config)?),
        Vendor::Baidu => Arc::new(crate::baidu::BaiduAsr::new(config)?),
        Vendor::StepfunTranscriptions | Vendor::StepfunRealtime => Arc::new(crate::stepfun::StepfunAsr::new(config)?),
        Vendor::Zhipu => Arc::new(crate::zhipu::ZhipuAsr::new(config)?),
        Vendor::Mimo => Arc::new(crate::mimo::MimoAsr::new(config)?),
    })
}

/// The endpoint `path` under the stored `base` (`https://host[/prefix]`): the base's own path
/// stays in front of `path` (a proxy's prefix), query and fragment are dropped, and with `ws` the
/// scheme becomes `wss` (`ws` for a plain `http` base, as the tests use).
pub fn endpoint(base: &str, ws: bool, path: &str) -> Result<Url, AsrError> {
    let raw = base.trim();
    if raw.is_empty() {
        return Err(AsrError::InvalidConfig("base_url is empty".into()));
    }
    let mut url = Url::parse(raw).map_err(|e| AsrError::InvalidConfig(format!("base_url {raw:?}: {e}")))?;
    let secure = match url.scheme() {
        "https" => true,
        "http" => false,
        _ => return Err(AsrError::InvalidConfig(format!("base_url {raw:?}: scheme must be http or https"))),
    };
    if url.host_str().is_none() {
        return Err(AsrError::InvalidConfig(format!("base_url {raw:?}: missing host")));
    }
    let joined = format!("{}/{}", url.path().trim_end_matches('/'), path.trim_start_matches('/'));
    url.set_path(&joined);
    url.set_query(None);
    url.set_fragment(None);
    if ws {
        url.set_scheme(if secure { "wss" } else { "ws" }).map_err(|()| AsrError::InvalidConfig(format!("base_url {raw:?}: no WebSocket form")))?;
    }
    Ok(url)
}

/// The error of a vendor whose client is not finished.
pub(crate) fn not_ready(vendor: &str) -> AsrError {
    AsrError::InvalidConfig(format!("{vendor}: this client is not implemented yet"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_hang_off_the_stored_base() {
        let cases = [
            ("https://openspeech.bytedance.com", true, "/api/v3/sauc/bigmodel_async", "wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_async"),
            ("https://openspeech.bytedance.com/", true, "api/v3/sauc/bigmodel_async", "wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_async"),
            ("https://proxy.example.test/volc/", true, "/api/v3/sauc/bigmodel_async", "wss://proxy.example.test/volc/api/v3/sauc/bigmodel_async"),
            ("http://127.0.0.1:9000", true, "/v2/iat", "ws://127.0.0.1:9000/v2/iat"),
            ("https://api.stepfun.com/v1", false, "/audio/transcriptions", "https://api.stepfun.com/v1/audio/transcriptions"),
            ("https://api.stepfun.com/v1?x=1#f", true, "/realtime/asr/stream", "wss://api.stepfun.com/v1/realtime/asr/stream"),
            ("  https://asr.cloud.tencent.com  ", false, "/asr/flash/v1/1250000000", "https://asr.cloud.tencent.com/asr/flash/v1/1250000000"),
        ];
        for (base, ws, path, want) in cases {
            assert_eq!(endpoint(base, ws, path).map(|u| u.to_string()).as_deref(), Ok(want), "{base} {path}");
        }
        for bad in ["", "wss://openspeech.bytedance.com", "not a url", "file:///tmp/x"] {
            assert!(matches!(endpoint(bad, true, "/x"), Err(AsrError::InvalidConfig(_))), "{bad}");
        }
    }

    #[test]
    fn a_client_needs_a_model() {
        let config = AsrConfig::new("https://iat-api.xfyun.cn", " ");
        assert!(matches!(client(Vendor::IflytekIat, config), Err(AsrError::InvalidConfig(_))));
    }
}
