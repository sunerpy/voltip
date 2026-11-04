//! iFlytek (docs/dictation.md §3.15): dictation (`iat`, `/v2/iat`, HMAC-SHA256 signed, 60 seconds
//! a session; a live take longer than that continues on a new session) and realtime transcription
//! (`rtasr`, `/v1/ws`, signed with `signa`, no length limit).

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::client::Transcript;
use crate::config::AsrConfig;
use crate::error::AsrError;
use crate::vendor::{RealtimeEvents, RealtimeInput, VendorAsr, not_ready};

/// The client (to be written).
pub(crate) struct IatAsr {
    config: AsrConfig,
}

impl IatAsr {
    /// The client of `config`.
    pub(crate) fn new(config: AsrConfig) -> Result<Self, AsrError> {
        Ok(Self { config })
    }
}

#[async_trait]
impl VendorAsr for IatAsr {
    fn model(&self) -> &str {
        &self.config.model
    }

    fn sends_glossary(&self, _glossary: &[String]) -> bool {
        false
    }

    async fn transcribe(&self, _wav: &[u8], _language: Option<&str>, _glossary: &[String]) -> Result<Transcript, AsrError> {
        Err(not_ready("讯飞语音听写"))
    }

    async fn probe(&self) -> Result<(), AsrError> {
        Err(not_ready("讯飞语音听写"))
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
        Err(not_ready("讯飞语音听写"))
    }
}

/// The client (to be written).
pub(crate) struct RtasrAsr {
    config: AsrConfig,
}

impl RtasrAsr {
    /// The client of `config`.
    pub(crate) fn new(config: AsrConfig) -> Result<Self, AsrError> {
        Ok(Self { config })
    }
}

#[async_trait]
impl VendorAsr for RtasrAsr {
    fn model(&self) -> &str {
        &self.config.model
    }

    fn sends_glossary(&self, _glossary: &[String]) -> bool {
        false
    }

    async fn transcribe(&self, _wav: &[u8], _language: Option<&str>, _glossary: &[String]) -> Result<Transcript, AsrError> {
        Err(not_ready("讯飞实时语音转写"))
    }

    async fn probe(&self) -> Result<(), AsrError> {
        Err(not_ready("讯飞实时语音转写"))
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
        Err(not_ready("讯飞实时语音转写"))
    }
}
