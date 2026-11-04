//! StepFun (docs/dictation.md §3.15): `stepaudio-2.5-asr` takes whole files at
//! `POST {base}/audio/transcriptions`; the `…-stream` models recognise live over
//! `wss://…/realtime/asr/stream`, and their recorded takes go to the whole-file model.

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::client::Transcript;
use crate::config::AsrConfig;
use crate::error::AsrError;
use crate::vendor::{RealtimeEvents, RealtimeInput, VendorAsr, not_ready};

/// The client (to be written).
pub(crate) struct StepfunAsr {
    config: AsrConfig,
}

impl StepfunAsr {
    /// The client of `config`.
    pub(crate) fn new(config: AsrConfig) -> Result<Self, AsrError> {
        Ok(Self { config })
    }
}

#[async_trait]
impl VendorAsr for StepfunAsr {
    fn model(&self) -> &str {
        &self.config.model
    }

    fn sends_glossary(&self, _glossary: &[String]) -> bool {
        false
    }

    async fn transcribe(&self, _wav: &[u8], _language: Option<&str>, _glossary: &[String]) -> Result<Transcript, AsrError> {
        Err(not_ready("阶跃星辰语音识别"))
    }

    async fn probe(&self) -> Result<(), AsrError> {
        Err(not_ready("阶跃星辰语音识别"))
    }

    fn streams(&self) -> bool {
        self.config.model.trim().to_ascii_lowercase().ends_with("-stream")
    }

    async fn realtime(
        &self,
        _language: Option<&str>,
        _glossary: &[String],
        _input: mpsc::UnboundedReceiver<RealtimeInput>,
        _events: RealtimeEvents,
    ) -> Result<(), AsrError> {
        Err(not_ready("阶跃星辰语音识别"))
    }
}
