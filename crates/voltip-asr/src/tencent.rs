//! Tencent Cloud ASR (docs/dictation.md §3.15): live takes over the signed realtime WebSocket
//! (`/asr/v2/<appid>`), recorded takes through the flash file API (`/asr/flash/v1/<appid>`); the
//! model is the `engine_model_type`. The credential is the AppId, SecretId and SecretKey.

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::client::Transcript;
use crate::config::AsrConfig;
use crate::error::AsrError;
use crate::vendor::{RealtimeEvents, RealtimeInput, VendorAsr, not_ready};

/// The client (to be written).
pub(crate) struct TencentAsr {
    config: AsrConfig,
}

impl TencentAsr {
    /// The client of `config`.
    pub(crate) fn new(config: AsrConfig) -> Result<Self, AsrError> {
        Ok(Self { config })
    }
}

#[async_trait]
impl VendorAsr for TencentAsr {
    fn model(&self) -> &str {
        &self.config.model
    }

    fn sends_glossary(&self, _glossary: &[String]) -> bool {
        false
    }

    async fn transcribe(&self, _wav: &[u8], _language: Option<&str>, _glossary: &[String]) -> Result<Transcript, AsrError> {
        Err(not_ready("腾讯云语音识别"))
    }

    async fn probe(&self) -> Result<(), AsrError> {
        Err(not_ready("腾讯云语音识别"))
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
        Err(not_ready("腾讯云语音识别"))
    }
}
