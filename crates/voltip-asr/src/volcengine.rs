//! 豆包语音, Volcengine's speech service (docs/dictation.md §3.15): Seed-ASR over its binary
//! WebSocket protocol. Live takes go to `bigmodel_async` (双向流式优化版, with 二遍识别), recorded
//! takes to `bigmodel_nostream` (流式输入). The credential is the new console's API key
//! (`X-Api-Key`) or the old one's App ID and Access Token; the model is the `X-Api-Resource-Id`.

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::client::Transcript;
use crate::config::AsrConfig;
use crate::error::AsrError;
use crate::vendor::{RealtimeEvents, RealtimeInput, VendorAsr, not_ready};

/// The client (to be written).
pub(crate) struct VolcengineAsr {
    config: AsrConfig,
}

impl VolcengineAsr {
    /// The client of `config`.
    pub(crate) fn new(config: AsrConfig) -> Result<Self, AsrError> {
        Ok(Self { config })
    }
}

#[async_trait]
impl VendorAsr for VolcengineAsr {
    fn model(&self) -> &str {
        &self.config.model
    }

    fn sends_glossary(&self, _glossary: &[String]) -> bool {
        false
    }

    async fn transcribe(&self, _wav: &[u8], _language: Option<&str>, _glossary: &[String]) -> Result<Transcript, AsrError> {
        Err(not_ready("豆包语音"))
    }

    async fn probe(&self) -> Result<(), AsrError> {
        Err(not_ready("豆包语音"))
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
        Err(not_ready("豆包语音"))
    }
}
