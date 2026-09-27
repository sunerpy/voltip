//! The phone's microphone (docs/dictation.md §20): the audio crate's recorder behind the core's
//! [`AudioSource`] port, with the 16 kHz live tap a phone take streams from. On Android cpal
//! opens AAudio; the `RECORD_AUDIO` runtime permission is asked for first through
//! `MicrophonePlugin.kt` ([`ensure_permission`]).

use std::sync::Arc;

use tauri::{AppHandle, Runtime};
use voltip_audio::{Backend, CpalBackend, LiveConsumer, LiveTapConfig, Recorder, RecorderConfig};
use voltip_core::dictation::{AudioSource, Capture, CaptureOptions, DictationError, LevelFrame, LivePcm, Recording};

/// Why a phone take could not open the microphone because of the permission.
pub const MICROPHONE_DENIED: &str = "microphone: 麦克风权限被拒绝，请在系统设置中允许 Voltip 使用麦克风";

/// The recorder as the core's microphone port.
pub struct PhoneMicrophone {
    backend: Arc<dyn Backend + Send + Sync>,
}

impl PhoneMicrophone {
    /// The platform sound system (AAudio on Android).
    pub fn cpal() -> Self {
        Self::with_backend(Arc::new(CpalBackend::new()))
    }

    /// Any backend (tests use `voltip_audio::FakeBackend`).
    pub fn with_backend(backend: Arc<dyn Backend + Send + Sync>) -> Self {
        Self { backend }
    }
}

impl AudioSource for PhoneMicrophone {
    fn start(
        &self,
        device_id: Option<&str>,
        on_level: Box<dyn Fn(LevelFrame) + Send>,
        on_ready: Box<dyn FnOnce() + Send>,
        options: CaptureOptions,
    ) -> Result<Box<dyn Capture>, DictationError> {
        let config = RecorderConfig {
            device_id: device_id.map(str::to_owned),
            live_tap: options.live.then(LiveTapConfig::default),
            max_duration: options.max_duration,
            ..RecorderConfig::default()
        };
        let recorder = Recorder::start_with_ready(
            self.backend.as_ref(),
            config,
            move |f| {
                on_level(LevelFrame {
                    rms_dbfs: f.rms_dbfs,
                    peak_dbfs: f.peak_dbfs,
                    clipping: f.clipping,
                    sample_rate_hz: f.sample_rate_hz,
                    channels: f.channels,
                    seq: f.seq,
                })
            },
            on_ready,
        )
        .map_err(|e| DictationError::Audio(e.to_string()))?;
        tracing::info!(device = %recorder.device().name, live = options.live, "phone microphone open");
        Ok(Box::new(PhoneCapture { recorder }))
    }
}

struct PhoneCapture {
    recorder: Recorder,
}

impl Capture for PhoneCapture {
    fn stop(self: Box<Self>) -> Result<Recording, DictationError> {
        let recording = self.recorder.stop().map_err(|e| DictationError::Audio(e.to_string()))?;
        Ok(Recording { wav: recording.to_wav(), duration_ms: recording.duration_ms, sample_rate_hz: recording.sample_rate_hz })
    }

    fn live_pcm(&mut self) -> Option<Box<dyn LivePcm>> {
        self.recorder.live_consumer().map(|consumer| Box::new(LiveTap(consumer)) as Box<dyn LivePcm>)
    }
}

struct LiveTap(LiveConsumer);

impl LivePcm for LiveTap {
    fn read(&mut self, out: &mut [f32]) -> usize {
        self.0.read(out)
    }

    fn overrun(&self) -> bool {
        self.0.overrun()
    }

    fn is_closed(&self) -> bool {
        self.0.is_closed()
    }
}

/// The Android plugin that owns the `RECORD_AUDIO` permission (`MicrophonePlugin.kt`).
#[cfg(target_os = "android")]
pub struct MicrophonePermission<R: Runtime>(tauri::plugin::PluginHandle<R>);

/// Registers the Android permission plugin; a no-op elsewhere (desktop builds of this crate).
pub fn init<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri::plugin::Builder::new("voltip-microphone")
        .setup(|_app, _api| {
            #[cfg(target_os = "android")]
            {
                use tauri::Manager as _;
                let handle = _api.register_android_plugin("dev.voltip.mobile", "MicrophonePlugin")?;
                _app.manage(MicrophonePermission(handle));
            }
            Ok(())
        })
        .build()
}

/// Whether a `checkPermissions` / `requestPermissions` answer grants the microphone.
pub fn granted(answer: &serde_json::Value) -> bool {
    answer.get("microphone").and_then(serde_json::Value::as_str) == Some("granted")
}

/// Make sure the app may record: on Android ask for `RECORD_AUDIO` when it is not granted yet
/// (the system dialog blocks, so it runs on a blocking thread). Elsewhere there is nothing to ask.
pub async fn ensure_permission<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        use tauri::Manager as _;
        let Some(plugin) = app.try_state::<MicrophonePermission<R>>() else { return Err("microphone: permission plugin missing".into()) };
        let handle = plugin.0.clone();
        let answer = tauri::async_runtime::spawn_blocking(move || -> Result<bool, String> {
            let now: serde_json::Value = handle.run_mobile_plugin("checkPermissions", ()).map_err(|e| e.to_string())?;
            if granted(&now) {
                return Ok(true);
            }
            let asked: serde_json::Value =
                handle.run_mobile_plugin("requestPermissions", serde_json::json!({ "permissions": ["microphone"] })).map_err(|e| e.to_string())?;
            Ok(granted(&asked))
        })
        .await
        .map_err(|e| e.to_string())??;
        if !answer {
            return Err(MICROPHONE_DENIED.into());
        }
    }
    let _ = app;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::time::Duration;

    use voltip_audio::FakeBackend;

    use super::*;

    /// The phone take streams from the live tap: the recorder opens with it, and stopping closes it.
    #[test]
    fn the_phone_microphone_opens_a_live_tap() {
        let mic = PhoneMicrophone::with_backend(Arc::new(FakeBackend::default()));
        let mut capture = mic.start(None, Box::new(|_| {}), Box::new(|| {}), CaptureOptions::LIVE).unwrap();
        let mut live = capture.live_pcm().unwrap();
        let mut buf = vec![0.0f32; 1600];
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut got = 0;
        while got == 0 && std::time::Instant::now() < deadline {
            got = live.read(&mut buf);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(got > 0, "the fake device delivers audio into the tap");
        capture.stop().unwrap();
        assert!(live.is_closed());
        assert!(granted(&serde_json::json!({ "microphone": "granted" })));
        assert!(!granted(&serde_json::json!({ "microphone": "prompt-with-rationale" })));
        assert!(!granted(&serde_json::json!({})));
    }
}
