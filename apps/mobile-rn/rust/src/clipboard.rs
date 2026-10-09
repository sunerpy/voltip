//! Where a take the phone recognised itself goes (docs/dictation.md §20.7): onto the phone's
//! clipboard, the one place it can, whatever `EngineSettings.inject` says. Voice edit stays the
//! computer's: the default `copy_selection` refuses. The core calls the injector on a blocking
//! thread, which is what the host's clipboard call needs (it waits for Android's main thread).

use std::sync::Arc;

use voltip_core::dictation::{DictationError, Injection, Injector, Via};

use crate::host::Host;

/// The phone's clipboard as the core's injector.
pub struct ClipboardInjector(pub Arc<dyn Host>);

impl Injector for ClipboardInjector {
    fn inject(&self, text: &str) -> Result<Injection, DictationError> {
        self.0.clipboard_write(text).map_err(DictationError::Inject)?;
        tracing::info!(chars = text.chars().count(), "result copied to the phone clipboard");
        Ok(Injection { via: Via::Clipboard, note: None })
    }

    fn copy(&self, text: &str) -> Result<(), DictationError> {
        self.0.clipboard_write(text).map_err(DictationError::Inject)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::host::{HostCall, RecordingHost};

    #[test]
    fn a_result_goes_onto_the_clipboard_and_voice_edit_is_refused() {
        let host = Arc::new(RecordingHost::default());
        let injector = ClipboardInjector(host.clone());
        let injection = injector.inject("你好").unwrap();
        assert_eq!(injection.via, Via::Clipboard);
        injector.copy("再见").unwrap();
        assert_eq!(host.calls(), [HostCall::ClipboardWrite("你好".into()), HostCall::ClipboardWrite("再见".into())]);
        assert_eq!(host.clipboard().as_deref(), Some("再见"));
        assert!(matches!(injector.copy_selection(&[]), Err(DictationError::EditUnavailable(_))));
    }

    /// A clipboard that refuses is the core's `Inject` error with the platform's reason, never a
    /// claimed copy.
    #[test]
    fn a_refusing_clipboard_reports_its_reason() {
        let host = Arc::new(RecordingHost::refusing("clipboard: busy"));
        let injector = ClipboardInjector(host);
        assert!(matches!(injector.inject("你好"), Err(DictationError::Inject(m)) if m == "clipboard: busy"));
        assert!(matches!(injector.copy("你好"), Err(DictationError::Inject(m)) if m == "clipboard: busy"));
    }
}
