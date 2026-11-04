//! Baidu AI Cloud realtime recognition (docs/dictation.md §3.15): `/realtime_asr`, a START frame
//! with the AppID and API Key, binary audio, FINISH; recorded takes go the same way. The model is
//! the `dev_pid`.

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::client::Transcript;
use crate::config::AsrConfig;
use crate::error::AsrError;
use crate::vendor::{RealtimeEvents, RealtimeInput, VendorAsr, not_ready};

/// The client (to be written).
pub(crate) struct BaiduAsr {
    config: AsrConfig,
}

impl BaiduAsr {
    /// The client of `config`.
    pub(crate) fn new(config: AsrConfig) -> Result<Self, AsrError> {
        Ok(Self { config })
    }
}

#[async_trait]
impl VendorAsr for BaiduAsr {
    fn model(&self) -> &str {
        &self.config.model
    }

    fn sends_glossary(&self, _glossary: &[String]) -> bool {
        false
    }

    async fn transcribe(&self, _wav: &[u8], _language: Option<&str>, _glossary: &[String]) -> Result<Transcript, AsrError> {
        Err(not_ready("百度语音识别"))
    }

    async fn probe(&self) -> Result<(), AsrError> {
        Err(not_ready("百度语音识别"))
    }

    fn streams(&self) -> bool {
        true
    }

    async fn realtime(
        &self,
        _language: Option<&str>,
        _glossary: &[String],
        _input: mpsc::UnboundedReceiver<RealtimeInput>,
        _events: RealtimeEvents,
    ) -> Result<(), AsrError> {
        Err(not_ready("百度语音识别"))
    }
}
