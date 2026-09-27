//! The real sherpa-onnx loaders through the official `sherpa-onnx` crate (dynamic
//! `sherpa-onnx-c-api` + ONNX Runtime, CPU provider): [`SherpaLoader`] for the offline SenseVoice /
//! Paraformer recognisers (docs/dictation.md §10), [`SherpaStreamingLoader`] for the streaming
//! Zipformer transducer behind the live preview (§11) and [`SherpaVadLoader`] for the Silero voice
//! activity detector behind `vad_trim` (§12). The only module that names the binding.

use std::path::Path;
use std::sync::Arc;

use sherpa_onnx::{
    OfflineParaformerModelConfig, OfflineRecognizer, OfflineRecognizerConfig, OfflineSenseVoiceModelConfig, OnlineRecognizer, OnlineRecognizerConfig,
    OnlineStream, OnlineTransducerModelConfig, SileroVadModelConfig, VadModelConfig, VoiceActivityDetector,
};
use voltip_core::dictation::{DictationError, LIVE_SAMPLE_RATE_HZ, Segment, StreamEvent, StreamFinal, StreamingSession};

use crate::catalogue::{Engine, ModelEntry};
use crate::compute::Compute;
use crate::streaming::{StreamingLoader, StreamingRecognizer};
use crate::transcriber::{Recognizer, RecognizerLoader, default_threads};
use crate::vad::{MAX_SPEECH, MIN_SILENCE, MIN_SPEECH, SpeechSpan, THRESHOLD, VAD_SAMPLE_RATE_HZ, VadLoader, VoiceActivity, WINDOW_SAMPLES};

/// Loads SenseVoice / Paraformer recognisers from an installed model directory.
#[derive(Clone, Copy, Debug, Default)]
pub struct SherpaLoader;

impl RecognizerLoader for SherpaLoader {
    fn load(&self, entry: &ModelEntry, dir: &Path, language: Option<&str>, compute: &Compute) -> Result<Box<dyn Recognizer>, String> {
        // sherpa-onnx runs these on the CPU only; the device choice does not apply, the threads do.
        let threads = compute.threads_or(default_threads());
        let file = |name: &str| {
            entry.file(name).map(|f| dir.join(f.name).to_string_lossy().into_owned()).ok_or_else(|| format!("{}: no {name} in the catalogue", entry.id))
        };
        let model = file("model.int8.onnx")?;
        let tokens = file("tokens.txt")?;
        let mut config = OfflineRecognizerConfig::default();
        config.model_config.tokens = Some(tokens);
        config.model_config.num_threads = i32::try_from(threads).unwrap_or(1).max(1);
        config.model_config.provider = Some("cpu".to_owned());
        match entry.engine {
            Engine::SenseVoice => {
                config.model_config.sense_voice =
                    OfflineSenseVoiceModelConfig { model: Some(model), language: Some(language.unwrap_or("auto").to_owned()), use_itn: true };
            }
            Engine::Paraformer => {
                config.model_config.paraformer = OfflineParaformerModelConfig { model: Some(model) };
            }
            Engine::TranscribeCpp | Engine::ZipformerStreaming | Engine::SileroVad => return Err(format!("{}: not a sherpa-onnx offline model", entry.id)),
        }
        // `create` returns `None` when the model files cannot be loaded (bad file, unsupported
        // opset, out of memory); sherpa-onnx logs the reason itself.
        let recognizer = OfflineRecognizer::create(&config).ok_or_else(|| format!("sherpa-onnx could not load {}", entry.id))?;
        Ok(Box::new(Offline(recognizer)))
    }
}

struct Offline(OfflineRecognizer);

impl Recognizer for Offline {
    fn transcribe(&mut self, sample_rate: u32, samples: &[f32]) -> Result<String, String> {
        let stream = self.0.create_stream();
        stream.accept_waveform(i32::try_from(sample_rate).map_err(|_| "sample rate out of range".to_owned())?, samples);
        self.0.decode(&stream);
        stream.get_result().map(|r| r.text).ok_or_else(|| "sherpa-onnx returned no result".to_owned())
    }
}

/// Endpoint rule 1: seconds of trailing silence that end an utterance before any token was
/// decoded (a pause before the first word).
pub const RULE1_TRAILING_SILENCE_S: f32 = 2.0;
/// Endpoint rule 2: seconds of trailing silence after tokens were decoded — the dictation rhythm's
/// sentence boundary.
pub const RULE2_TRAILING_SILENCE_S: f32 = 0.8;
/// Endpoint rule 3: an utterance this long (seconds) is committed regardless of silence.
pub const RULE3_UTTERANCE_S: f32 = 20.0;

/// Loads the streaming Zipformer transducer (`encoder` / `decoder` / `joiner` + `tokens`) as an
/// `OnlineRecognizer` with endpoint detection and greedy search.
#[derive(Clone, Copy, Debug, Default)]
pub struct SherpaStreamingLoader;

impl StreamingLoader for SherpaStreamingLoader {
    fn load(&self, entry: &ModelEntry, dir: &Path, threads: usize) -> Result<Arc<dyn StreamingRecognizer>, String> {
        if entry.engine != Engine::ZipformerStreaming {
            return Err(format!("{}: not a streaming model", entry.id));
        }
        let file = |name: &str| {
            entry.file(name).map(|f| dir.join(f.name).to_string_lossy().into_owned()).ok_or_else(|| format!("{}: no {name} in the catalogue", entry.id))
        };
        let mut config = OnlineRecognizerConfig {
            decoding_method: Some("greedy_search".to_owned()),
            enable_endpoint: true,
            rule1_min_trailing_silence: RULE1_TRAILING_SILENCE_S,
            rule2_min_trailing_silence: RULE2_TRAILING_SILENCE_S,
            rule3_min_utterance_length: RULE3_UTTERANCE_S,
            ..OnlineRecognizerConfig::default()
        };
        config.model_config.transducer = OnlineTransducerModelConfig {
            encoder: Some(file("encoder.int8.onnx")?),
            decoder: Some(file("decoder.onnx")?),
            joiner: Some(file("joiner.int8.onnx")?),
        };
        config.model_config.tokens = Some(file("tokens.txt")?);
        config.model_config.num_threads = i32::try_from(threads).unwrap_or(1).max(1);
        config.model_config.provider = Some("cpu".to_owned());
        let recognizer = OnlineRecognizer::create(&config).ok_or_else(|| format!("sherpa-onnx could not load {}", entry.id))?;
        Ok(Arc::new(Online { recognizer }))
    }
}

struct Online {
    recognizer: OnlineRecognizer,
}

impl StreamingRecognizer for Online {
    fn open(self: Arc<Self>) -> Box<dyn StreamingSession> {
        let stream = self.recognizer.create_stream();
        Box::new(OnlineSession { shared: self, stream, fed: 0, sentence_start_ms: 0, committed: Vec::new(), last_partial: String::new() })
    }
}

/// One recording's stream: `feed` accepts samples and decodes what is ready; `poll` reports an
/// endpoint (commit + reset) or a changed partial; `finish` flushes.
struct OnlineSession {
    shared: Arc<Online>,
    stream: OnlineStream,
    /// Samples accepted so far (stream time).
    fed: u64,
    sentence_start_ms: u64,
    committed: Vec<Segment>,
    last_partial: String,
}

impl OnlineSession {
    fn fed_ms(&self) -> u64 {
        self.fed * 1000 / u64::from(LIVE_SAMPLE_RATE_HZ)
    }

    fn decode_ready(&self) {
        let r = &self.shared.recognizer;
        while r.is_ready(&self.stream) {
            r.decode(&self.stream);
        }
    }

    fn text(&self) -> String {
        self.shared.recognizer.get_result(&self.stream).map(|r| r.text.trim().to_owned()).unwrap_or_default()
    }
}

impl StreamingSession for OnlineSession {
    fn feed(&mut self, pcm16k: &[f32]) {
        // `LIVE_SAMPLE_RATE_HZ` is 16 000: it fits an i32.
        self.stream.accept_waveform(LIVE_SAMPLE_RATE_HZ as i32, pcm16k);
        self.fed += pcm16k.len() as u64;
        self.decode_ready();
    }

    fn poll(&mut self) -> StreamEvent {
        let r = &self.shared.recognizer;
        if r.is_endpoint(&self.stream) {
            let text = self.text();
            r.reset(&self.stream);
            let (start_ms, end_ms) = (self.sentence_start_ms, self.fed_ms());
            self.sentence_start_ms = end_ms;
            self.last_partial.clear();
            if text.is_empty() {
                return StreamEvent::Idle;
            }
            self.committed.push(Segment { text: text.clone(), start_ms, end_ms });
            return StreamEvent::Endpoint { text, start_ms, end_ms };
        }
        let text = self.text();
        if text == self.last_partial {
            return StreamEvent::Idle;
        }
        self.last_partial = text.clone();
        StreamEvent::Partial { current: text }
    }

    fn finish(mut self: Box<Self>) -> Result<StreamFinal, DictationError> {
        self.stream.input_finished();
        self.decode_ready();
        let tail = self.text();
        Ok(StreamFinal { committed: std::mem::take(&mut self.committed), tail })
    }
}

/// Seconds of audio the detector's ring buffer holds: one speech segment of [`MAX_SPEECH`] plus
/// the silence that closes it, with room to spare (segments are popped as they come).
pub const VAD_BUFFER_SECONDS: f32 = 30.0;

/// Loads the Silero VAD (`silero_vad.onnx`) as a `VoiceActivityDetector` with the docs/dictation.md
/// §12 parameters.
#[derive(Clone, Copy, Debug, Default)]
pub struct SherpaVadLoader;

impl VadLoader for SherpaVadLoader {
    fn load(&self, entry: &ModelEntry, dir: &Path) -> Result<Box<dyn VoiceActivity>, String> {
        if entry.engine != Engine::SileroVad {
            return Err(format!("{}: not a VAD model", entry.id));
        }
        let model = entry
            .file("silero_vad.onnx")
            .map(|f| dir.join(f.name).to_string_lossy().into_owned())
            .ok_or_else(|| format!("{}: no silero_vad.onnx in the catalogue", entry.id))?;
        let config = VadModelConfig {
            silero_vad: SileroVadModelConfig {
                model: Some(model),
                threshold: THRESHOLD,
                min_silence_duration: MIN_SILENCE.as_secs_f32(),
                min_speech_duration: MIN_SPEECH.as_secs_f32(),
                window_size: i32::try_from(WINDOW_SAMPLES).unwrap_or(512),
                max_speech_duration: MAX_SPEECH.as_secs_f32(),
            },
            sample_rate: i32::try_from(VAD_SAMPLE_RATE_HZ).unwrap_or(16_000),
            num_threads: 1,
            provider: Some("cpu".to_owned()),
            ..VadModelConfig::default()
        };
        let vad = VoiceActivityDetector::create(&config, VAD_BUFFER_SECONDS).ok_or_else(|| format!("sherpa-onnx could not load {}", entry.id))?;
        Ok(Box::new(Silero(vad)))
    }
}

struct Silero(VoiceActivityDetector);

impl Silero {
    fn drain(&self, spans: &mut Vec<SpeechSpan>) {
        while let Some(segment) = self.0.front() {
            let start = usize::try_from(segment.start()).unwrap_or(0);
            let n = usize::try_from(segment.n()).unwrap_or(0);
            spans.push(SpeechSpan { start, end: start + n });
            drop(segment);
            self.0.pop();
        }
    }
}

impl VoiceActivity for Silero {
    fn spans(&mut self, pcm16k: &[f32]) -> Result<Vec<SpeechSpan>, String> {
        self.0.reset();
        let mut spans = Vec::new();
        for window in pcm16k.chunks(WINDOW_SAMPLES) {
            self.0.accept_waveform(window);
            self.drain(&mut spans);
        }
        self.0.flush();
        self.drain(&mut spans);
        Ok(spans)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_rules_follow_the_dictation_rhythm() {
        assert_eq!(RULE1_TRAILING_SILENCE_S, 2.0);
        assert_eq!(RULE2_TRAILING_SILENCE_S, 0.8);
        assert_eq!(RULE3_UTTERANCE_S, 20.0);
        assert!(format!("{SherpaLoader:?}").contains("SherpaLoader") && format!("{SherpaStreamingLoader:?}").contains("Streaming"));
        assert!(format!("{SherpaVadLoader:?}").contains("Vad"));
        assert!(VAD_BUFFER_SECONDS > MAX_SPEECH.as_secs_f32() + MIN_SILENCE.as_secs_f32());
    }

    /// Wrong families are refused before any file is touched.
    #[test]
    fn loaders_refuse_the_other_families() {
        let dir = tempfile::tempdir().unwrap();
        let gguf = crate::catalogue::entry("qwen3-asr-0.6b").unwrap();
        let Err(err) = SherpaLoader.load(gguf, dir.path(), None, &Compute::default()) else { panic!("loaded a gguf with sherpa") };
        assert!(err.contains("no model.int8.onnx"), "{err}");
        assert_eq!(SherpaStreamingLoader.load(gguf, dir.path(), 1).err(), Some("qwen3-asr-0.6b: not a streaming model".into()));
        let streaming = crate::catalogue::streaming_entry();
        let Err(err) = SherpaLoader.load(streaming, dir.path(), None, &Compute::default()) else { panic!("loaded a streaming model offline") };
        assert!(err.contains("no model.int8.onnx"), "{err}");
        let vad = crate::catalogue::vad_entry();
        assert_eq!(SherpaLoader.load(vad, dir.path(), None, &Compute::default()).err(), Some("silero-vad: no model.int8.onnx in the catalogue".into()));
        assert_eq!(SherpaVadLoader.load(gguf, dir.path()).err(), Some("qwen3-asr-0.6b: not a VAD model".into()));
        // The right entry with no file on disk: sherpa-onnx refuses to load, no panic.
        let Err(err) = SherpaVadLoader.load(vad, dir.path()) else { panic!("loaded a VAD from an empty directory") };
        assert!(err.contains("silero-vad"), "{err}");
    }
}
