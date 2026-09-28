//! The phone as the desktop's microphone and keyboard (docs/dictation.md §20): what the phone's UI
//! shows about its take and the texts it sent, and how the desktop's dictation phases become the
//! [`TakeState`]s it reports back. The wire messages are [`voltip_protocol::app::AppMessage`]'s
//! `Take*` and `PhoneText*` variants.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
pub use voltip_protocol::app::{MAX_PHONE_TEXT_CHARS, PhoneTextFailure, PhoneTextSource, PhoneTextState, TAKE_SAMPLE_RATE_HZ, TakeFailure, TakeState};

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
    /// The audio goes out as Opus (the desktop said it decodes it, docs/dictation.md §20.1)
    /// rather than PCM. Absent in payloads from before it (= PCM).
    #[serde(default)]
    pub opus: bool,
}

/// Why a text the phone sent did not arrive. The first three come from the desktop
/// ([`PhoneTextFailure`]); the last two are the phone's own.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SentTextFailure {
    /// Too many texts are waiting on the desktop.
    Busy,
    /// The desktop does not insert texts from a phone.
    Unavailable,
    /// The insertion failed on the desktop.
    Failed,
    /// The desktop was not online.
    Offline,
    /// The desktop never answered (an older version, or it went away).
    NoAnswer,
}

impl From<PhoneTextFailure> for SentTextFailure {
    fn from(code: PhoneTextFailure) -> Self {
        match code {
            PhoneTextFailure::Busy => Self::Busy,
            PhoneTextFailure::Unavailable => Self::Unavailable,
            PhoneTextFailure::Failed => Self::Failed,
        }
    }
}

/// Where a text the phone sent is.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SentTextState {
    /// On its way; the desktop has not answered yet.
    Sending,
    /// The desktop has it and inserts it once its own take is over.
    Queued,
    /// Inserted at the desktop's cursor (`pasted`) or left on its clipboard.
    Delivered {
        /// Pasted rather than left on the clipboard.
        pasted: bool,
    },
    /// Did not arrive.
    Failed {
        /// Why.
        code: SentTextFailure,
        /// The explanation (the desktop's, or the phone's own).
        message: String,
    },
}

impl SentTextState {
    /// No answer is expected any more.
    pub fn is_final(&self) -> bool {
        matches!(self, Self::Delivered { .. } | Self::Failed { .. })
    }
}

impl From<PhoneTextState> for SentTextState {
    fn from(state: PhoneTextState) -> Self {
        match state {
            PhoneTextState::Queued => Self::Queued,
            PhoneTextState::Delivered { pasted } => Self::Delivered { pasted },
            PhoneTextState::Failed { code, message } => Self::Failed { code: code.into(), message },
        }
    }
}

/// A text the phone sent to a desktop (docs/dictation.md §20.6), for its list.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct SentText {
    /// The phone's id for it (unique across restarts: the store remembers the last one).
    pub id: u32,
    /// The desktop (lower-case hex public key).
    pub device: String,
    /// The desktop's name when it was sent.
    pub device_name: String,
    /// What was sent.
    pub body: String,
    /// Typed or the clipboard.
    pub source: PhoneTextSource,
    /// When (Unix milliseconds).
    pub sent_at: u64,
    /// Where it is.
    pub state: SentTextState,
}

/// Most texts the phone keeps in its list; older ones are dropped.
pub const MAX_SENT_TEXTS: usize = 50;
/// File the list lives in, next to `settings.json`.
pub const SENT_TEXTS_FILE_NAME: &str = "sent-texts.json";
const SENT_TEXTS_SCHEMA: u16 = 1;

#[derive(Serialize, Deserialize)]
struct SentTextsFile {
    schema: u16,
    texts: Vec<SentText>,
}

/// The phone's list of sent texts, newest first, persisted to [`SENT_TEXTS_FILE_NAME`]. A missing,
/// corrupt or unknown-schema file reads as empty (the list is a convenience, never a reason to fail).
#[derive(Debug)]
pub struct SentTexts {
    path: PathBuf,
    texts: Vec<SentText>,
}

impl SentTexts {
    /// Open `dir/sent-texts.json`.
    pub fn open(dir: &Path) -> Self {
        let path = dir.join(SENT_TEXTS_FILE_NAME);
        let texts = std::fs::read(&path)
            .ok()
            .and_then(|bytes| match serde_json::from_slice::<SentTextsFile>(&bytes) {
                Ok(file) if file.schema == SENT_TEXTS_SCHEMA => Some(file.texts),
                Ok(file) => {
                    tracing::warn!(schema = file.schema, "sent texts: unknown schema; starting empty");
                    None
                }
                Err(e) => {
                    tracing::warn!(error = %e, "sent texts: unreadable file; starting empty");
                    None
                }
            })
            .unwrap_or_default();
        Self { path, texts }
    }

    /// Newest first.
    pub fn texts(&self) -> &[SentText] {
        &self.texts
    }

    /// The highest id in the list (the next text takes a higher one).
    pub fn last_id(&self) -> u32 {
        self.texts.iter().map(|t| t.id).max().unwrap_or(0)
    }

    /// Put `text` first, drop what falls past [`MAX_SENT_TEXTS`], save.
    pub fn push(&mut self, text: SentText) {
        self.texts.insert(0, text);
        self.texts.truncate(MAX_SENT_TEXTS);
        self.save();
    }

    /// Set the state of the text `id` sent to `device`; whether one matched (and was saved).
    pub fn update(&mut self, device: &str, id: u32, state: SentTextState) -> bool {
        let Some(text) = self.texts.iter_mut().find(|t| t.id == id && t.device == device) else { return false };
        if text.state == state {
            return false;
        }
        text.state = state;
        self.save();
        true
    }

    /// Apply `f` to every text still waiting for an answer; saves when any changed.
    pub fn expire(&mut self, mut f: impl FnMut(&SentText) -> Option<SentTextState>) -> bool {
        let mut changed = false;
        for text in self.texts.iter_mut().filter(|t| !t.state.is_final()) {
            if let Some(state) = f(text) {
                text.state = state;
                changed = true;
            }
        }
        if changed {
            self.save();
        }
        changed
    }

    /// Forget every text.
    pub fn clear(&mut self) {
        self.texts.clear();
        self.save();
    }

    fn save(&self) {
        let write = || -> std::io::Result<()> {
            if let Some(dir) = self.path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let file = SentTextsFile { schema: SENT_TEXTS_SCHEMA, texts: self.texts.clone() };
            let bytes = serde_json::to_vec_pretty(&file).map_err(std::io::Error::other)?;
            let tmp = self.path.with_extension("json.tmp");
            std::fs::write(&tmp, bytes)?;
            std::fs::rename(&tmp, &self.path)
        };
        if let Err(e) = write() {
            tracing::warn!(error = %e, "sent texts: could not save the list");
        }
    }
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

    fn sent(id: u32, state: SentTextState) -> SentText {
        SentText { id, device: "ab".repeat(32), device_name: "Studio".into(), body: format!("第 {id} 段"), source: PhoneTextSource::Typed, sent_at: 1, state }
    }

    /// docs/dictation.md §20.6: the phone's list is newest first, capped, persisted, and a broken
    /// file only means an empty list.
    #[test]
    fn the_sent_list_is_newest_first_capped_and_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let mut list = SentTexts::open(dir.path());
        assert!(list.texts().is_empty() && list.last_id() == 0);
        for id in 1..=(MAX_SENT_TEXTS as u32 + 3) {
            list.push(sent(id, SentTextState::Sending));
        }
        assert_eq!(list.texts().len(), MAX_SENT_TEXTS);
        assert_eq!(list.texts()[0].id, MAX_SENT_TEXTS as u32 + 3);
        let device = "ab".repeat(32);
        assert!(list.update(&device, 53, SentTextState::Delivered { pasted: true }));
        assert!(!list.update(&device, 53, SentTextState::Delivered { pasted: true }), "no change, no save");
        assert!(!list.update("cd", 52, SentTextState::Queued), "another desktop's id");
        // Only the texts still waiting are offered to `expire`.
        let mut offered = 0;
        assert!(list.expire(|t| {
            offered += 1;
            (t.id == 52).then(|| SentTextState::Failed { code: SentTextFailure::NoAnswer, message: "没有回应".into() })
        }));
        assert_eq!(offered, MAX_SENT_TEXTS - 1);
        let again = SentTexts::open(dir.path());
        assert_eq!(again.texts(), list.texts());
        assert_eq!(again.last_id(), MAX_SENT_TEXTS as u32 + 3);
        assert!(matches!(again.texts()[1].state, SentTextState::Failed { code: SentTextFailure::NoAnswer, .. }));
        std::fs::write(dir.path().join(SENT_TEXTS_FILE_NAME), b"{not json").unwrap();
        assert!(SentTexts::open(dir.path()).texts().is_empty());
        let mut cleared = SentTexts::open(dir.path());
        cleared.push(sent(9, SentTextState::Queued));
        cleared.clear();
        assert!(SentTexts::open(dir.path()).texts().is_empty());
        // The desktop's answers read back as the phone's states.
        assert_eq!(
            SentTextState::from(PhoneTextState::failed(PhoneTextFailure::Busy, "满了")),
            SentTextState::Failed { code: SentTextFailure::Busy, message: "满了".into() }
        );
        assert!(SentTextState::Delivered { pasted: false }.is_final() && !SentTextState::Queued.is_final());
    }

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
