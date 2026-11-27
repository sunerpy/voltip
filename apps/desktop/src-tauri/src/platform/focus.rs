//! The insert check's reader of the focused control for the core (docs/dictation.md §28): this
//! platform's [`voltip_focus::Reader`] (UI Automation, the Accessibility API, AT-SPI) behind the
//! core's [`FocusReader`] port. The readers and their real tests live in `voltip-focus`, so CI can
//! run them on real desktops without building the app.

use std::sync::Arc;
use std::time::Duration;

use voltip_core::ForegroundApp;
use voltip_core::dictation::insert_check::Expected;
use voltip_core::dictation::{FocusContext, FocusFollow, FocusReader, FocusWatch, Settled, UnconfirmedWhy};

/// The core's [`FocusReader`] over a [`voltip_focus::Reader`].
pub struct SystemFocusReader(pub voltip_focus::Reader);

impl FocusReader for SystemFocusReader {
    fn watch(&self, app: &ForegroundApp, expected: &Expected) -> Result<Box<dyn FocusWatch>, UnconfirmedWhy> {
        let watch = self.0.watch(&app.app_id, expected)?;
        Ok(Box::new(Watch(watch)))
    }

    fn context(&self, app: &ForegroundApp, hint: Option<&str>) -> Result<FocusContext, UnconfirmedWhy> {
        self.0.context(&app.app_id, hint)
    }
}

/// One paste's reading, settled by the reader's own polling.
struct Watch(voltip_focus::Watch);

impl FocusWatch for Watch {
    fn settle(self: Box<Self>, within: Duration) -> Settled {
        self.0.settle(within)
    }

    fn settle_and_keep(self: Box<Self>, within: Duration) -> (Settled, Option<Box<dyn FocusFollow>>) {
        let (settled, follow) = self.0.settle_and_keep(within);
        (settled, follow.map(|f| Box::new(Follow(f)) as Box<dyn FocusFollow>))
    }
}

/// A written paste's control, kept while 自动跟踪输入框里的修改 follows it (docs/dictation.md §35.10).
struct Follow(voltip_focus::Follow);

impl FocusFollow for Follow {
    fn context(&self, hint: Option<&str>) -> Result<FocusContext, UnconfirmedWhy> {
        self.0.context(hint)
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

    static AROUND: std::sync::Mutex<&str> = std::sync::Mutex::new("前文你好");

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
        fn context(&mut self, _control: &u32, _hint: Option<&str>) -> Option<FocusContext> {
            let text = *AROUND.lock().unwrap();
            let n = text.chars().count();
            FocusContext::around(text, n, n)
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

    /// docs/dictation.md §35.10: a written paste's control comes back kept, and reads the user's
    /// edits through the platform reader.
    #[test]
    fn a_written_paste_keeps_its_control_for_following_edits() {
        let reader = SystemFocusReader(Reader::new(HostOs::Windows, open));
        let app = ForegroundApp { app_id: "notepad".into(), name: "notepad".into(), title: None, window: Some(1) };
        let watch = reader.watch(&app, &Expected::of("你好")).expect("read");
        let (settled, follow) = watch.settle_and_keep(Duration::from_secs(2));
        assert_eq!(settled, Settled::sure(InsertCheck::Written));
        let follow = follow.expect("kept");
        assert_eq!(follow.context(None).expect("read").before, "前文你好");
        *AROUND.lock().unwrap() = "前文您好";
        assert_eq!(follow.context(None).expect("read").before, "前文您好");
    }
}
