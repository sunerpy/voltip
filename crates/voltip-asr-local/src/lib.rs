//! Offline speech recognition for the dictation pipeline (docs/dictation.md §10, §11).
//!
//! * [`catalogue`] — the static table of downloadable models (id, engine, tier, capabilities, HF
//!   repo, file sizes and sha256), shipped with the binary: two Qwen3-ASR GGUF tiers, two int8
//!   ONNX offline models, one streaming Zipformer.
//! * [`store`] — [`ModelStore`]: the on-disk library under `<app data dir>/models`: scan, resumable
//!   download with sha256 verification and `manifest.json`, removal. Implements
//!   [`voltip_core::ModelManager`] so the core can drive it.
//! * [`transcriber`] — [`LocalTranscriber`]: [`voltip_core::dictation::Transcriber`] over a local
//!   recogniser, loaded once and kept, inference on a blocking thread. The engines sit behind
//!   [`transcriber::RecognizerLoader`]: transcribe.cpp for the GGUF tiers (`gguf.rs`), sherpa-onnx
//!   for SenseVoice / Paraformer (`sherpa.rs`); the unit tests run on a fake without a model file,
//!   the real inference tests need `VOLTIP_LOCAL_MODEL_DIR` / `VOLTIP_LOCAL_GGUF`.
//! * [`streaming`] — [`LocalStreamingTranscriber`]: [`voltip_core::dictation::StreamingTranscriber`]
//!   over the streaming Zipformer (sherpa-onnx `OnlineRecognizer`) for the live preview.
//! * [`vad`] — [`VadTrimmer`]: the Silero VAD (sherpa-onnx `VoiceActivityDetector`, catalogue
//!   entry `silero-vad`, an auxiliary entry the UI never shows) that cuts a take to its speech
//!   before a local whole-take transcription when `vad_trim` is on (docs/dictation.md §12).
//!
//! Nothing here knows a production hostname: the optional first-choice mirror comes from the
//! build-time `VOLTIP_MODEL_BASE_URL`; the public sources are huggingface.co and hf-mirror.com.
//! Desktop only: the mobile shell does not depend on this crate.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod catalogue;
pub mod compute;
mod gguf;
mod sherpa;
pub mod store;
pub mod streaming;
pub mod transcriber;
pub mod vad;

pub use catalogue::{
    CATALOGUE, Capability, DEFAULT_MODEL_ID, Engine, ModelEntry, ModelFile, STREAMING_MODEL_ID, Tier, VAD_MODEL_ID, entry, streaming_entry, vad_entry,
};
pub use compute::{Compute, GpuInfo, HardwareInfo, LocalDevice, hardware};
pub use store::{CATALOGUE_VERSION, DISK_HEADROOM, FreeSpace, MANIFEST_FILE, Manifest, ModelStore, SlowSourcePolicy, Source, StoreError, bytes_to_fetch};
pub use streaming::{LocalStreamingTranscriber, StreamingLoader, StreamingRecognizer};
pub use transcriber::{DefaultLoader, LocalTranscriber, MIN_INPUT, Recognizer, RecognizerLoader, decode_wav};
pub use vad::{SpeechSpan, Trim, VadLoader, VadTrimmer, VoiceActivity};
