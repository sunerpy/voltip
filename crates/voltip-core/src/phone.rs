//! The phone as the desktop's microphone (docs/dictation.md §20): what the phone's UI shows about
//! its take, and how the desktop's dictation phases become the [`TakeState`]s it reports back.
//! The wire messages are [`voltip_protocol::app::AppMessage`]'s `Take*` variants.

use serde::{Deserialize, Serialize};
pub use voltip_protocol::app::{TAKE_SAMPLE_RATE_HZ, TakeFailure, TakeState};

use crate::dictation::{DictationPhase, FailureCode, Via};

/// Why the phone's take ended without delivering. The first four come from the desktop
/// ([`TakeFailure`]); the last two are the phone's own.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhoneTakeFailure {
    /// The desktop is running another take.
    Busy,
    /// The desktop has no recognition configured, or does not take phone audio.
    Unavailable,
    /// The desktop heard nothing.
    NoSpeech,
    /// Recognition, clean-up or delivery failed on the desktop.
    Failed,
    /// This phone's microphone could not be opened (permission, device busy).
    Microphone,
    /// The desktop went offline during the take.
    Offline,
}

impl From<TakeFailure> for PhoneTakeFailure {
    fn from(code: TakeFailure) -> Self {
        match code {
            TakeFailure::Busy => Self::Busy,
            TakeFailure::Unavailable => Self::Unavailable,
            TakeFailure::NoSpeech => Self::NoSpeech,
            TakeFailure::Failed => Self::Failed,
        }
    }
}

/// Where the phone's take is.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PhoneTakeState {
    /// The microphone is opening; the desktop has not answered yet.
    Starting,
    /// Streaming: the desktop records.
    Listening,
    /// Released: the desktop recognises, cleans up and delivers.
    Processing,
    /// Delivered on the desktop, at the cursor (`pasted`) or on its clipboard.
    Done {
        /// What was delivered.
        text: String,
        /// Pasted rather than left on the clipboard.
        pasted: bool,
    },
    /// Did not deliver.
    Failed {
        /// Why.
        code: PhoneTakeFailure,
        /// The explanation (the desktop's, or the phone's own).
        message: String,
    },
    /// Discarded.
    Cancelled,
}

impl PhoneTakeState {
    /// The take is over.
    pub fn is_final(&self) -> bool {
        matches!(self, Self::Done { .. } | Self::Failed { .. } | Self::Cancelled)
    }
}

impl From<TakeState> for PhoneTakeState {
    fn from(state: TakeState) -> Self {
        match state {
            TakeState::Listening => Self::Listening,
            TakeState::Processing => Self::Processing,
            TakeState::Done { text, pasted } => Self::Done { text, pasted },
            TakeState::Failed { code, message } => Self::Failed { code: code.into(), message },
            TakeState::Cancelled => Self::Cancelled,
        }
    }
}

/// The phone's current (or last) take, for its UI (`UiState.phone_take`).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct PhoneTakeView {
    /// The desktop the audio goes to (lower-case hex public key, as in `DeviceView`).
    pub device: String,
    /// The phone's id for the take.
    pub take: u32,
    /// When the phone started it (Unix milliseconds): the UI's timer.
    pub started_at: u64,
    /// Where it is.
    pub state: PhoneTakeState,
}

/// What the desktop reports to the phone for one of its dictation phases; `None` for `Idle`
/// (the take is over and was reported already).
pub fn take_state_for(phase: &DictationPhase) -> Option<TakeState> {
    match phase {
        DictationPhase::Idle => None,
        DictationPhase::Listening { .. } => Some(TakeState::Listening),
        DictationPhase::Processing { .. } => Some(TakeState::Processing),
        DictationPhase::Done { text, via, .. } => Some(TakeState::done(text, *via == Via::Paste)),
        DictationPhase::Failed { code, message, .. } => {
            let failure = match code {
                FailureCode::NoSpeech => TakeFailure::NoSpeech,
                _ => TakeFailure::Failed,
            };
            Some(TakeState::failed(failure, message))
        }
        DictationPhase::Cancelled { .. } => Some(TakeState::Cancelled),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dictation::{OutputMode, ProcessingStage};

    #[test]
    fn desktop_phases_become_the_phones_take_states() {
        assert_eq!(take_state_for(&DictationPhase::Idle), None);
        assert_eq!(take_state_for(&DictationPhase::Listening { started_at: 1, ready: true, live: None, locked: false }), Some(TakeState::Listening));
        assert_eq!(
            take_state_for(&DictationPhase::Processing { stage: ProcessingStage::Transcribing, started_at: 1, preview: None }),
            Some(TakeState::Processing)
        );
        let done = DictationPhase::Done {
            text: "你好".into(),
            raw_text: "你好".into(),
            chars: 2,
            via: Via::Clipboard,
            refined: false,
            duration_ms: 900,
            asr_ms: 300,
            refine_ms: None,
            refine_error: None,
            mode: OutputMode::WholeTake,
            segments: None,
            live_error: None,
        };
        assert_eq!(take_state_for(&done), Some(TakeState::Done { text: "你好".into(), pasted: false }));
        let silent = DictationPhase::Failed { code: FailureCode::NoSpeech, message: "没有听到声音".into(), text: None };
        assert_eq!(take_state_for(&silent), Some(TakeState::Failed { code: TakeFailure::NoSpeech, message: "没有听到声音".into() }));
        let asr = DictationPhase::Failed { code: FailureCode::Asr, message: "asr: 401".into(), text: None };
        assert!(matches!(take_state_for(&asr), Some(TakeState::Failed { code: TakeFailure::Failed, .. })));
        assert_eq!(take_state_for(&DictationPhase::Cancelled { injected_chars: 0 }), Some(TakeState::Cancelled));
        // …and the phone reads them back.
        let back: PhoneTakeState = TakeState::failed(TakeFailure::Busy, "正在听写").into();
        assert_eq!(back, PhoneTakeState::Failed { code: PhoneTakeFailure::Busy, message: "正在听写".into() });
        assert!(back.is_final() && !PhoneTakeState::Starting.is_final());
        let json = serde_json::to_string(&PhoneTakeState::Done { text: "x".into(), pasted: true }).unwrap();
        assert_eq!(json, r#"{"state":"done","text":"x","pasted":true}"#);
    }
}
