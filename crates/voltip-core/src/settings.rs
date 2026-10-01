//! Persisted user settings (JSON, app data dir). No secrets live here.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::CoreError;
use crate::dictation::activation::{Activation, DEFAULT_HOLD_THRESHOLD_MS};
use crate::engines::EngineSettings;
use crate::hotkey::SoloKey;
use crate::scenes::ContextSharing;

/// File name inside the app data directory.
pub const SETTINGS_FILE_NAME: &str = "settings.json";
/// Schema version. 1 is the format of the first release (0.0.1); the unreleased builds
/// before it wrote other engine fields and secret-store entries, and those are not migrated.
pub const SETTINGS_SCHEMA: u16 = 1;

/// Where the dictation pill appears (`Settings.overlay`; the desktop shell places the window).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayPlacement {
    /// Centred above the bottom edge of the work area (default).
    #[default]
    Bottom,
    /// Centred below the top edge of the work area.
    Top,
    /// No pill: a take shows only in the main window and the tray.
    Off,
}

/// The four built-in themes of the design system.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeId {
    /// 明亮 · 白瓷 (default).
    #[default]
    Light,
    /// 暗黑 · 夜灯.
    Dark,
    /// 暖纸 · 手稿.
    Warm,
    /// 石墨 · 仪表.
    Graphite,
}

/// UI language. `System` follows the OS / webview language; the webview does the resolution,
/// the core only persists the choice so every window (main, pill, phone) reads the same value.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Locale {
    /// Follow the operating system (default).
    #[default]
    System,
    /// 简体中文.
    ZhCn,
    /// English.
    En,
}

/// How much dictation history is kept (docs/dictation.md §4).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct HistorySettings {
    /// Record takes at all; off keeps nothing new (what is already there stays until cleared).
    pub enabled: bool,
    /// Newest entries kept, [`crate::history::MIN_KEEP`]..=[`crate::history::MAX_ENTRIES`].
    pub keep: u32,
}

impl Default for HistorySettings {
    fn default() -> Self {
        Self { enabled: true, keep: crate::history::MAX_ENTRIES as u32 }
    }
}

/// Where a dictation take's audio comes from (docs/dictation.md §22).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingSource {
    /// The microphone ([`Settings::microphone`]); the default.
    #[default]
    Microphone,
    /// What the computer plays: the output device's sound.
    System,
    /// The microphone and the computer's sound, mixed.
    Mixed,
}

impl RecordingSource {
    /// Wire / log name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Microphone => "microphone",
            Self::System => "system",
            Self::Mixed => "mixed",
        }
    }

    /// Whether the microphone is part of it.
    pub const fn uses_microphone(self) -> bool {
        matches!(self, Self::Microphone | Self::Mixed)
    }

    /// Whether the computer's sound is part of it.
    pub const fn uses_output(self) -> bool {
        matches!(self, Self::System | Self::Mixed)
    }
}

/// The lengths a take may be limited to, in minutes ([`RecordingSettings::max_minutes`]).
pub const MAX_MINUTES_CHOICES: [u16; 7] = [1, 2, 5, 10, 30, 60, 120];
/// Default [`RecordingSettings::max_minutes`].
pub const DEFAULT_MAX_MINUTES: u16 = 10;

/// What a dictation take records and for how long (docs/dictation.md §22).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct RecordingSettings {
    /// The microphone, the computer's sound, or both.
    pub source: RecordingSource,
    /// The output device the computer's sound is recorded from (`system` / `mixed`): an id of
    /// `audio_outputs`; `None` follows the system's default output. A device that is not
    /// connected falls back to the default for that take, like the microphone.
    pub output_device: Option<String>,
    /// A take stops by itself after this many minutes: one of [`MAX_MINUTES_CHOICES`].
    pub max_minutes: u16,
    /// `mixed`: remove the microphone's echo of the computer's sound before the two are summed
    /// (docs/dictation.md §22.6). On by default; a settings file from before it has none and
    /// reads as on.
    pub echo_cancel: bool,
}

impl Default for RecordingSettings {
    fn default() -> Self {
        Self { source: RecordingSource::Microphone, output_device: None, max_minutes: DEFAULT_MAX_MINUTES, echo_cancel: true }
    }
}

impl RecordingSettings {
    /// How long a take records at most.
    pub fn max_duration(&self) -> Duration {
        Duration::from_secs(u64::from(self.max_minutes) * 60)
    }
}

/// User settings.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Settings {
    /// Schema version.
    pub schema: u16,
    /// UI theme.
    pub theme: ThemeId,
    /// Follow the OS light/dark preference.
    pub follow_system_theme: bool,
    /// Explicit relay override; `None` = build default (or no relay in release builds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay_url: Option<String>,
    /// Whether to use a relay at all.
    pub relay_enabled: bool,
    /// Global dictation hotkey in display form (`Ctrl+Alt+Space`); validated by [`crate::Hotkey`].
    #[serde(default = "default_hotkey")]
    pub hotkey: String,
    /// ASR / refine / inject configuration; absent in files written before the dictation pipeline.
    #[serde(default)]
    pub engines: EngineSettings,
    /// UI language; absent in files written before the language switch.
    #[serde(default)]
    pub locale: Locale,
    /// Check for updates on launch and download them in the background (desktop shell; needs an
    /// update source baked into the build). Off by default: the user opts in.
    #[serde(default)]
    pub auto_update: bool,
    /// How the hotkey drives a dictation (docs/dictation.md §13); absent in files written before
    /// the activation machine (= `hold`).
    #[serde(default)]
    pub activation: Activation,
    /// `hold_or_toggle`: a press released after this many milliseconds stops, a shorter one locks.
    #[serde(default = "default_hold_threshold_ms")]
    pub hold_threshold_ms: u32,
    /// Keep recording this long after a stop before the microphone closes (trailing syllables);
    /// `0` stops at once.
    #[serde(default)]
    pub extra_recording_ms: u32,
    /// Which parts of a take's context may go to the LLM clean-up (docs/dictation.md §18.5):
    /// the app name (on) and the window title (off); absent in files written before scenes.
    #[serde(default)]
    pub context_sharing: ContextSharing,
    /// The voice-edit hotkey (docs/dictation.md §19) in display form; `None` switches voice edit
    /// off. Absent in files written before it (= [`crate::hotkey::DEFAULT_EDIT_HOTKEY`]); always
    /// serialised, so a switched-off hotkey stays off (`null`) instead of the default coming back.
    #[serde(default = "default_edit_hotkey")]
    pub edit_hotkey: Option<String>,
    /// The lone-key trigger (docs/dictation.md §13.1): a right-hand modifier, Fn or a mouse button
    /// that drives a take on its own, next to [`Settings::hotkey`]; `None` (the default, and files
    /// written before it) = off.
    #[serde(default)]
    pub solo_key: Option<SoloKey>,
    /// Announce this device on the LAN and browse for the others (docs/pairing.md 「局域网发现」);
    /// on by default, and in files written before it.
    #[serde(default = "default_true")]
    pub lan_discovery: bool,
    /// Keep a pairing open on this desktop until turned off (docs/pairing.md 「常开配对」): a
    /// session always waits for a phone and is renewed before it lapses. Off by default.
    #[serde(default)]
    pub pairing_always_on: bool,
    /// History recording and retention.
    #[serde(default)]
    pub history: HistorySettings,
    /// Where the dictation pill appears.
    #[serde(default)]
    pub overlay: OverlayPlacement,
    /// The microphone takes record from: a device id of `audio_devices` (the stable
    /// `host:identifier` the audio backend hands out); `None` (the default, and files written
    /// before it) follows the system's default input. A chosen device that is not connected falls
    /// back to the default for that take (the desktop shell's recorder).
    #[serde(default)]
    pub microphone: Option<String>,
    /// The source and the longest length of a dictation take (docs/dictation.md §22); absent in
    /// files written before it (the microphone, 10 minutes).
    #[serde(default)]
    pub recording: RecordingSettings,
}

/// Longest device id `SetMicrophone` accepts (cpal ids are a host name and an endpoint id: well
/// under this).
pub const MAX_MICROPHONE_ID_BYTES: usize = 1024;

fn default_hotkey() -> String {
    crate::hotkey::DEFAULT_HOTKEY.to_string()
}

fn default_edit_hotkey() -> Option<String> {
    Some(crate::hotkey::DEFAULT_EDIT_HOTKEY.to_string())
}

fn default_true() -> bool {
    true
}

fn default_hold_threshold_ms() -> u32 {
    DEFAULT_HOLD_THRESHOLD_MS
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema: SETTINGS_SCHEMA,
            theme: ThemeId::Light,
            follow_system_theme: false,
            relay_url: None,
            relay_enabled: true,
            hotkey: default_hotkey(),
            engines: EngineSettings::default(),
            locale: Locale::System,
            auto_update: false,
            activation: Activation::Hold,
            hold_threshold_ms: DEFAULT_HOLD_THRESHOLD_MS,
            extra_recording_ms: 0,
            context_sharing: ContextSharing::default(),
            edit_hotkey: default_edit_hotkey(),
            solo_key: None,
            lan_discovery: true,
            pairing_always_on: false,
            history: HistorySettings::default(),
            overlay: OverlayPlacement::Bottom,
            microphone: None,
            recording: RecordingSettings::default(),
        }
    }
}

/// Loads / saves [`Settings`].
#[derive(Debug)]
pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    /// Store at `dir/settings.json`.
    pub fn new(dir: &Path) -> Self {
        Self { path: dir.join(SETTINGS_FILE_NAME) }
    }

    /// Load, or defaults when the file is absent. A corrupt file is an error, not silently reset.
    pub fn load(&self) -> Result<Settings, CoreError> {
        match std::fs::read(&self.path) {
            Ok(bytes) => {
                let s: Settings = serde_json::from_slice(&bytes).map_err(|e| CoreError::Settings(e.to_string()))?;
                if s.schema != SETTINGS_SCHEMA {
                    return Err(CoreError::Settings(format!("unsupported settings schema {}", s.schema)));
                }
                Ok(s)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(e) => Err(CoreError::Settings(e.to_string())),
        }
    }

    /// [`Self::load`], except that a corrupt or unsupported file does not stop the app from
    /// starting: the file is moved aside to `settings.json.corrupt-<unix seconds>` (never deleted,
    /// never overwritten) and the defaults are returned together with the reason, for the caller
    /// to log. A missing file is not a quarantine. I/O errors other than "not found" still fail:
    /// they say nothing about the file's content.
    pub fn load_or_quarantine(&self) -> Result<(Settings, Option<String>), CoreError> {
        match self.load() {
            Ok(settings) => Ok((settings, None)),
            Err(CoreError::Settings(reason)) if self.path.is_file() => {
                let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                let quarantine = self.path.with_file_name(format!("{SETTINGS_FILE_NAME}.corrupt-{stamp}"));
                std::fs::rename(&self.path, &quarantine).map_err(|e| CoreError::Settings(format!("{reason}; and moving the file aside failed: {e}")))?;
                Ok((Settings::default(), Some(format!("{reason}; the file was moved to {}", quarantine.display()))))
            }
            Err(e) => Err(e),
        }
    }

    /// Save atomically.
    pub fn save(&self, settings: &Settings) -> Result<(), CoreError> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| CoreError::Settings(e.to_string()))?;
        }
        let bytes = serde_json::to_vec_pretty(settings).map_err(|e| CoreError::Settings(e.to_string()))?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, bytes).map_err(|e| CoreError::Settings(e.to_string()))?;
        std::fs::rename(&tmp, &self.path).map_err(|e| CoreError::Settings(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every key of `script` survives a round trip through the schema (keys only: values may be
    /// normalised). A key the core no longer knows is dropped silently on load.
    fn assert_known_keys(script: &serde_json::Value, round: &serde_json::Value, at: &str) {
        let (Some(script), Some(round)) = (script.as_object(), round.as_object()) else { return };
        for (key, value) in script {
            let Some(kept) = round.get(key) else { panic!("{at}.{key}: not a settings key any more") };
            assert_known_keys(value, kept, &format!("{at}.{key}"));
        }
    }

    /// The smoke scripts write `settings.json` by hand. After the provider rework they still wrote
    /// `asr_kind` / `asr_url`, which load as nothing: the smoke that meant its mock ASR would have
    /// run the build's built-in service instead (2026-09-27).
    #[test]
    fn regression_the_smoke_scripts_write_the_current_settings_schema() {
        use crate::providers::ProviderId;
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for (script, provider) in [("scripts/lib/smoke-dictation.sh", ProviderId::Custom), ("scripts/smoke-streaming-linux.sh", ProviderId::Local)] {
            let text = std::fs::read_to_string(root.join(script)).unwrap();
            let start = text.find("{\n  \"schema\": 1").unwrap_or_else(|| panic!("{script}: no settings heredoc"));
            let len = text[start..].find("\n}\n").unwrap() + 2;
            let json = text[start..start + len].replace("$2", "http://127.0.0.1:9").replace("${4:-false}", "false").replace("${3:-paste}", "paste");
            let written: serde_json::Value = serde_json::from_str(&json).unwrap_or_else(|e| panic!("{script}: {e}"));
            let settings: Settings = serde_json::from_value(written.clone()).unwrap();
            assert_eq!(settings.engines.asr_provider, provider, "{script}");
            assert_known_keys(&written, &serde_json::to_value(&settings).unwrap(), script);
        }
    }

    #[test]
    fn defaults_roundtrip_and_corruption_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path());
        assert_eq!(store.load().unwrap(), Settings::default());
        let s = Settings { theme: ThemeId::Graphite, relay_url: Some("wss://x.example/ws".into()), ..Settings::default() };
        store.save(&s).unwrap();
        assert_eq!(store.load().unwrap(), s);
        std::fs::write(dir.path().join(SETTINGS_FILE_NAME), b"{").unwrap();
        assert!(matches!(store.load().unwrap_err(), CoreError::Settings(_)));
        std::fs::write(dir.path().join(SETTINGS_FILE_NAME), br#"{"schema":9,"theme":"light","follow_system_theme":false,"relay_enabled":true}"#).unwrap();
        assert!(store.load().unwrap_err().to_string().contains("schema"));
        assert_eq!(serde_json::to_string(&ThemeId::Warm).unwrap(), r#""warm""#);
        assert!(format!("{store:?}").contains("settings.json"));
    }

    /// A `settings.json` the app cannot parse (a hand edit, a crash mid-write, a value from a newer
    /// build) must not keep the app from starting (seen 2026-09-26: the setup hook panicked on an
    /// unknown `inject` variant). The file is moved aside, not deleted, and the defaults are used.
    #[test]
    fn regression_a_corrupt_settings_file_is_quarantined_and_the_app_starts_with_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path());
        assert_eq!(store.load_or_quarantine().unwrap(), (Settings::default(), None), "no file: defaults, nothing to quarantine");
        std::fs::write(
            dir.path().join(SETTINGS_FILE_NAME),
            br#"{"schema":1,"theme":"light","follow_system_theme":false,"relay_enabled":true,"engines":{"inject":"clipboard"}}"#,
        )
        .unwrap();
        let (settings, note) = store.load_or_quarantine().unwrap();
        assert_eq!(settings, Settings::default());
        let note = note.expect("the reason is reported");
        assert!(note.contains("`clipboard`") && note.contains(".corrupt-"), "{note}");
        assert!(!dir.path().join(SETTINGS_FILE_NAME).exists(), "the bad file is gone from the live path");
        let quarantined: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("settings.json.corrupt-"))
            .collect();
        assert_eq!(quarantined.len(), 1, "exactly one quarantine copy: {quarantined:?}");
        assert!(std::fs::read_to_string(dir.path().join(&quarantined[0])).unwrap().contains("\"clipboard\""), "the user's bytes are kept verbatim");
        let good = Settings { theme: ThemeId::Graphite, ..Settings::default() };
        store.save(&good).unwrap();
        assert_eq!(store.load_or_quarantine().unwrap(), (good, None), "a valid file is never touched");
    }

    /// A `settings.json` written before the dictation pipeline has no `engines` block (and no
    /// `hotkey`): it must load with the defaults, not fail or reset the user's other settings.
    #[test]
    fn regression_settings_without_engines_load_with_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path());
        std::fs::write(dir.path().join(SETTINGS_FILE_NAME), br#"{"schema":1,"theme":"warm","follow_system_theme":true,"relay_enabled":false}"#).unwrap();
        let s = store.load().unwrap();
        assert_eq!(s.theme, ThemeId::Warm);
        assert!(!s.relay_enabled);
        assert_eq!(s.hotkey, crate::DEFAULT_HOTKEY);
        assert_eq!(s.engines, EngineSettings::default());
        store.save(&s).unwrap();
        let text = std::fs::read_to_string(dir.path().join(SETTINGS_FILE_NAME)).unwrap();
        assert!(text.contains(r#""engines""#) && text.contains(r#""refine_enabled": true"#), "{text}");
        let s2 = Settings { engines: EngineSettings { inject: crate::InjectMode::ClipboardOnly, ..EngineSettings::default() }, ..Settings::default() };
        store.save(&s2).unwrap();
        assert_eq!(store.load().unwrap(), s2);
    }

    /// A `settings.json` written before the language switch and the updater has neither `locale`
    /// nor `auto_update`: it loads with `system` / off, and the next save writes both keys in the
    /// kebab-case / snake_case forms `packages/shared/src/schema.ts` expects.
    #[test]
    fn regression_settings_without_locale_and_auto_update_load_with_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path());
        std::fs::write(
            dir.path().join(SETTINGS_FILE_NAME),
            br#"{"schema":1,"theme":"dark","follow_system_theme":false,"relay_enabled":true,"hotkey":"Ctrl+Shift+D","engines":{"refine_enabled":false,"inject":"paste"}}"#,
        )
        .unwrap();
        let s = store.load().unwrap();
        assert_eq!(s.locale, Locale::System);
        assert!(!s.auto_update, "auto-update is opt-in");
        assert_eq!(s.theme, ThemeId::Dark);
        assert!(!s.engines.refine_enabled, "other settings survive");
        store.save(&s).unwrap();
        let text = std::fs::read_to_string(dir.path().join(SETTINGS_FILE_NAME)).unwrap();
        assert!(text.contains(r#""locale": "system""#) && text.contains(r#""auto_update": false"#), "{text}");
        let s2 = Settings { locale: Locale::ZhCn, auto_update: true, ..Settings::default() };
        store.save(&s2).unwrap();
        assert_eq!(store.load().unwrap(), s2);
        assert_eq!(serde_json::to_string(&Locale::ZhCn).unwrap(), r#""zh-cn""#);
        assert_eq!(serde_json::to_string(&Locale::En).unwrap(), r#""en""#);
        assert_eq!(serde_json::from_str::<Locale>(r#""system""#).unwrap(), Locale::System);
        assert!(serde_json::from_str::<Locale>(r#""zh_cn""#).is_err(), "only the kebab-case form is on the wire");
    }

    /// A `settings.json` written before scenes (docs/dictation.md §18.5) has no `context_sharing`:
    /// it loads with the app name shared and the window title not, and the next save writes both.
    #[test]
    fn regression_settings_without_context_sharing_load_with_the_private_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path());
        std::fs::write(
            dir.path().join(SETTINGS_FILE_NAME),
            br#"{"schema":1,"theme":"warm","follow_system_theme":false,"relay_enabled":true,"activation":"toggle"}"#,
        )
        .unwrap();
        let s = store.load().unwrap();
        assert_eq!(s.context_sharing, ContextSharing { app_name: true, window_title: false });
        assert_eq!(s.activation, Activation::Toggle, "other settings survive");
        store.save(&s).unwrap();
        let text = std::fs::read_to_string(dir.path().join(SETTINGS_FILE_NAME)).unwrap();
        assert!(text.contains(r#""context_sharing": {"#) && text.contains(r#""window_title": false"#), "{text}");
        let s2 = Settings { context_sharing: ContextSharing { app_name: false, window_title: true }, ..Settings::default() };
        store.save(&s2).unwrap();
        assert_eq!(store.load().unwrap(), s2);
    }

    /// A `settings.json` written before the activation machine (docs/dictation.md §13) has no
    /// `activation` / `hold_threshold_ms` / `extra_recording_ms`: it loads as `hold` / 300 / 0, the
    /// next save writes the three keys in the snake_case forms the TypeScript schema expects, and
    /// a file with the new keys round-trips.
    #[test]
    fn regression_settings_without_activation_load_with_hold_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path());
        std::fs::write(
            dir.path().join(SETTINGS_FILE_NAME),
            br#"{"schema":1,"theme":"warm","follow_system_theme":false,"relay_enabled":true,"hotkey":"Ctrl+Alt+Space","engines":{"refine_enabled":true,"inject":"paste"},"locale":"en","auto_update":true}"#,
        )
        .unwrap();
        let s = store.load().unwrap();
        assert_eq!(s.activation, Activation::Hold);
        assert_eq!(s.hold_threshold_ms, 300);
        assert_eq!(s.extra_recording_ms, 0);
        assert_eq!((s.theme, s.locale, s.auto_update), (ThemeId::Warm, Locale::En, true), "other settings survive");
        store.save(&s).unwrap();
        let text = std::fs::read_to_string(dir.path().join(SETTINGS_FILE_NAME)).unwrap();
        assert!(
            text.contains(r#""activation": "hold""#) && text.contains(r#""hold_threshold_ms": 300"#) && text.contains(r#""extra_recording_ms": 0"#),
            "{text}"
        );
        let s2 = Settings { activation: Activation::HoldOrToggle, hold_threshold_ms: 450, extra_recording_ms: 200, ..Settings::default() };
        store.save(&s2).unwrap();
        assert_eq!(store.load().unwrap(), s2);
        assert!(std::fs::read_to_string(dir.path().join(SETTINGS_FILE_NAME)).unwrap().contains(r#""activation": "hold_or_toggle""#));
        assert!(
            serde_json::from_str::<Settings>(r#"{"schema":1,"theme":"light","follow_system_theme":false,"relay_enabled":true,"activation":"press"}"#).is_err()
        );
    }

    /// docs/dictation.md §19: a `settings.json` written before voice edit reads with the default
    /// edit hotkey; switching it off persists `null` (not a missing key that would bring the
    /// default back on the next start), and a chosen chord round-trips.
    #[test]
    fn regression_settings_without_edit_hotkey_load_with_the_default_and_null_stays_off() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path());
        std::fs::write(
            dir.path().join(SETTINGS_FILE_NAME),
            br#"{"schema":1,"theme":"light","follow_system_theme":false,"relay_enabled":true,"hotkey":"Ctrl+Alt+Space"}"#,
        )
        .unwrap();
        let s = store.load().unwrap();
        assert_eq!(s.edit_hotkey.as_deref(), Some(crate::hotkey::DEFAULT_EDIT_HOTKEY));
        assert_eq!(Settings::default().edit_hotkey.as_deref(), Some("Ctrl+Alt+E"));
        let off = Settings { edit_hotkey: None, ..s };
        store.save(&off).unwrap();
        let text = std::fs::read_to_string(dir.path().join(SETTINGS_FILE_NAME)).unwrap();
        assert!(text.contains(r#""edit_hotkey": null"#), "switched off is written out: {text}");
        assert_eq!(store.load().unwrap().edit_hotkey, None, "and stays off");
        let chosen = Settings { edit_hotkey: Some("Ctrl+Shift+E".into()), ..Settings::default() };
        store.save(&chosen).unwrap();
        assert_eq!(store.load().unwrap(), chosen);
    }

    /// docs/dictation.md §13.1: files written before the lone-key trigger read with it off; a
    /// chosen key round-trips under its wire name, an unknown one is refused.
    #[test]
    fn settings_written_before_always_on_pairing_read_it_off() {
        let old: Settings = serde_json::from_str(r#"{"schema":1,"theme":"light","follow_system_theme":false,"relay_enabled":true}"#).unwrap();
        assert!(!old.pairing_always_on && old.lan_discovery);
        assert!(!Settings::default().pairing_always_on);
    }

    #[test]
    fn regression_settings_without_a_microphone_follow_the_default_and_a_chosen_one_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path());
        store.save(&Settings::default()).unwrap();
        assert_eq!(store.load().unwrap().microphone, None);
        let chosen = Settings { microphone: Some("wasapi:{0.0.1.00000000}.{c2}".into()), ..Settings::default() };
        store.save(&chosen).unwrap();
        assert_eq!(store.load().unwrap().microphone.as_deref(), Some("wasapi:{0.0.1.00000000}.{c2}"));
        // A file written before the setting reads as the default input.
        let old: Settings = serde_json::from_str(r#"{"schema":1,"theme":"light","follow_system_theme":false,"relay_enabled":true}"#).unwrap();
        assert_eq!(old.microphone, None);
    }

    #[test]
    fn settings_without_solo_key_read_it_off_and_a_chosen_key_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path());
        std::fs::write(dir.path().join(SETTINGS_FILE_NAME), br#"{"schema":1,"theme":"light","follow_system_theme":false,"relay_enabled":true}"#).unwrap();
        assert_eq!(store.load().unwrap().solo_key, None);
        let chosen = Settings { solo_key: Some(SoloKey::MouseBack), ..Settings::default() };
        store.save(&chosen).unwrap();
        assert!(std::fs::read_to_string(dir.path().join(SETTINGS_FILE_NAME)).unwrap().contains(r#""solo_key": "mouse_back""#));
        assert_eq!(store.load().unwrap(), chosen);
        assert!(
            serde_json::from_str::<Settings>(r#"{"schema":1,"theme":"light","follow_system_theme":false,"relay_enabled":true,"solo_key":"caps_lock"}"#)
                .is_err()
        );
    }
}
