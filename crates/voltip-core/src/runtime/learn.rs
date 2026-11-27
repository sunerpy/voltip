//! 从选中文字学习 (`CoreCommand::LearnSelection`, docs/dictation.md §35): the user corrected a take
//! in the application it went to, selected the corrected text (or left the caret in it) and pressed
//! the shortcut (or ran `voltip --learn-selection`). On a task: the probe names the application in
//! front (a terminal's selection is program output: no key is pressed, nothing is read), the copy
//! chord reads the selection (§19.2, the clipboard put back afterwards), the focused control's
//! reader reads the paragraph around the selection or the caret (`FocusReader::context`, at most
//! [`crate::dictation::CONTEXT_CHARS`] characters a side), and the newest [`LEARN_TAKES`] recognised
//! takes are compared with that paragraph, the take at the selection kept, or with the selection
//! alone when nothing was read (`crate::vocabulary::learn`). Back here the corrections are stored with their entries; then
//! the learning suggestions are counted over the history once more for the pairs the corrected
//! takes give, and the notice ([`LearnNotice`], `UiState.learn_notice`) offers them on the pill.
//!
//! The notice is shown while no take uses the pill: one that comes while a take's result is still
//! on it waits, without a timer, until the pill is idle again; then it stays
//! [`LEARN_NOTICE_DWELL`], again after each of its buttons; a new take clears it. With the pill off
//! there is no notice: the outcome is logged and the pairs wait in the learning suggestions.
//! Nothing here logs a text.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use uuid::Uuid;

use super::{CoreEvent, Runtime, now_ms};
use crate::dictation::activation::EdgeSource;
use crate::dictation::{DictationPhase, ForegroundApp};
use crate::history::{CorrectedText, CorrectionVia, HistoryReader, MAX_RECENT_TEXTS};
use crate::settings::OverlayPlacement;
use crate::vocabulary::learn::{LEARN_TAKES, MAX_LEARN_SELECTION_CHARS, match_around, match_selection, storable};
use crate::vocabulary::{BatchItem, BatchUndoOutcome, SuggestionPick, edit_picks};

/// How long the notice stays on the pill, counted from when it shows and again after each button.
pub const LEARN_NOTICE_DWELL: Duration = Duration::from_secs(8);

/// What came of one 从选中文字学习 (docs/dictation.md §35).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LearnOutcome {
    /// The corrections give these pairs; `safe`: none has a short heard form (one character, or one
    /// or two Latin letters), so the notice may offer 加入词典 at once.
    Learned {
        /// The pairs, in the order the takes come.
        pairs: Vec<SuggestionPick>,
        /// No pair is short enough to change other words.
        safe: bool,
    },
    /// The selection corrects takes, but no change is one to put in the dictionary (only
    /// punctuation, new or deleted words, what the clean-up, the dictionary or a rule wrote, or a
    /// pair known or ignored already).
    NoPairs,
    /// The selection is what was inserted (or what it was put back to).
    Unchanged,
    /// None of the newest takes is like the selection, or like the paragraph read around it.
    NoMatch {
        /// The paragraph around the selection (or the caret) was read from the focused control; when
        /// it was not, the notice asks for the unchanged words around the change to be selected too.
        #[serde(default)]
        context: bool,
    },
    /// Nothing was selected (or the application did not copy).
    NoSelection,
    /// The selection is longer than `max` characters.
    TooLong {
        /// [`MAX_LEARN_SELECTION_CHARS`].
        max: u32,
    },
    /// A terminal is in front: its selection is program output, so nothing was copied.
    Terminal,
    /// The history is off: there is nothing to compare with, and nothing would be kept.
    HistoryOff,
    /// The selection could not be read, or the history could not be read or written.
    Failed {
        /// Why (no text of the user's).
        message: String,
    },
}

/// What 加入词典 on the notice did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoticeAccepted {
    /// Pairs that went into the dictionary.
    pub added: u32,
    /// Pairs the dictionary or a rule refused (the dictionary page says why).
    pub refused: u32,
    /// The batch 撤销 takes back, when anything went in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo: Option<Uuid>,
}

/// The pill's notice after a 从选中文字学习 (`UiState.learn_notice`, docs/dictation.md §35).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnNotice {
    /// Which notice: the buttons name it, and a newer one replaces it.
    pub id: u64,
    /// What came of it.
    pub outcome: LearnOutcome,
    /// On the pill now; a notice that waits behind a take's result is not (yet).
    pub shown: bool,
    /// 加入词典 ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted: Option<NoticeAccepted>,
    /// 撤销 ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undone: Option<BatchUndoOutcome>,
}

/// A heard form short enough to change other words (the dictionary page's warning, `shortForm`
/// in `packages/shared/src/quick-fix.ts`): one character, or one or two Latin letters.
fn short_form(heard: &str) -> bool {
    let form = heard.trim();
    let n = form.chars().count();
    n == 1 || (n <= 2 && !form.is_empty() && form.chars().all(|c| c.is_ascii_alphabetic()))
}

/// One take a selection corrects, as the task found it.
#[derive(Debug)]
pub(super) struct Correction {
    id: Uuid,
    /// The whole corrected text to keep, or `None` to clear an earlier one.
    keep: Option<String>,
    /// The take had a correction before.
    had: bool,
    /// The selection is what was inserted.
    unchanged: bool,
}

/// What the task found: the takes the selection corrects, and whether the paragraph around it was
/// read (the 没有找到 notice says so).
#[derive(Debug)]
pub(super) struct Compared {
    corrections: Vec<Correction>,
    context: bool,
}

/// What the tasks send back.
#[derive(Debug)]
pub(super) enum LearnInbound {
    /// The selection was read and compared, or the attempt ended early.
    Compared { request: u64, result: Result<Compared, LearnOutcome> },
    /// The pairs of the corrected takes are counted.
    Counted { request: u64, picks: Result<Vec<SuggestionPick>, String> },
    /// A notice's time is up.
    Expired { id: u64, generation: u64 },
    /// A step of 自动跟踪输入框里的修改 (docs/dictation.md §35.10, `super::edit_follow`).
    Followed { generation: u64, id: Uuid, step: crate::vocabulary::follow::FollowStep, reads: u32 },
}

/// The runtime's part of 从选中文字学习.
pub(super) struct LearnRuntime {
    tx: mpsc::Sender<LearnInbound>,
    reader: Arc<HistoryReader>,
    /// The newest request; answers to older ones are dropped.
    request: u64,
    notice: Option<LearnNotice>,
    next_id: u64,
    /// Restarts of the notice's timer; an older timer's expiry is dropped.
    generation: u64,
}

impl LearnRuntime {
    pub(super) fn new(data_dir: &std::path::Path) -> (Self, mpsc::Receiver<LearnInbound>) {
        let (tx, rx) = mpsc::channel(8);
        (Self { tx, reader: Arc::new(HistoryReader::new(data_dir)), request: 0, notice: None, next_id: 0, generation: 0 }, rx)
    }

    /// Where the tasks answer.
    pub(super) fn tx(&self) -> mpsc::Sender<LearnInbound> {
        self.tx.clone()
    }
}

/// A take is under way: not idle and not yet finished.
fn take_running(phase: &DictationPhase) -> bool {
    !matches!(phase, DictationPhase::Idle) && !phase.is_terminal()
}

impl Runtime {
    /// `LearnSelection`: read the selection and learn from it, unless a take is under way (the key
    /// belongs to it then) or the history is off.
    pub(super) fn learn_selection(&mut self, source: EdgeSource) {
        if take_running(&self.dictation.status().phase) {
            tracing::info!("learn selection ignored: a take is under way");
            return;
        }
        self.learn.request += 1;
        let request = self.learn.request;
        if !self.settings.history.enabled {
            tracing::info!(request, "learn selection: the history is off");
            self.show_learned(LearnOutcome::HistoryOff);
            return;
        }
        // The user may still hold the shortcut's modifiers when the copy chord goes out (§19.2).
        let held = match source {
            EdgeSource::Cli => Vec::new(),
            EdgeSource::Hotkey | EdgeSource::Ui => {
                self.settings.learn_hotkey.as_deref().and_then(|h| crate::Hotkey::parse(h).ok()).map(|h| h.modifiers).unwrap_or_default()
            }
        };
        let (probe, injector, reader, tx) = (self.dictation.probe(), self.dictation.injector(), self.learn.reader.clone(), self.learn.tx.clone());
        tokio::spawn(async move {
            let result = read_and_compare(probe, injector, reader, held).await;
            let _ = tx.send(LearnInbound::Compared { request, result }).await;
        });
    }

    /// An answer from a task, or a notice's timer.
    pub(super) fn on_learn(&mut self, inbound: LearnInbound) {
        match inbound {
            LearnInbound::Compared { request, result } if request == self.learn.request => match result {
                Err(outcome) => self.show_learned(outcome),
                Ok(compared) => self.keep_corrections(request, compared),
            },
            LearnInbound::Counted { request, picks } if request == self.learn.request => match picks {
                Ok(picks) if picks.is_empty() => self.show_learned(LearnOutcome::NoPairs),
                Ok(pairs) => {
                    let safe = !pairs.iter().any(|p| short_form(&p.heard));
                    self.show_learned(LearnOutcome::Learned { pairs, safe });
                }
                Err(message) => self.show_learned(LearnOutcome::Failed { message }),
            },
            LearnInbound::Expired { id, generation } => {
                if self.learn.notice.as_ref().is_some_and(|n| n.id == id) && generation == self.learn.generation {
                    self.set_learn_notice(None);
                }
            }
            LearnInbound::Followed { generation, id, step, reads } => self.on_followed(generation, id, step, reads),
            LearnInbound::Compared { .. } | LearnInbound::Counted { .. } => tracing::debug!("an older learn selection answered; dropped"),
        }
    }

    /// Store what the selection corrects, then count the pairs those takes give.
    fn keep_corrections(&mut self, request: u64, compared: Compared) {
        let Compared { corrections, context } = compared;
        if corrections.is_empty() {
            self.show_learned(LearnOutcome::NoMatch { context });
            return;
        }
        let mut written = false;
        let mut kept: Vec<Uuid> = Vec::new();
        for c in &corrections {
            let result = match &c.keep {
                Some(text) => {
                    kept.push(c.id);
                    // docs/dictation.md §35.10: the user's own correction; a follow of it stops.
                    self.edit_follow_corrected_by_user(c.id, false);
                    self.history.set_corrected(c.id, Some(CorrectedText { text: text.clone(), at_ms: now_ms(), via: CorrectionVia::Selection }))
                }
                None if c.had => self.history.set_corrected(c.id, None),
                None => Ok(false),
            };
            match result {
                Ok(changed) => written |= changed,
                Err(e) => {
                    tracing::warn!(error = %e, "learn selection: the correction could not be stored");
                    self.show_learned(LearnOutcome::Failed { message: e.to_string() });
                    return;
                }
            }
        }
        if written {
            self.emit_history();
        }
        tracing::info!(request, takes = corrections.len(), kept = kept.len(), "learn selection: compared");
        if kept.is_empty() {
            let unchanged = corrections.iter().all(|c| c.unchanged);
            self.show_learned(if unchanged { LearnOutcome::Unchanged } else { LearnOutcome::NoPairs });
            return;
        }
        let (dictionary, rules, ignored) = (self.dictionary.entries().to_vec(), self.rules.rules().to_vec(), self.ignored.list().to_vec());
        let (reader, tx) = (self.learn.reader.clone(), self.learn.tx.clone());
        tokio::spawn(async move {
            let picks = tokio::task::spawn_blocking(move || {
                let history = reader.recent_texts(MAX_RECENT_TEXTS).map_err(|e| e.to_string())?;
                Ok(edit_picks(&dictionary, &rules, &history, &ignored, &kept))
            })
            .await
            .unwrap_or_else(|e| Err(format!("learn task failed: {e}")));
            let _ = tx.send(LearnInbound::Counted { request, picks }).await;
        });
    }

    /// Put `outcome` on the pill as a new notice (none with the pill off), waiting while a take's
    /// result is still there.
    fn show_learned(&mut self, outcome: LearnOutcome) {
        tracing::info!(outcome = learned_name(&outcome), "learn selection");
        // No pill to show it on, or a take began meanwhile and has the pill: the pairs wait in the
        // learning suggestions.
        if self.settings.overlay == OverlayPlacement::Off || take_running(&self.dictation.status().phase) {
            return;
        }
        self.learn.next_id += 1;
        let shown = matches!(self.dictation.status().phase, DictationPhase::Idle);
        self.set_learn_notice(Some(LearnNotice { id: self.learn.next_id, outcome, shown, accepted: None, undone: None }));
    }

    /// Replace the notice, restart its timer when it is on the pill, and tell the UI.
    fn set_learn_notice(&mut self, notice: Option<LearnNotice>) {
        self.learn.generation += 1;
        if let Some(n) = notice.as_ref().filter(|n| n.shown) {
            let (id, generation, tx, dwell) = (n.id, self.learn.generation, self.learn.tx.clone(), self.config.learn_notice_dwell);
            tokio::spawn(async move {
                tokio::time::sleep(dwell).await;
                let _ = tx.send(LearnInbound::Expired { id, generation }).await;
            });
        }
        self.learn.notice = notice;
        self.emit(CoreEvent::LearnNotice(self.learn.notice.clone()));
    }

    /// The dictation phase moved (docs/dictation.md §35): a take that starts clears the notice; a
    /// pill that is idle again shows a waiting one.
    pub(super) fn follow_learn_notice(&mut self, phase: &DictationPhase) {
        let Some(notice) = self.learn.notice.clone() else { return };
        if take_running(phase) {
            self.set_learn_notice(None);
        } else if !notice.shown && matches!(phase, DictationPhase::Idle) {
            self.set_learn_notice(Some(LearnNotice { shown: true, ..notice }));
        }
    }

    /// The notice's 加入词典: its pairs through the batch accept of the learning suggestions.
    pub(super) fn learn_notice_accept(&mut self, id: u64) {
        let Some(notice) = self.learn.notice.clone().filter(|n| n.id == id && n.accepted.is_none()) else { return };
        let LearnOutcome::Learned { pairs, safe: true } = &notice.outcome else {
            tracing::info!(id, "learn notice: nothing to add at once");
            return;
        };
        let outcome = self.accept_suggestions(pairs);
        let added = outcome.items.iter().filter(|i| matches!(i, BatchItem::Accepted { .. })).count();
        let accepted = NoticeAccepted {
            added: u32::try_from(added).unwrap_or(u32::MAX),
            refused: u32::try_from(outcome.items.len() - added).unwrap_or(u32::MAX),
            undo: outcome.undo,
        };
        self.set_learn_notice(Some(LearnNotice { accepted: Some(accepted), ..notice }));
    }

    /// The notice's 撤销: the batch its 加入词典 made, taken back.
    pub(super) fn learn_notice_undo(&mut self, id: u64) {
        let Some(notice) = self.learn.notice.clone().filter(|n| n.id == id && n.undone.is_none()) else { return };
        let Some(batch) = notice.accepted.as_ref().and_then(|a| a.undo) else { return };
        let outcome = self.undo_suggestions(batch);
        self.set_learn_notice(Some(LearnNotice { undone: Some(outcome), ..notice }));
    }

    /// The notice's 查看 or 打开设置: shown, it stays its whole time again (D3).
    pub(super) fn learn_notice_keep(&mut self, id: u64) {
        if let Some(notice) = self.learn.notice.clone().filter(|n| n.id == id && n.shown) {
            self.set_learn_notice(Some(notice));
        }
    }

    /// The notice's ×.
    pub(super) fn learn_notice_dismiss(&mut self, id: u64) {
        if self.learn.notice.as_ref().is_some_and(|n| n.id == id) {
            self.set_learn_notice(None);
        }
    }

    /// The history detail's 移除: forget the user's correction of entry `id`.
    pub(super) fn clear_correction(&mut self, id: Uuid) -> Result<(), crate::CoreError> {
        // docs/dictation.md §35.10: a follow of it stops, and it is not followed again.
        self.edit_follow_corrected_by_user(id, true);
        if self.history.set_corrected(id, None)? {
            self.emit_history();
        }
        Ok(())
    }
}

/// The outcome's name for the log.
fn learned_name(outcome: &LearnOutcome) -> &'static str {
    match outcome {
        LearnOutcome::Learned { .. } => "learned",
        LearnOutcome::NoPairs => "no_pairs",
        LearnOutcome::Unchanged => "unchanged",
        LearnOutcome::NoMatch { .. } => "no_match",
        LearnOutcome::NoSelection => "no_selection",
        LearnOutcome::TooLong { .. } => "too_long",
        LearnOutcome::Terminal => "terminal",
        LearnOutcome::HistoryOff => "history_off",
        LearnOutcome::Failed { .. } => "failed",
    }
}

/// The task: the application in front, the selection, the paragraph around it, the takes they
/// correct.
async fn read_and_compare(
    probe: Option<Arc<dyn crate::dictation::ForegroundProbe>>,
    injector: Arc<dyn crate::dictation::Injector>,
    reader: Arc<HistoryReader>,
    held: Vec<crate::hotkey::Modifier>,
) -> Result<Compared, LearnOutcome> {
    // §19.2: no key is pressed before the probe said which application is in front; a terminal's
    // selection is program output. No answer (pure Wayland, a slow probe): copied anyway, but
    // nothing is read around it (§28.2: a field whose application nobody named is not read).
    let mut front = None;
    if let Some(probe) = probe {
        let deadline = probe.deadline();
        let app = match tokio::time::timeout(deadline, tokio::task::spawn_blocking(move || probe.foreground())).await {
            Ok(Ok(Ok(app))) => app.and_then(ForegroundApp::sanitized),
            _ => None,
        };
        if let Some(app) = &app
            && injector.holds_program_output(&app.app_id)
        {
            tracing::info!(app = %app.app_id, "learn selection refused: a terminal is in front");
            return Err(LearnOutcome::Terminal);
        }
        front = app;
    }
    let copier = injector.clone();
    let copied = match tokio::task::spawn_blocking(move || copier.copy_selection(&held)).await {
        Ok(Ok(text)) => text.filter(|t| !t.trim().is_empty()),
        Ok(Err(e)) => return Err(LearnOutcome::Failed { message: e.to_string() }),
        Err(e) => return Err(LearnOutcome::Failed { message: format!("copy task failed: {e}") }),
    };
    if copied.as_ref().is_some_and(|t| t.chars().count() > MAX_LEARN_SELECTION_CHARS) {
        return Err(LearnOutcome::TooLong { max: u32::try_from(MAX_LEARN_SELECTION_CHARS).unwrap_or(u32::MAX) });
    }
    // §35: the paragraph around the selection, or the caret, with the copied selection placing it
    // in a control that tells no caret. In memory only; nothing of it is logged.
    let context = match (front, injector.focus_reader()) {
        (Some(app), Some(focus)) => {
            let hint = copied.clone();
            match tokio::task::spawn_blocking(move || focus.context(&app, hint.as_deref())).await {
                Ok(Ok(context)) if !context.is_empty() => Some(context),
                Ok(Ok(_)) => None,
                Ok(Err(why)) => {
                    tracing::debug!(?why, "learn selection: nothing read around the selection");
                    None
                }
                Err(e) => {
                    tracing::debug!(error = %e, "learn selection: the read task failed");
                    None
                }
            }
        }
        _ => None,
    };
    if copied.is_none() && context.is_none() {
        return Err(LearnOutcome::NoSelection);
    }
    tracing::debug!(
        copied = copied.as_ref().map(|t| t.chars().count()),
        context = context.as_ref().map(|c| c.before.chars().count() + c.selected.chars().count() + c.after.chars().count()),
        "learn selection: read"
    );
    tokio::task::spawn_blocking(move || {
        let takes = reader.recent_texts(LEARN_TAKES).map_err(|e| LearnOutcome::Failed { message: e.to_string() })?;
        let around = context.as_ref().map(|c| match_around(&c.before, &c.selected, &c.after, &takes)).unwrap_or_default();
        // A paragraph that holds no take at the selection (a stale or unusual reading): the
        // selection alone, as before the context was read.
        let found = match (around.is_empty(), copied.as_deref()) {
            (true, Some(selection)) => match_selection(selection, &takes),
            _ => around,
        };
        let corrections = found
            .into_iter()
            .map(|m| {
                let take = &takes[m.index];
                let keep = storable(take, &m.corrected).then_some(m.corrected.clone());
                Correction { id: take.id, keep, had: take.corrected.is_some(), unchanged: m.corrected == take.text }
            })
            .collect();
        Ok(Compared { corrections, context: context.is_some() })
    })
    .await
    .unwrap_or_else(|e| Err(LearnOutcome::Failed { message: format!("learn task failed: {e}") }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_forms_are_the_dictionary_pages() {
        for short in ["非", "a", "ab", " AB "] {
            assert!(short_form(short), "{short}");
        }
        for long in ["非书", "abc", "a1", "Quinn for"] {
            assert!(!short_form(long), "{long}");
        }
    }
}
