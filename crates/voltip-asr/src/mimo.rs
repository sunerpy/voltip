//! MiMo-ASR (docs/dictation.md §3.15): `POST {base}/chat/completions` with the take as one
//! `input_audio` part; no hot words. Long takes go in pieces that keep the part within its limit.

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::client::Transcript;
use crate::config::AsrConfig;
use crate::error::AsrError;
use crate::vendor::{RealtimeEvents, RealtimeInput, VendorAsr, not_ready};

/// The client (to be written).
pub(crate) struct MimoAsr {
    config: AsrConfig,
}

impl MimoAsr {
    /// The client of `config`.
    pub(crate) fn new(config: AsrConfig) -> Result<Self, AsrError> {
        Ok(Self { config })
    }
}

#[async_trait]
impl VendorAsr for MimoAsr {
    fn model(&self) -> &str {
        &self.config.model
    }

    fn sends_glossary(&self, _glossary: &[String]) -> bool {
        false
    }

    async fn transcribe(&self, _wav: &[u8], _language: Option<&str>, _glossary: &[String]) -> Result<Transcript, AsrError> {
        Err(not_ready("MiMo 语音识别"))
    }

    async fn probe(&self) -> Result<(), AsrError> {
        Err(not_ready("MiMo 语音识别"))
    }

    fn streams(&self) -> bool {
        false
    }

    async fn realtime(
        &self,
        _language: Option<&str>,
        _glossary: &[String],
        _input: mpsc::UnboundedReceiver<RealtimeInput>,
        _events: RealtimeEvents,
    ) -> Result<(), AsrError> {
        Err(not_ready("MiMo 语音识别"))
    }
}
