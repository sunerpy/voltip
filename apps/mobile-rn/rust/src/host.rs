//! What the shell needs from the app around it (docs/mobile-rn.md §2): a way to hand events to the
//! JavaScript side, and the few capabilities only the platform's own APIs reach. On a phone the
//! Kotlin module implements it, through the UniFFI interface [`crate::ffi::PlatformHost`]; tests
//! use [`RecordingHost`].
//!
//! Every call may block (the clipboard and the share sheet run on Android's main thread and the
//! caller waits for them): the shell makes them from blocking threads, never from a core task.

use std::sync::Mutex;

/// The platform around the shell.
pub trait Host: Send + Sync + 'static {
    /// One `UiEvent`, serialized, for `voltip://event`.
    fn event(&self, json: &str);
    /// One frame of the stream `channel` (the level meter's `onFrame`), serialized.
    fn channel(&self, channel: u64, json: &str);
    /// The text on the clipboard; `None` when it holds none.
    fn clipboard_read(&self) -> Result<Option<String>, String>;
    /// Put `text` on the clipboard.
    fn clipboard_write(&self, text: &str) -> Result<(), String>;
    /// Open the system share sheet with `text`.
    fn share_text(&self, text: &str) -> Result<(), String>;
    /// Open the system share sheet with `text` as the file `name` of type `mime`.
    fn share_file(&self, name: &str, text: &str, mime: &str) -> Result<(), String>;
    /// Hold (`true`) or release the Wi-Fi multicast lock LAN discovery needs.
    fn multicast(&self, held: bool);
    /// Open `url` in the browser (or the app that handles it).
    fn open_url(&self, url: &str) -> Result<(), String>;
    /// The package that installed the app (`com.android.vending` for Google Play), as the system
    /// recorded it; `None` when it recorded none (`adb`) or cannot say. Updates follow it
    /// (`crate::update`).
    fn installer(&self) -> Option<String> {
        None
    }
}

/// One call a [`RecordingHost`] saw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostCall {
    /// [`Host::event`].
    Event(String),
    /// [`Host::channel`].
    Channel(u64, String),
    /// [`Host::clipboard_read`].
    ClipboardRead,
    /// [`Host::clipboard_write`].
    ClipboardWrite(String),
    /// [`Host::share_text`].
    ShareText(String),
    /// [`Host::share_file`].
    ShareFile {
        /// File name.
        name: String,
        /// Content.
        text: String,
        /// MIME type.
        mime: String,
    },
    /// [`Host::multicast`].
    Multicast(bool),
    /// [`Host::open_url`].
    OpenUrl(String),
}

/// A host for tests (and for a build without a platform): it records every call, answers the
/// clipboard from its own buffer, and refuses what it is told to refuse.
#[derive(Debug, Default)]
pub struct RecordingHost {
    calls: Mutex<Vec<HostCall>>,
    clipboard: Mutex<Option<String>>,
    /// When set, every capability call fails with this text (events still arrive).
    refuse: Mutex<Option<String>>,
    /// What [`Host::installer`] answers.
    installer: Mutex<Option<String>>,
}

impl RecordingHost {
    /// A host whose capability calls all fail with `reason`.
    pub fn refusing(reason: &str) -> Self {
        let host = Self::default();
        host.refuse(Some(reason));
        host
    }

    /// From now on fail every capability call with `reason`, or answer them again (`None`).
    pub fn refuse(&self, reason: Option<&str>) {
        *self.refuse.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = reason.map(str::to_owned);
    }

    /// Everything seen so far.
    pub fn calls(&self) -> Vec<HostCall> {
        self.calls.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }

    /// The events seen so far, parsed.
    pub fn events(&self) -> Vec<serde_json::Value> {
        self.calls()
            .into_iter()
            .filter_map(|c| match c {
                HostCall::Event(json) => serde_json::from_str(&json).ok(),
                _ => None,
            })
            .collect()
    }

    /// Who [`Host::installer`] says installed the app.
    pub fn set_installer(&self, installer: Option<&str>) {
        *self.installer.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = installer.map(str::to_owned);
    }

    /// Put `text` on the recorded clipboard.
    pub fn set_clipboard(&self, text: Option<&str>) {
        *self.clipboard.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = text.map(str::to_owned);
    }

    /// What the recorded clipboard holds.
    pub fn clipboard(&self) -> Option<String> {
        self.clipboard.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }

    fn record(&self, call: HostCall) {
        self.calls.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(call);
    }

    fn outcome(&self) -> Result<(), String> {
        self.refuse.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone().map_or(Ok(()), Err)
    }
}

impl Host for RecordingHost {
    fn event(&self, json: &str) {
        self.record(HostCall::Event(json.to_owned()));
    }

    fn channel(&self, channel: u64, json: &str) {
        self.record(HostCall::Channel(channel, json.to_owned()));
    }

    fn clipboard_read(&self) -> Result<Option<String>, String> {
        self.record(HostCall::ClipboardRead);
        self.outcome()?;
        Ok(self.clipboard())
    }

    fn clipboard_write(&self, text: &str) -> Result<(), String> {
        self.record(HostCall::ClipboardWrite(text.to_owned()));
        self.outcome()?;
        self.set_clipboard(Some(text));
        Ok(())
    }

    fn share_text(&self, text: &str) -> Result<(), String> {
        self.record(HostCall::ShareText(text.to_owned()));
        self.outcome()
    }

    fn share_file(&self, name: &str, text: &str, mime: &str) -> Result<(), String> {
        self.record(HostCall::ShareFile { name: name.to_owned(), text: text.to_owned(), mime: mime.to_owned() });
        self.outcome()
    }

    fn multicast(&self, held: bool) {
        self.record(HostCall::Multicast(held));
    }

    fn open_url(&self, url: &str) -> Result<(), String> {
        self.record(HostCall::OpenUrl(url.to_owned()));
        self.outcome()
    }

    fn installer(&self) -> Option<String> {
        self.installer.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }
}
