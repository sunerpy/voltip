//! 复制上一条结果 (docs/dictation.md §28, user decision D12): the tray's entry, the optional global
//! hotkey and `voltip --copy-last` put the newest result on the clipboard through the core
//! ([`Bridge::copy_last`]) without bringing up the main window. When it was copied, the pill says
//! so for a moment between takes ([`crate::overlay::flash_copied`]); with nothing to copy yet, or a
//! clipboard that refused, nothing shows and the log says why.

use std::time::Duration;

use tauri::{AppHandle, Manager as _, Runtime};
use voltip_core::last_result::CopyLastOutcome;
use voltip_tauri_bridge::Bridge;

/// How long the shell waits for the core's answer.
pub const ANSWER_WITHIN: Duration = Duration::from_secs(5);

/// Copy the newest result; `source` names what asked (`tray`, `hotkey`, `cli`) in the log.
pub fn run<R: Runtime>(app: &AppHandle<R>, source: &'static str) {
    let Some(bridge) = app.try_state::<Bridge>().map(|b| b.inner().clone()) else {
        tracing::warn!(source, "copy of the last result before the core is up; dropped");
        return;
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let outcome = bridge.copy_last(ANSWER_WITHIN).await;
        tracing::info!(source, ?outcome, "copy of the last result");
        if outcome == CopyLastOutcome::Copied {
            let state = bridge.state();
            crate::overlay::flash_copied(&app, &state.dictation.phase, state.settings.overlay);
        }
    });
}
