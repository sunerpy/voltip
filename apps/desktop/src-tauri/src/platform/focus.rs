//! The insert check's reader of the focused control for the core (docs/dictation.md §28): this
//! platform's [`voltip_focus::Reader`] (UI Automation, the Accessibility API, AT-SPI) behind the
//! core's [`FocusReader`] port. The readers and their real tests live in `voltip-focus`, so CI can
//! run them on real desktops without building the app.

use std::sync::Arc;
use std::time::Duration;

use voltip_core::ForegroundApp;
use voltip_core::dictation::insert_check::Expected;
use voltip_core::dictation::{FocusReader, FocusWatch, Settled, UnconfirmedWhy};

/// The core's [`FocusReader`] over a [`voltip_focus::Reader`].
pub struct SystemFocusReader(pub voltip_focus::Reader);

impl FocusReader for SystemFocusReader {
    fn watch(&self, app: &ForegroundApp, expected: &Expected) -> Result<Box<dyn FocusWatch>, UnconfirmedWhy> {
        let watch = self.0.watch(&app.app_id, expected)?;
        Ok(Box::new(Watch(watch)))
    }
}

/// One paste's reading, settled by the reader's own polling.
struct Watch(voltip_focus::Watch);

impl FocusWatch for Watch {
    fn settle(self: Box<Self>, within: Duration) -> Settled {
        self.0.settle(within)
    }
}

/// The reader of this build's platform; none where `voltip-focus` has no reader.
pub fn system_reader() -> Option<Arc<dyn FocusReader>> {
    voltip_focus::Reader::system().map(|reader| Arc::new(SystemFocusReader(reader)) as Arc<dyn FocusReader>)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use voltip_core::dictation::InsertCheck;
    use voltip_focus::{FocusPlatform, FocusText, Reader};
    use voltip_platform::HostOs;

    use super::*;

    /// A field that already shows the take after the first reading.
    struct Pasted(u32);

    impl FocusPlatform for Pasted {
        type Control = u32;
        fn focused(&mut self) -> Result<u32, UnconfirmedWhy> {
            Ok(1)
        }
        fn same(&mut self, a: &u32, b: &u32) -> bool {
            a == b
        }
        fn text(&mut self, _control: &u32) -> Option<FocusText> {
            self.0 += 1;
            let text = if self.0 == 1 { "前文" } else { "前文你好" };
            let n = text.chars().count();
            Some(FocusText::read(Some(n), Some(n), Some((n, n)), None, Some(text)))
        }
    }

    fn open() -> Result<Pasted, String> {
        Ok(Pasted(0))
    }

    /// The port hands the id of the application the paste goes to to the reader (a terminal is
    /// skipped) and settles with the reader's verdict.
    #[test]
    fn the_core_port_reads_through_the_platform_reader() {
        let reader = SystemFocusReader(Reader::new(HostOs::Windows, open));
        let app = |id: &str| ForegroundApp { app_id: id.into(), name: id.into(), title: None, window: Some(1) };
        assert_eq!(reader.watch(&app("windowsterminal"), &Expected::of("你好")).err(), Some(UnconfirmedWhy::Skipped));
        let watch = reader.watch(&app("notepad"), &Expected::of("你好")).expect("read");
        assert_eq!(watch.settle(Duration::from_secs(2)), Settled::sure(InsertCheck::Written));
    }
}
