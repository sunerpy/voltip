//! 用 AI 预设处理 (`CoreCommand::HistoryProcess`, docs/dictation.md §22): an entry's text through a
//! preset in parts ([`crate::history::process`]) on a task, with the language and the dictionary's
//! terms of the current engines; progress goes straight to the UI, the result comes back here to be
//! stored with the entry. A cancel aborts the task and stores nothing.

use uuid::Uuid;

use super::{CoreEvent, Runtime, now_ms};
use crate::dictation::RefineHints;
use crate::history::ProcessedText;
use crate::history::process::{self, PROCESS_ENTRY_GONE, PROCESS_UNCONFIGURED, ProcessState};
use crate::presets::{PresetId, PresetRef, resolve};
use crate::vocabulary::Vocabulary;

/// What a processing task ends with.
#[derive(Debug)]
pub(super) struct Processed {
    request_id: u64,
    id: Uuid,
    preset: PresetRef,
    result: Result<String, String>,
}

impl Runtime {
    /// Start processing entry `id` with `preset`; the answers are [`CoreEvent::HistoryProcess`].
    pub(super) fn process_entry(&mut self, request_id: u64, id: Uuid, preset: PresetId) {
        let failed = |reason: String| CoreEvent::HistoryProcess { request_id, id, state: ProcessState::Failed { reason } };
        let Some(refiner) = self.dictation.refiner() else {
            self.emit(failed(PROCESS_UNCONFIGURED.to_owned()));
            return;
        };
        let text = match self.history.get(id) {
            Ok(Some(entry)) => entry.text,
            Ok(None) => return self.emit(failed(PROCESS_ENTRY_GONE.to_owned())),
            Err(e) => return self.emit(failed(e.to_string())),
        };
        let (preset, missing) = resolve(preset, self.presets.presets());
        if missing {
            tracing::info!(request_id, "the chosen custom preset no longer exists; processing with 校对");
        }
        let vocabulary = Vocabulary::compile(self.dictionary.entries(), self.rules.rules());
        let hints = RefineHints {
            preset: preset.clone(),
            language: self.resolved_engines().language.clone(),
            glossary: vocabulary.glossary().to_vec(),
            ..RefineHints::default()
        };
        let (evt, done) = (self.evt.clone(), self.process_tx.clone());
        tracing::info!(request_id, chars = text.chars().count(), "processing a history entry with a preset");
        let task = tokio::spawn(async move {
            let progress = |done: u32, total: u32| {
                // Progress may be dropped when the UI lags; the end never is.
                let _ = evt.try_send(CoreEvent::HistoryProcess { request_id, id, state: ProcessState::Running { done, total } });
            };
            let result = process::run(refiner.as_ref(), &hints, &text, progress).await.map_err(|e| e.to_string());
            let _ = done.send(Processed { request_id, id, preset: preset.to_ref(), result }).await;
        });
        if let Some((_, earlier)) = self.processing.insert(request_id, (id, task)) {
            earlier.abort();
        }
    }

    /// Stop a running request; nothing is stored.
    pub(super) fn cancel_processing(&mut self, request_id: u64) {
        let Some((id, task)) = self.processing.remove(&request_id) else { return };
        task.abort();
        tracing::info!(request_id, "history processing cancelled");
        self.emit(CoreEvent::HistoryProcess { request_id, id, state: ProcessState::Cancelled });
    }

    /// A task finished: store its text with the entry and say so (a cancelled request is ignored).
    pub(super) fn on_processed(&mut self, processed: Processed) {
        let Processed { request_id, id, preset, result } = processed;
        if self.processing.remove(&request_id).is_none() {
            return;
        }
        let state = match result {
            Ok(text) => {
                let processed = ProcessedText { text, preset, at_ms: now_ms() };
                match self.history.set_processed(id, processed.clone()) {
                    Ok(true) => {
                        self.emit_history();
                        ProcessState::Done { processed }
                    }
                    Ok(false) => ProcessState::Failed { reason: PROCESS_ENTRY_GONE.to_owned() },
                    Err(e) => ProcessState::Failed { reason: e.to_string() },
                }
            }
            Err(reason) => ProcessState::Failed { reason },
        };
        self.emit(CoreEvent::HistoryProcess { request_id, id, state });
    }
}
