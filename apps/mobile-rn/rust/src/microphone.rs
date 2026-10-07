//! The phone's microphone (docs/dictation.md §20): the audio crate's recorder behind the core's
//! [`AudioSource`] port, with the 16 kHz live tap a phone take streams from and, for a long take
//! the phone recognises itself (§20.7, §22), the stream the core writes to its recording file. On
//! Android cpal opens AAudio. The `RECORD_AUDIO` runtime permission is the app's to ask for before a
//! take starts (`src/backend/transport.ts`), so this module never shows a dialog.
//!
//! The same recorder as the Tauri phone shell's (`apps/mobile/src-tauri/src/microphone.rs`), without
//! its permission plugin; see docs/mobile-rn.md §3 on the copy.

use std::sync::Arc;

use voltip_audio::{Backend, CpalBackend, LiveConsumer, LiveTapConfig, PcmConsumer, PcmStreamConfig, Recorder, RecorderConfig};
use voltip_core::dictation::{AudioSource, Capture, CaptureOptions, DictationError, LevelFrame, LivePcm, MAX_RECORDING, PcmStream, Recording};

/// Why a phone take could not open the microphone because of the permission (the Tauri shell's
/// text, which `@voltip/shared`'s labels already know).
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
            // As on the desktop: a long take keeps its first two minutes in memory, and the core
            // writes the whole take to its recording file from the stream.
            max_duration: if options.long { options.max_duration.min(MAX_RECORDING) } else { options.max_duration },
            pcm_stream: options.long.then(PcmStreamConfig::default),
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

    fn pcm_stream(&mut self) -> Option<Box<dyn PcmStream>> {
        self.recorder.pcm_consumer().map(|consumer| Box::new(TakeStream(consumer)) as Box<dyn PcmStream>)
    }
}

/// The recorder's long-take stream as the core's [`PcmStream`] (docs/dictation.md §22).
struct TakeStream(PcmConsumer);

impl PcmStream for TakeStream {
    fn read(&mut self, out: &mut [f32]) -> usize {
        self.0.read(out)
    }

    fn gap(&mut self) -> Option<u64> {
        self.0.gap()
    }

    fn is_closed(&self) -> bool {
        self.0.is_closed()
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::time::{Duration, Instant};

    use voltip_audio::FakeBackend;

    use super::*;

    /// Read from `read` until it delivers samples or five seconds pass (the fake device delivers on
    /// its own thread); the number read.
    fn first_samples(mut read: impl FnMut(&mut [f32]) -> usize) -> usize {
        let mut buf = vec![0.0f32; 1600];
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let got = read(&mut buf);
            if got > 0 || Instant::now() >= deadline {
                return got;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// The phone take streams from the live tap: the recorder opens with it, and stopping closes it.
    /// The take's level frames reach the core (the meter shows them).
    #[test]
    fn the_phone_microphone_opens_a_live_tap() {
        let mic = PhoneMicrophone::with_backend(Arc::new(FakeBackend::default()));
        let levels = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = levels.clone();
        let on_level = Box::new(move |f: LevelFrame| seen.lock().unwrap().push(f));
        let mut capture = mic.start(None, on_level, Box::new(|| {}), CaptureOptions::LIVE).unwrap();
        let mut live = capture.live_pcm().unwrap();
        assert!(first_samples(|buf| live.read(buf)) > 0, "the fake device delivers audio into the tap");
        assert!(!live.overrun());
        let deadline = Instant::now() + Duration::from_secs(5);
        while levels.lock().unwrap().is_empty() {
            assert!(Instant::now() < deadline, "no level frame within five seconds");
            std::thread::sleep(Duration::from_millis(10));
        }
        let first = levels.lock().unwrap()[0];
        assert!(first.sample_rate_hz > 0 && first.channels > 0, "{first:?}");
        let recording = capture.stop().unwrap();
        assert!(recording.wav.starts_with(b"RIFF"));
        assert!(live.is_closed());
    }

    /// A long take the phone recognises itself (docs/dictation.md §20.7, §22) streams the whole
    /// take for the core's recording file, as the desktop does; a short one has no stream.
    #[test]
    fn a_long_take_on_the_phone_streams_the_whole_take() {
        let mic = PhoneMicrophone::with_backend(Arc::new(FakeBackend::default()));
        let options = CaptureOptions::dictation(&voltip_core::RecordingSettings::default(), false);
        assert!(options.long, "a dictation take may run ten minutes by default");
        let mut capture = mic.start(None, Box::new(|_| {}), Box::new(|| {}), options).unwrap();
        let mut stream = capture.pcm_stream().expect("a long take streams");
        assert!(first_samples(|buf| stream.read(buf)) > 0, "the fake device delivers audio into the stream");
        assert_eq!(stream.gap(), None);
        capture.stop().unwrap();
        assert!(stream.is_closed());
        let mut short = mic.start(None, Box::new(|_| {}), Box::new(|| {}), CaptureOptions::default()).unwrap();
        assert!(short.pcm_stream().is_none());
        assert!(short.live_pcm().is_none());
        short.stop().unwrap();
    }

    /// A device the backend does not have is the core's `Audio` error, not a panic.
    #[test]
    fn an_unknown_device_is_an_audio_error() {
        let mic = PhoneMicrophone::with_backend(Arc::new(FakeBackend::default()));
        let err = mic.start(Some("no-such-device"), Box::new(|_| {}), Box::new(|| {}), CaptureOptions::default()).err().expect("no such device");
        assert!(matches!(err, DictationError::Audio(_)), "{err:?}");
    }
}
