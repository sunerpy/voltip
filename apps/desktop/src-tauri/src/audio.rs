//! Native microphone enumeration and level metering for the webview (`voltip-audio` on cpal).
//!
//! One [`AudioHub`] owns the microphone. Every `audio_meter_start` registers a subscriber
//! ([`tauri::ipc::Channel`], ≈ 30 frames a second) and gets a subscription id back;
//! `audio_meter_stop { id }` removes exactly that subscriber. The hub opens a device meter while
//! at least one subscriber exists and no dictation capture is running. When the dictation
//! recorder needs the microphone it calls [`AudioHub::enter_capture`] **before** opening the
//! device, which drops the meter and re-routes every subscriber to the recorder's level frames;
//! [`AudioHub::leave_capture`] reopens the device meter for the remaining subscribers. Subscribers
//! never notice the switch, so the home card's meter keeps moving after a dictation and the
//! device is never opened twice (WASAPI exclusive-mode devices would refuse the second open).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;
use serde::Serialize;

/// A microphone as enumerated by the backend.
pub type Device = voltip_audio::AudioDevice;
/// One level frame.
pub type Frame = voltip_audio::LevelFrame;

/// Where frames go (a webview channel in production, a closure in tests).
pub type Sink = Arc<dyn Fn(Frame) + Send + Sync>;

/// How frames are produced right now.
enum Source {
    /// Nothing open: no subscribers.
    Idle,
    /// Our own device stream (held so it drops — and stops — when replaced).
    Device(#[allow(dead_code)] voltip_audio::Meter),
    /// The dictation recorder owns the microphone and pushes frames through [`AudioHub::push`].
    Capture,
    /// A device meter was wanted but could not be opened (device gone / busy); retried on the
    /// next subscriber change or when the capture ends.
    Unavailable(String),
}

struct Inner {
    source: Source,
    subscribers: BTreeMap<u64, Sink>,
    /// Device the subscribers asked for (the last `audio_meter_start` wins); `None` = default.
    device_id: Option<String>,
    /// Frames counted per source so a switch is visible in tests / logs.
    frames: u64,
}

/// Single owner of the microphone level stream (managed Tauri state).
pub struct AudioHub {
    inner: Mutex<Inner>,
    next_id: AtomicU64,
    /// Opens the device meter; swapped for a fake in tests.
    open: Box<dyn Fn(Option<String>, Sink) -> Result<voltip_audio::Meter, String> + Send + Sync>,
}

impl Default for AudioHub {
    fn default() -> Self {
        Self::with_opener(Box::new(|device_id, sink| {
            let open = |device_id: Option<String>| {
                let sink = sink.clone();
                let config = voltip_audio::MeterConfig { device_id, ..voltip_audio::MeterConfig::default() };
                voltip_audio::Meter::start(config, move |frame| sink(frame))
            };
            // The chosen microphone is gone (unplugged): meter the default input, as a take would.
            match open(device_id.clone()) {
                Err(voltip_audio::AudioError::DeviceNotFound(id)) if device_id.is_some() => {
                    tracing::warn!(device = %id, "the chosen microphone is not connected; metering the default input");
                    open(None).map_err(|e| e.to_string())
                }
                other => other.map_err(|e| e.to_string()),
            }
        }))
    }
}

impl AudioHub {
    /// A hub whose device meter comes from `open` (tests pass a fake backend).
    pub fn with_opener(open: Box<dyn Fn(Option<String>, Sink) -> Result<voltip_audio::Meter, String> + Send + Sync>) -> Self {
        Self { inner: Mutex::new(Inner { source: Source::Idle, subscribers: BTreeMap::new(), device_id: None, frames: 0 }), next_id: AtomicU64::new(1), open }
    }

    /// Register a subscriber; returns its id. Opens the device meter when this is the first one and
    /// no capture is running. If the device cannot be opened the subscription is **rolled back**
    /// and the error returned: a caller that never received an id could never unsubscribe, and a
    /// subscriber nobody can remove would keep reopening the device after its page is gone.
    pub fn subscribe(self: &Arc<Self>, device_id: Option<String>, sink: Sink) -> Result<u64, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (outcome, retired) = {
            let mut inner = self.inner.lock();
            inner.subscribers.insert(id, sink);
            if device_id.is_some() {
                inner.device_id = device_id;
            }
            let (outcome, retired) = self.reconcile(&mut inner);
            if outcome.is_err() {
                inner.subscribers.remove(&id);
                if inner.subscribers.is_empty() {
                    inner.source = Source::Idle;
                }
            }
            (outcome, retired)
        };
        drop(retired); // a stream drop may join the audio thread; never do that under the lock
        match &outcome {
            Ok(()) => tracing::info!(id, subscribers = self.subscriber_count(), "level meter subscriber added"),
            Err(e) => tracing::warn!(id, error = %e, "level meter subscription rolled back"),
        }
        outcome.map(|()| id)
    }

    /// Remove a subscriber; the device meter closes with the last one.
    pub fn unsubscribe(self: &Arc<Self>, id: u64) {
        let retired = {
            let mut inner = self.inner.lock();
            if inner.subscribers.remove(&id).is_none() {
                return;
            }
            tracing::info!(id, subscribers = inner.subscribers.len(), "level meter subscriber removed");
            self.reconcile(&mut inner).1
        };
        drop(retired);
    }

    /// The dictation recorder is about to open the microphone: release ours and take frames from
    /// [`AudioHub::push`] until [`AudioHub::leave_capture`]. Idempotent.
    pub fn enter_capture(&self) {
        let retired = {
            let mut inner = self.inner.lock();
            if matches!(inner.source, Source::Capture) {
                return;
            }
            inner.frames = 0;
            tracing::info!(subscribers = inner.subscribers.len(), "level meter following the dictation capture");
            std::mem::replace(&mut inner.source, Source::Capture)
        };
        drop(retired); // stops the device stream, outside the lock
    }

    /// The recorder released the microphone: reopen the device meter if anyone still listens.
    pub fn leave_capture(self: &Arc<Self>) {
        let retired = {
            let mut inner = self.inner.lock();
            if !matches!(inner.source, Source::Capture) {
                return;
            }
            inner.source = Source::Idle;
            tracing::info!("level meter detached from the dictation capture");
            self.reconcile(&mut inner).1
        };
        drop(retired);
    }

    /// A frame from the dictation recorder (ignored unless a capture is active).
    pub fn push(&self, frame: Frame) {
        let sinks: Vec<Sink> = {
            let mut inner = self.inner.lock();
            if !matches!(inner.source, Source::Capture) {
                return;
            }
            inner.frames += 1;
            inner.subscribers.values().cloned().collect()
        };
        for sink in sinks {
            sink(frame);
        }
    }

    /// Whether a device meter or capture feed is live.
    pub fn is_running(&self) -> bool {
        matches!(self.inner.lock().source, Source::Device(_) | Source::Capture)
    }

    /// Number of subscribers (tests / diagnostics).
    pub fn subscriber_count(&self) -> usize {
        self.inner.lock().subscribers.len()
    }

    /// Whether frames currently come from the dictation capture.
    pub fn following_capture(&self) -> bool {
        matches!(self.inner.lock().source, Source::Capture)
    }

    /// Why the device meter is not running although subscribers want it (`None` when it runs or
    /// nobody listens).
    pub fn unavailable_reason(&self) -> Option<String> {
        match &self.inner.lock().source {
            Source::Unavailable(reason) => Some(reason.clone()),
            _ => None,
        }
    }

    /// Make the source match the subscriber set: open a device meter when subscribers exist and
    /// no capture owns the microphone; close it when the last subscriber left. Never touches a
    /// running capture.
    fn reconcile(self: &Arc<Self>, inner: &mut Inner) -> (Result<(), String>, Option<Source>) {
        match (&inner.source, inner.subscribers.is_empty()) {
            (Source::Capture, _) => (Ok(()), None),
            (Source::Device(_), true) => {
                tracing::info!("level meter stopped (no subscribers)");
                (Ok(()), Some(std::mem::replace(&mut inner.source, Source::Idle)))
            }
            (Source::Device(_), false) => (Ok(()), None),
            (Source::Idle | Source::Unavailable(_), true) => {
                inner.source = Source::Idle;
                (Ok(()), None)
            }
            (Source::Idle | Source::Unavailable(_), false) => {
                let hub = Arc::downgrade(self);
                let sink: Sink = Arc::new(move |frame| {
                    if let Some(hub) = hub.upgrade() {
                        hub.fan_out(frame);
                    }
                });
                match (self.open)(inner.device_id.clone(), sink) {
                    Ok(meter) => {
                        tracing::info!(device = %meter.device().name, "level meter started");
                        inner.source = Source::Device(meter);
                        inner.frames = 0;
                        (Ok(()), None)
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "level meter could not open the device");
                        inner.source = Source::Unavailable(e.clone());
                        (Err(e), None)
                    }
                }
            }
        }
    }

    /// Device-meter frames: deliver to every subscriber (only while the device source is live, so a
    /// late frame from a meter being dropped cannot leak after a switch to the capture).
    fn fan_out(&self, frame: Frame) {
        let sinks: Vec<Sink> = {
            let mut inner = self.inner.lock();
            if !matches!(inner.source, Source::Device(_)) {
                return;
            }
            inner.frames += 1;
            inner.subscribers.values().cloned().collect()
        };
        for sink in sinks {
            sink(frame);
        }
    }
}

/// Enumerate input devices off the UI thread (WASAPI / CoreAudio enumeration can block).
pub async fn devices() -> Result<Vec<Device>, String> {
    tauri::async_runtime::spawn_blocking(voltip_audio::list_input_devices).await.map_err(|e| e.to_string())?.map_err(|e| e.to_string())
}

/// Register a webview channel as a subscriber (device open happens off the UI thread).
pub async fn start(hub: Arc<AudioHub>, device_id: Option<String>, on_frame: tauri::ipc::Channel<Frame>) -> Result<u64, String> {
    let sink: Sink = Arc::new(move |frame| {
        if let Err(e) = on_frame.send(frame) {
            tracing::debug!(error = %e, "level frame dropped (webview gone?)");
        }
    });
    tauri::async_runtime::spawn_blocking(move || hub.subscribe(device_id, sink)).await.map_err(|e| e.to_string())?
}

/// Mirror of the wire shape, kept so the contract test can pin the field names.
#[derive(Serialize)]
#[allow(dead_code)]
struct FrameShape {
    rms_dbfs: f32,
    peak_dbfs: f32,
    clipping: bool,
    sample_rate_hz: u32,
    channels: u16,
    seq: u64,
}

/// Whether a status is a phone's take that is recording (docs/dictation.md §20).
pub fn phone_take_listening(status: &voltip_core::DictationStatus) -> bool {
    status.remote.is_some() && matches!(status.phase, voltip_core::DictationPhase::Listening { .. })
}

/// While a paired phone's take records, the level meters show the phone's audio: the hub leaves
/// the local microphone alone (it would open the device for the pill's meter) and takes the core's
/// level frames instead, which the phone's stream produces. A local take feeds the hub from its
/// own recorder, so only phone takes are forwarded.
pub fn follow_phone_takes(bridge: voltip_tauri_bridge::Bridge, hub: Arc<AudioHub>) {
    use tokio::sync::broadcast::error::RecvError;
    let mut events = bridge.events();
    let mut levels = bridge.levels();
    tauri::async_runtime::spawn(async move {
        let mut forwarding = false;
        loop {
            tokio::select! {
                event = events.recv() => match event {
                    Ok(voltip_core::ui::UiEvent::Dictation(status)) => {
                        let now = phone_take_listening(&status);
                        if now != forwarding {
                            forwarding = now;
                            if now {
                                // Frames queued before the take are the microphone's, not the phone's.
                                levels = levels.resubscribe();
                                hub.enter_capture();
                            } else {
                                hub.leave_capture();
                            }
                        }
                    }
                    Ok(_) | Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => break,
                },
                frame = levels.recv(), if forwarding => match frame {
                    Ok(frame) => hub.push(crate::dictation::to_audio_frame(frame)),
                    Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => break,
                },
            }
        }
    });
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    /// docs/dictation.md §20: only a phone's take that records feeds the meters from the core.
    #[test]
    fn only_a_recording_phone_take_is_forwarded_to_the_meters() {
        use voltip_core::{DictationPhase, DictationStatus};
        let listening = DictationPhase::Listening { started_at: 1, ready: true, live: None, locked: false };
        let mut status = DictationStatus::dictation(listening.clone(), 1);
        assert!(!phone_take_listening(&status), "the microphone's own take feeds the hub itself");
        status.remote = Some("Pixel 8".into());
        assert!(phone_take_listening(&status));
        status.phase = DictationPhase::Cancelled { injected_chars: 0 };
        assert!(!phone_take_listening(&status));
    }
    use std::sync::atomic::AtomicUsize;

    fn fake_hub(fail: bool) -> Arc<AudioHub> {
        Arc::new(AudioHub::with_opener(Box::new(move |_, sink| {
            if fail {
                return Err("device busy".into());
            }
            let backend = voltip_audio::FakeBackend::new();
            voltip_audio::Meter::start_with(&backend, voltip_audio::MeterConfig::default(), move |f| sink(f)).map_err(|e| e.to_string())
        })))
    }

    fn frame(seq: u64) -> Frame {
        Frame { rms_dbfs: -20.0, peak_dbfs: -10.0, clipping: false, sample_rate_hz: 16_000, channels: 1, seq }
    }

    #[test]
    fn subscribers_keep_receiving_across_a_capture_switch() {
        let hub = fake_hub(false);
        let got = Arc::new(AtomicUsize::new(0));
        let g = got.clone();
        let id = hub
            .subscribe(
                None,
                Arc::new(move |_| {
                    g.fetch_add(1, Ordering::Relaxed);
                }),
            )
            .expect("device meter opens");
        assert!(hub.is_running() && !hub.following_capture());
        // The recorder takes the microphone: the device meter is dropped, frames now come from push.
        hub.enter_capture();
        assert!(hub.following_capture());
        let before = got.load(Ordering::Relaxed);
        hub.push(frame(1));
        hub.push(frame(2));
        assert_eq!(got.load(Ordering::Relaxed), before + 2);
        // Capture over: the device meter reopens for the same subscriber without any webview call.
        hub.leave_capture();
        assert!(hub.is_running() && !hub.following_capture());
        // Pushes outside a capture are ignored.
        let before = got.load(Ordering::Relaxed);
        hub.push(frame(3));
        assert_eq!(got.load(Ordering::Relaxed), before);
        hub.unsubscribe(id);
        assert!(!hub.is_running());
        assert_eq!(hub.subscriber_count(), 0);
    }

    #[test]
    fn last_unsubscribe_closes_the_device_and_capture_without_subscribers_is_harmless() {
        let hub = fake_hub(false);
        let a = hub.subscribe(None, Arc::new(|_| {})).expect("open");
        let b = hub.subscribe(Some("x".into()), Arc::new(|_| {})).expect("second subscriber shares the meter");
        assert_eq!(hub.subscriber_count(), 2);
        hub.unsubscribe(a);
        assert!(hub.is_running(), "one subscriber left keeps the meter");
        hub.unsubscribe(b);
        assert!(!hub.is_running());
        hub.unsubscribe(b); // unknown id: no-op
        hub.enter_capture();
        hub.push(frame(1));
        hub.leave_capture();
        assert!(!hub.is_running(), "nobody listens, so no device meter reopens");
    }

    /// Regression: a failed device open used to leave a subscription nobody
    /// could cancel (the caller never got an id), which later reopened the device for a page that
    /// was long gone. Now the subscription is rolled back with the error.
    #[test]
    fn regression_a_failed_open_rolls_the_subscription_back_so_nothing_leaks() {
        let hub = fake_hub(true);
        let got = Arc::new(AtomicUsize::new(0));
        let g = got.clone();
        let err = hub
            .subscribe(
                None,
                Arc::new(move |_| {
                    g.fetch_add(1, Ordering::Relaxed);
                }),
            )
            .expect_err("device busy");
        assert!(err.contains("busy"));
        assert_eq!(hub.subscriber_count(), 0, "the failed subscription must not linger");
        assert!(!hub.is_running());
        assert_eq!(hub.unavailable_reason(), None, "nothing is wanted, so nothing is unavailable");
        // A later capture has nobody to feed, and its end reopens no device.
        hub.enter_capture();
        hub.push(frame(1));
        assert_eq!(got.load(Ordering::Relaxed), 0);
        hub.leave_capture();
        assert!(!hub.is_running());
        assert_eq!(hub.subscriber_count(), 0);
        // A second subscriber that also fails does not disturb the first's absence; unsubscribe of a
        // never-issued id is a no-op.
        assert!(hub.subscribe(None, Arc::new(|_| {})).is_err());
        hub.unsubscribe(1);
        assert_eq!(hub.subscriber_count(), 0);
    }
}
