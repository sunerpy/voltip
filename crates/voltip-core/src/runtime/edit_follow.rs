//! 自动跟踪输入框里的修改 (`EngineSettings.follow_edits`, docs/dictation.md §35.10), the runtime's
//! part: after a take was written to a field, its control is kept (the engine hands it over once
//! the take is in the history) and a task reads its paragraph again and again, feeding the follow's
//! state ([`EditFollow`]); what it calls for is written here: the take's correction, with `via:
//! follow`, for the learning suggestions. There is no notice and nothing is added to the
//! dictionary.
//!
//! One follow at a time. It ends when the follow says so (the take left the field, the focus moved,
//! quiet or too long), and here: a new take, a phone's text or a history paste going into a field,
//! the switch, the insert check or the history turned off, the entry deleted or its correction
//! removed (it is not followed again), or a correction the user taught with 从选中文字学习 (never
//! overwritten). Nothing here logs a text.

use std::sync::{Arc, Weak};

use uuid::Uuid;

use super::learn::LearnInbound;
use super::{Runtime, now_ms};
use crate::dictation::{FocusFollow, TakeKind, UnconfirmedWhy};
use crate::history::{CorrectedText, CorrectionVia, HistoryEntry, HistoryText};
use crate::vocabulary::follow::{EditFollow, FollowStep, FollowWrite};

/// The follow under way.
pub(super) struct Active {
    id: Uuid,
    generation: u64,
    task: tokio::task::JoinHandle<()>,
    /// The kept control, released with the follow: the task holds it only while it reads.
    _follow: Arc<dyn FocusFollow>,
}

impl Drop for Active {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The runtime's follows: the one under way, its generation, and the entries not to follow again.
#[derive(Default)]
pub(super) struct EditFollows {
    active: Option<Active>,
    generation: u64,
    /// Entries whose correction the user removed while they were followed (this run).
    blocked: std::collections::HashSet<Uuid>,
}

/// The take's text as the follow compares it: as inserted.
fn take_of(entry: &HistoryEntry) -> HistoryText {
    HistoryText {
        id: entry.id,
        at_ms: entry.at_ms,
        raw_text: entry.raw_text.clone(),
        text: entry.text.clone(),
        refined: entry.refined,
        vocabulary: entry.vocabulary.clone().unwrap_or_default(),
        corrected: None,
        corrected_via: None,
    }
}

impl Runtime {
    /// The follow may run: the switch, the insert check and the history are on.
    fn follow_allowed(&self) -> bool {
        self.settings.engines.follow_edits && self.settings.engines.insert_check && self.settings.history.enabled
    }

    /// `entry` was just put into the history: take the control its paste was written to, if the
    /// engine kept one for this take, and follow it (docs/dictation.md §35.10). What is not
    /// followed is released.
    pub(super) fn start_edit_follow(&mut self, entry: &HistoryEntry) {
        let Some(follow) = self.dictation.take_follow(self.dictation.status().session) else { return };
        let corrected_by_user = entry.corrected.as_ref().is_some_and(|c| c.via == CorrectionVia::Selection);
        if !self.follow_allowed() || entry.kind != TakeKind::Dictation || corrected_by_user || self.follows.blocked.contains(&entry.id) {
            return;
        }
        let Some(state) = EditFollow::new(take_of(entry), self.config.follow_times) else {
            tracing::info!("edit follow: the take is not followed (a line break, or longer than the paragraph read)");
            return;
        };
        self.end_edit_follow("a newer take");
        self.follows.generation += 1;
        let generation = self.follows.generation;
        let follow: Arc<dyn FocusFollow> = Arc::from(follow);
        let task = tokio::spawn(run(state, Arc::downgrade(&follow), generation, self.learn.tx()));
        self.follows.active = Some(Active { id: entry.id, generation, task, _follow: follow });
        tracing::info!("edit follow: started");
    }

    /// End the follow under way, if any, and release its control.
    pub(super) fn end_edit_follow(&mut self, why: &str) {
        if self.follows.active.take().is_some() {
            tracing::info!(why, "edit follow: ended");
        }
    }

    /// Before a text goes into a field without a take (a phone's text, a history paste): it is not
    /// the user's edit of the followed take, and the control kept for the last take is not wanted.
    pub(super) fn end_edit_follow_for_paste(&mut self) {
        self.dictation.drop_follow();
        self.end_edit_follow("a text pasted without a take");
    }

    /// The settings changed: end the follow when it may no longer run.
    pub(super) fn check_edit_follow(&mut self) {
        if !self.follow_allowed() {
            self.end_edit_follow("switched off");
        }
    }

    /// `ids` left the history (deleted, cleared): their follow ends.
    pub(super) fn edit_follow_entries_gone(&mut self, ids: Option<&[Uuid]>) {
        let followed = self.follows.active.as_ref().map(|a| a.id);
        if followed.is_some_and(|id| ids.is_none_or(|ids| ids.contains(&id))) {
            self.end_edit_follow("the entry is gone");
        }
    }

    /// The user removed `id`'s correction (历史详情 › 移除): its follow ends and it is not followed
    /// again; or they taught it with 从选中文字学习, which the follow never overwrites.
    pub(super) fn edit_follow_corrected_by_user(&mut self, id: Uuid, removed: bool) {
        if removed {
            self.follows.blocked.insert(id);
        }
        if self.follows.active.as_ref().is_some_and(|a| a.id == id) {
            self.end_edit_follow(if removed { "the correction was removed" } else { "the user taught the take" });
        }
    }

    /// A step of the follow under way: write first, then end.
    pub(super) fn on_followed(&mut self, generation: u64, id: Uuid, step: FollowStep, reads: u32) {
        if self.follows.active.as_ref().is_none_or(|a| a.generation != generation || a.id != id) {
            return;
        }
        if let Some(write) = step.write {
            match self.write_followed(id, write) {
                Ok(true) => self.emit_history(),
                Ok(false) => {}
                Err(stop) => {
                    self.end_edit_follow(stop);
                    return;
                }
            }
        }
        if let Some(end) = step.end {
            tracing::info!(reads, why = end.name(), "edit follow: ended");
            self.follows.active = None;
        }
    }

    /// Write `write` into `id`'s entry, unless the user's own correction is there; whether the entry
    /// changed, or why the follow must stop.
    fn write_followed(&mut self, id: Uuid, write: FollowWrite) -> Result<bool, &'static str> {
        if !self.follow_allowed() {
            return Err("switched off");
        }
        let entry = match self.history.get(id) {
            Ok(Some(entry)) => entry,
            Ok(None) => return Err("the entry is gone"),
            Err(e) => {
                tracing::warn!(error = %e, "edit follow: the entry could not be read");
                return Err("the history could not be read");
            }
        };
        let current = entry.corrected.as_ref().map(|c| c.via);
        if current == Some(CorrectionVia::Selection) {
            return Err("the user taught the take");
        }
        let result = match write {
            FollowWrite::Store(text) => {
                if entry.corrected.as_ref().is_some_and(|c| c.text == text) {
                    return Ok(false);
                }
                self.history.set_corrected(id, Some(CorrectedText { text, at_ms: now_ms(), via: CorrectionVia::Follow }))
            }
            FollowWrite::Clear if current == Some(CorrectionVia::Follow) => self.history.set_corrected(id, None),
            FollowWrite::Clear => return Ok(false),
        };
        result.map_err(|e| {
            tracing::warn!(error = %e, "edit follow: the correction could not be stored");
            "the history could not be written"
        })
    }
}

/// The follow's task: read the kept control, step the follow, send what it calls for, wait, again;
/// until it ends or the runtime drops it. The control is the runtime's: once the follow ends there
/// it is released at once, unless a reading is under way (then when that reading returns).
async fn run(mut state: EditFollow, follow: Weak<dyn FocusFollow>, generation: u64, tx: tokio::sync::mpsc::Sender<LearnInbound>) {
    let started = tokio::time::Instant::now();
    loop {
        let Some(reader) = follow.upgrade() else { return };
        let reading = tokio::task::spawn_blocking(move || reader.context(None)).await.unwrap_or(Err(UnconfirmedWhy::Unreadable));
        let step = state.on_reading(started.elapsed(), reading);
        let end = step.end.is_some();
        if step != FollowStep::default() && tx.send(LearnInbound::Followed { generation, id: state.id(), step, reads: state.reads() }).await.is_err() {
            return;
        }
        if end {
            return;
        }
        tokio::time::sleep(state.next_in(started.elapsed())).await;
    }
}
