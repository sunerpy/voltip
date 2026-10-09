//! Text a phone sends for the desktop to insert (docs/dictation.md §20.6).
//!
//! **Phone.** [`CoreCommand::PhoneTextSend`](super::CoreCommand::PhoneTextSend) seals an
//! [`AppMessage::PhoneText`] for an online trusted desktop and puts it first in the phone's list
//! ([`SentTexts`]); the desktop's [`AppMessage::PhoneTextStatus`] moves it along. A text nobody
//! answers is given up on: [`NO_ANSWER_AFTER`] while sending, [`QUEUED_GIVE_UP`] while queued.
//!
//! **Desktop.** A trusted phone's text is inserted through the same injector a dictation uses
//! (paste, else the clipboard), one at a time and only while no take is running: texts that come
//! during a take wait in a queue of at most [`MAX_QUEUED_TEXTS`], and the phone hears `queued`.
//! Every delivered text goes into the history with its [`EntryOrigin`]. The same text arriving on
//! a second path is recognised by its id and dropped.

use std::collections::VecDeque;
use std::time::Duration;

use uuid::Uuid;
use voltip_crypto::PublicKey;
use voltip_protocol::ProtocolVersion;
use voltip_protocol::app::{AppMessage, MAX_PHONE_TEXT_CHARS, PhoneTextFailure, PhoneTextSource, PhoneTextState};

use super::takes::PhoneEvent;
use super::{CoreEvent, Runtime, now_ms};
use crate::CoreError;
use crate::dictation::{DictationError, DictationPhase, Injection, OutputMode, TakeKind, Via};
use crate::history::{EntryOrigin, HistoryEntry, OriginKind, Outcome};
use crate::paste::{PasteFailure, PasteOutcome, PasteTarget, paste_blocking, valid_paste_text};
use crate::phone::{SentText, SentTextFailure, SentTextState};

/// Most texts a desktop holds while its own take runs.
pub(super) const MAX_QUEUED_TEXTS: usize = 10;
/// A text the desktop has not acknowledged in this long failed (an older desktop drops the message).
pub(super) const NO_ANSWER_AFTER: Duration = Duration::from_secs(15);
/// A queued text the desktop has not delivered in this long is given up on.
pub(super) const QUEUED_GIVE_UP: Duration = Duration::from_secs(600);
/// How many recent `(phone, id)` pairs the desktop remembers to drop second-path duplicates.
const SEEN_TEXTS: usize = 64;

/// Desktop: a phone's text waiting for (or being) inserted.
pub(super) struct IncomingText {
    peer: PublicKey,
    id: u32,
    body: String,
    source: PhoneTextSource,
    /// The phone's name, for the history entry.
    name: String,
}

/// Desktop: the queue and what was seen.
#[derive(Default)]
pub(super) struct TextInbox {
    queue: VecDeque<IncomingText>,
    /// A text is with the injector.
    busy: bool,
    seen: VecDeque<(PublicKey, u32)>,
}

fn origin_kind(source: PhoneTextSource) -> OriginKind {
    match source {
        PhoneTextSource::Typed => OriginKind::Typed,
        PhoneTextSource::Clipboard => OriginKind::Clipboard,
    }
}

impl Runtime {
    // ---------------- desktop ----------------

    /// A trusted phone wants `body` inserted.
    pub(super) fn on_phone_text(&mut self, peer: PublicKey, id: u32, body: String, source: PhoneTextSource) {
        if !self.config.accepts_phone_takes {
            self.take_outbox
                .push((peer, AppMessage::phone_text_status(id, PhoneTextState::failed(PhoneTextFailure::Unavailable, "这台设备不接收手机发来的文字"))));
            return;
        }
        if self.texts.seen.contains(&(peer, id)) {
            return; // the same text on a second path
        }
        if self.texts.seen.len() == SEEN_TEXTS {
            self.texts.seen.pop_front();
        }
        self.texts.seen.push_back((peer, id));
        if self.texts.queue.len() >= MAX_QUEUED_TEXTS {
            self.take_outbox
                .push((peer, AppMessage::phone_text_status(id, PhoneTextState::failed(PhoneTextFailure::Busy, "电脑上排队的文字太多，请稍后再发"))));
            return;
        }
        let name = self.trusted.get_by_key(&peer).map_or_else(|| peer.fingerprint(), |d| d.name);
        tracing::info!(phone = %name, id, chars = body.chars().count(), ?source, "text from the phone");
        self.texts.queue.push_back(IncomingText { peer, id, body, source, name });
        if !self.deliver_next_text() {
            self.take_outbox.push((peer, AppMessage::phone_text_status(id, PhoneTextState::Queued)));
        }
    }

    /// No take is running (a finished one may still be on screen).
    fn take_idle(&self) -> bool {
        matches!(
            self.dictation.status().phase,
            DictationPhase::Idle | DictationPhase::Done { .. } | DictationPhase::Failed { .. } | DictationPhase::Cancelled { .. }
        )
    }

    /// Hand the oldest waiting text to the injector if none is there and no take is running;
    /// whether one went.
    pub(super) fn deliver_next_text(&mut self) -> bool {
        let idle = self.take_idle();
        if self.texts.busy || !idle {
            return false;
        }
        let Some(text) = self.texts.queue.pop_front() else { return false };
        self.texts.busy = true;
        let injector = self.dictation.injector();
        let tx = self.phone_tx.clone();
        tokio::spawn(async move {
            let body = text.body.clone();
            let result = match tokio::task::spawn_blocking(move || injector.inject(&body)).await {
                Ok(result) => result,
                Err(e) => Err(DictationError::Inject(format!("injector task failed: {e}"))),
            };
            let _ = tx.send(PhoneEvent::TextDelivered { text: Box::new(text), result }).await;
        });
        true
    }

    /// The injector is done with a phone's text: tell the phone, keep it in the history, take the
    /// next one.
    pub(super) fn on_text_delivered(&mut self, text: IncomingText, result: Result<Injection, DictationError>) {
        self.texts.busy = false;
        let (state, outcome) = match &result {
            Ok(Injection { via: Via::Paste, .. }) => (PhoneTextState::Delivered { pasted: true }, Outcome::Inserted { via: Via::Paste }),
            Ok(Injection { via: Via::Clipboard, note }) => {
                (PhoneTextState::Delivered { pasted: false }, note.clone().map_or(Outcome::Inserted { via: Via::Clipboard }, Outcome::clipboard))
            }
            Err(e) => (PhoneTextState::failed(PhoneTextFailure::Failed, &e.to_string()), Outcome::Failed { reason: e.to_string() }),
        };
        tracing::info!(phone = %text.name, id = text.id, ?state, "text from the phone handled");
        self.take_outbox.push((text.peer, AppMessage::phone_text_status(text.id, state)));
        let entry = HistoryEntry {
            id: Uuid::new_v4(),
            at_ms: now_ms(),
            raw_text: text.body.clone(),
            text: text.body,
            refined: false,
            asr_model: String::new(),
            refine_model: None,
            duration_ms: 0,
            asr_ms: 0,
            refine_ms: None,
            refine_failure: None,
            outcome,
            starred: false,
            mode: OutputMode::WholeTake,
            segments: None,
            live_error: None,
            vocabulary: None,
            kind: TakeKind::Dictation,
            edit: None,
            app: None,
            scene: None,
            preset: None,
            origin: Some(EntryOrigin { device: text.name, kind: origin_kind(text.source) }),
            processed: None,
        };
        self.record_history(entry);
        self.deliver_next_text();
    }

    /// `PasteText`: a result from the history into the window the user came from
    /// ([`crate::paste`]), on a blocking thread; refused while a take or another text runs. The
    /// injector is the dictation's, so the insert setting applies; nothing goes into the history.
    pub(super) fn paste_text(&mut self, request_id: u64, text: String, target: PasteTarget) {
        let refused = if !valid_paste_text(&text) {
            Some(PasteFailure::Invalid)
        } else if self.texts.busy || !self.take_idle() {
            Some(PasteFailure::Busy)
        } else {
            None
        };
        if let Some(reason) = refused {
            self.emit(CoreEvent::PasteResult { request_id, outcome: PasteOutcome::Failed { reason } });
            return;
        }
        self.texts.busy = true;
        let (injector, probe, tx) = (self.dictation.injector(), self.dictation.probe(), self.phone_tx.clone());
        tokio::spawn(async move {
            let outcome = tokio::task::spawn_blocking(move || paste_blocking(&*injector, probe.as_deref(), &text, target)).await.unwrap_or_else(|e| {
                tracing::warn!(error = %e, "paste task failed");
                PasteOutcome::Failed { reason: PasteFailure::Inject }
            });
            let _ = tx.send(PhoneEvent::Pasted { request_id, outcome }).await;
        });
    }

    /// The paste is done: answer the shell, and let the texts that waited go.
    pub(super) fn on_pasted(&mut self, request_id: u64, outcome: PasteOutcome) {
        self.texts.busy = false;
        tracing::info!(request_id, ?outcome, "paste from the history handled");
        self.emit(CoreEvent::PasteResult { request_id, outcome });
        self.deliver_next_text();
    }

    // ---------------- phone ----------------

    pub(super) fn emit_sent_texts(&mut self) {
        let texts = self.sent_texts.texts().to_vec();
        self.emit(CoreEvent::SentTexts(texts));
    }

    /// `PhoneTextSend`: seal the text for `to` and list it as sending.
    pub(super) fn phone_text_send(&mut self, to: PublicKey, body: String, source: PhoneTextSource) -> Result<(), CoreError> {
        if body.trim().is_empty() {
            return Err(CoreError::Invalid("phone text: 没有要发送的文字".into()));
        }
        if body.chars().count() > MAX_PHONE_TEXT_CHARS {
            return Err(CoreError::Invalid(format!("phone text: 文字太长（最多 {MAX_PHONE_TEXT_CHARS} 字）")));
        }
        let Some(device) = self.trusted.get_by_key(&to) else { return Err(CoreError::Invalid("未知设备".into())) };
        if !self.peer_online(&to) {
            return Err(CoreError::Invalid("设备不在线".into()));
        }
        let id = self.sent_texts.next_id();
        self.take_outbox.push((to, AppMessage::PhoneText { version: ProtocolVersion::CURRENT, id, body: body.clone(), source }));
        self.sent_texts.push(SentText { id, device: to.to_hex(), device_name: device.name, body, source, sent_at: now_ms(), state: SentTextState::Sending });
        self.emit_sent_texts();
        Ok(())
    }

    /// The desktop answered about one of our texts.
    pub(super) fn on_phone_text_status(&mut self, from: PublicKey, id: u32, state: PhoneTextState) {
        if self.sent_texts.update(&from.to_hex(), id, state.into()) {
            self.emit_sent_texts();
        }
    }

    /// Give up on texts nobody answers (every tick).
    pub(super) fn check_texts(&mut self) {
        let now = now_ms();
        let waited = |since: u64, limit: Duration| now.saturating_sub(since) > u64::try_from(limit.as_millis()).unwrap_or(u64::MAX);
        let expired = self.sent_texts.expire(|t| match t.state {
            SentTextState::Sending if waited(t.sent_at, NO_ANSWER_AFTER) => {
                Some(SentTextState::Failed { code: SentTextFailure::NoAnswer, message: "电脑没有回应（两边的版本可能不同）".into() })
            }
            SentTextState::Queued if waited(t.sent_at, QUEUED_GIVE_UP) => {
                Some(SentTextState::Failed { code: SentTextFailure::NoAnswer, message: "电脑一直没有插入这段文字".into() })
            }
            _ => None,
        });
        if expired {
            self.emit_sent_texts();
        }
    }

    /// `SentTextsClear`: forget the list.
    pub(super) fn sent_texts_clear(&mut self) {
        self.sent_texts.clear();
        self.emit_sent_texts();
    }
}
