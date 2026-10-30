//! 复制上一条结果 (docs/dictation.md §28, user decision D12): the runtime keeps the text of the
//! newest result in memory — a take's `Done.text`, or a failure's `Failed.text`, voice edits, a
//! recording recognised again and a paired phone's takes included, a phone's typed text not —
//! whatever the history settings say, and forgets it when the app quits. The tray's entry and the
//! optional hotkey put it on the clipboard through [`crate::CoreCommand::CopyLastResult`].

use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::dictation::DictationPhase;

/// What a [`crate::CoreCommand::CopyLastResult`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CopyLastOutcome {
    /// The newest result is on the clipboard.
    Copied,
    /// There is no result yet (none since the app started).
    Nothing,
    /// The clipboard could not be written.
    Failed,
}

/// Where the answer to a [`crate::CoreCommand::CopyLastResult`] goes.
#[derive(Clone)]
pub struct CopyLastReply(Arc<Mutex<Option<tokio::sync::oneshot::Sender<CopyLastOutcome>>>>);

impl CopyLastReply {
    /// The reply to hand the command, and the receiver the caller waits on.
    pub fn channel() -> (Self, tokio::sync::oneshot::Receiver<CopyLastOutcome>) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        (Self(Arc::new(Mutex::new(Some(tx)))), rx)
    }

    /// Answer; a second answer, or one nobody waits for any more, goes nowhere.
    pub fn send(&self, outcome: CopyLastOutcome) {
        if let Some(tx) = self.0.lock().take() {
            let _ = tx.send(outcome);
        }
    }
}

impl std::fmt::Debug for CopyLastReply {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CopyLastReply")
    }
}

/// The text a terminal phase leaves for 复制上一条结果, if any: the delivered text of `Done`, the
/// recoverable text of `Failed`.
pub fn result_text(phase: &DictationPhase) -> Option<&str> {
    match phase {
        DictationPhase::Done { text, .. } => Some(text.as_str()),
        DictationPhase::Failed { text: Some(text), .. } => Some(text.as_str()),
        _ => None,
    }
    .filter(|text| !text.trim().is_empty())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::dictation::FailureCode;

    #[test]
    fn only_a_result_with_text_counts_and_the_reply_answers_once() {
        let failed = |text: Option<&str>| DictationPhase::Failed { code: FailureCode::Inject, message: "inject: x".into(), text: text.map(str::to_owned) };
        assert_eq!(result_text(&failed(Some("留下的"))), Some("留下的"));
        assert_eq!(result_text(&failed(None)), None);
        assert_eq!(result_text(&failed(Some("  "))), None, "nothing to copy");
        assert_eq!(result_text(&DictationPhase::Idle), None);
        assert_eq!(result_text(&DictationPhase::CANCELLED), None);
        assert_eq!(serde_json::to_string(&CopyLastOutcome::Nothing).unwrap(), r#""nothing""#);
        let (reply, mut rx) = CopyLastReply::channel();
        reply.send(CopyLastOutcome::Copied);
        reply.send(CopyLastOutcome::Failed);
        assert_eq!(rx.try_recv().unwrap(), CopyLastOutcome::Copied, "the first answer only");
        assert_eq!(format!("{reply:?}"), "CopyLastReply");
    }
}
