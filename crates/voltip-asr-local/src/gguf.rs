//! The [`RecognizerLoader`] for [`Engine::TranscribeCpp`]: transcribe.cpp (ggml) over a Qwen3-ASR
//! GGUF, through the `transcribe-cpp` crate. The only module that names that binding.
//!
//! Verified 2026-09-25 (docs/dictation.md §10): `transcribe-cpp = "=0.2.3"` with
//! `default-features = false` builds from source with cmake on Linux (39 s) and cross-compiles to
//! `x86_64-pc-windows-msvc` under cargo-xwin, statically — no runtime DLL. `Model::load` then
//! `Model::session()`; `Session::run(&[f32] /* 16 kHz mono */, &RunOptions)` returns the
//! `Transcript`. The `Session` keeps the model resident, so one is held per loaded entry. The
//! Qwen3 family does not take a language hint (`RunOptions.language` stays `None`; the model
//! detects the language itself), and it does not stream. Measured here: Qwen3-ASR-0.6B Q6_K loads
//! in 0.64 s and transcribes 16 s of Chinese in 1.56 s.

use std::path::Path;

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler as _};
use transcribe_cpp::{Backend, DeviceType, Model, ModelOptions, RunOptions, Session, SessionOptions};

use crate::catalogue::ModelEntry;
use crate::compute::{Compute, GpuInfo, LocalDevice};
use crate::transcriber::{Recognizer, RecognizerLoader};

/// The sample rate transcribe.cpp models take.
pub const MODEL_RATE_HZ: u32 = 16_000;

/// The x86_64 extensions ggml's CPU kernels are compiled for (`TRANSCRIBE_CMAKE_ARGS` in
/// `.cargo/config.toml`: the Haswell baseline) that this CPU does not have. Every package is built
/// with them, so a CPU that lacks one would die with an illegal instruction inside the first
/// kernel; the loader checks up front and refuses with a reason instead. Always empty off x86_64,
/// where ggml keeps the compiler's baseline.
pub fn missing_cpu_features() -> Vec<&'static str> {
    #[cfg(target_arch = "x86_64")]
    {
        let checks: [(&str, bool); 6] = [
            ("SSE4.2", std::arch::is_x86_feature_detected!("sse4.2")),
            ("AVX", std::arch::is_x86_feature_detected!("avx")),
            ("AVX2", std::arch::is_x86_feature_detected!("avx2")),
            ("FMA", std::arch::is_x86_feature_detected!("fma")),
            ("F16C", std::arch::is_x86_feature_detected!("f16c")),
            ("BMI2", std::arch::is_x86_feature_detected!("bmi2")),
        ];
        checks.iter().filter(|(_, present)| !present).map(|(name, _)| *name).collect()
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        Vec::new()
    }
}

/// Why `entry` cannot run on a CPU missing `missing` (empty = it can).
pub fn cpu_refusal(entry: &ModelEntry, missing: &[&str]) -> Option<String> {
    (!missing.is_empty()).then(|| {
        format!("此处理器缺少 {}，无法运行本地模型「{}」（需要 2013 年以后的 x86 处理器）；请改用「轻量」模型或云端识别", missing.join(" / "), entry.name)
    })
}

/// The GPUs transcribe.cpp registered in this build (none without a GPU backend).
fn gpu_devices() -> Vec<transcribe_cpp::Device> {
    transcribe_cpp::devices().into_iter().filter(|d| matches!(d.device_type, DeviceType::Gpu | DeviceType::Igpu)).collect()
}

/// [`crate::compute::hardware`]'s GPU list.
pub(crate) fn gpus() -> Vec<GpuInfo> {
    gpu_devices()
        .into_iter()
        .map(|d| GpuInfo {
            integrated: d.device_type == DeviceType::Igpu,
            name: d.name,
            description: d.description,
            kind: d.kind,
            memory_total: d.memory_total,
        })
        .collect()
}

/// The load options for `compute`: `auto` lets the build pick (a GPU backend when it has one, the
/// CPU otherwise), `cpu` is strict CPU, `gpu` names the device — the one called `compute.gpu`, or
/// the first GPU. `None` means no GPU could be found for `gpu`.
fn model_options(compute: &Compute) -> Option<ModelOptions> {
    match compute.device {
        LocalDevice::Auto => Some(ModelOptions::default()),
        LocalDevice::Cpu => Some(ModelOptions { backend: Backend::Cpu, device: None }),
        LocalDevice::Gpu => {
            let gpus = gpu_devices();
            let wanted = compute.gpu.as_deref();
            let device = gpus.iter().find(|d| Some(d.name.as_str()) == wanted).or_else(|| gpus.first()).cloned()?;
            Some(ModelOptions { backend: Backend::Auto, device: Some(device) })
        }
    }
}

const CPU_ONLY: ModelOptions = ModelOptions { backend: Backend::Cpu, device: None };

/// Loads Qwen3-ASR GGUF sessions from an installed model directory.
#[derive(Clone, Copy, Debug, Default)]
pub struct GgufLoader;

impl RecognizerLoader for GgufLoader {
    fn load(&self, entry: &ModelEntry, dir: &Path, _language: Option<&str>, compute: &Compute) -> Result<Box<dyn Recognizer>, String> {
        if let Some(reason) = cpu_refusal(entry, &missing_cpu_features()) {
            return Err(reason);
        }
        let Some(file) = entry.files().first() else { return Err(format!("{}: catalogue entry has no file", entry.id)) };
        let path = dir.join(file.name);
        let options = model_options(compute).unwrap_or_else(|| {
            tracing::warn!(model = entry.id, gpu = ?compute.gpu, "no GPU this build can drive; running on the CPU");
            CPU_ONLY
        });
        let model = match Model::load_with(&path, &options) {
            Ok(model) => model,
            // A GPU that does not initialise (driver, memory) must not take the recogniser with it.
            Err(e) if options != CPU_ONLY => {
                tracing::warn!(model = entry.id, error = %e, "GPU load failed; running on the CPU");
                Model::load_with(&path, &CPU_ONLY).map_err(|e| format!("transcribe.cpp could not load {}: {e}", entry.id))?
            }
            Err(e) => return Err(format!("transcribe.cpp could not load {}: {e}", entry.id)),
        };
        // No thread setting: the library's own default. ggml decoding scales past sherpa's cap of
        // 4; measured on a 32-core host, 16 s Chinese sample, 0.6B Q6_K: 4 threads 2.67 s, library
        // default 1.60 s, 16 threads 1.07 s.
        let n_threads = compute.threads.map_or(0, |t| i32::try_from(t).unwrap_or(i32::MAX));
        let session_options = SessionOptions { n_threads, ..SessionOptions::default() };
        let session = model.session_with(&session_options).map_err(|e| format!("transcribe.cpp could not open a session for {}: {e}", entry.id))?;
        let backend = model.backend();
        tracing::info!(model = entry.id, arch = %model.arch(), variant = %model.variant(), %backend, n_threads, "gguf model loaded");
        Ok(Box::new(Gguf { session, backend }))
    }
}

struct Gguf {
    session: Session,
    /// `Model::backend()` at load: `CPU`, `Metal`, `Vulkan0`, … .
    backend: String,
}

impl Recognizer for Gguf {
    fn transcribe(&mut self, sample_rate: u32, samples: &[f32]) -> Result<String, String> {
        let resampled;
        let pcm = if sample_rate == MODEL_RATE_HZ {
            samples
        } else {
            resampled = resample_to_model_rate(samples, sample_rate)?;
            &resampled
        };
        // No language hint: unsupported by the Qwen3 family (the call would be refused).
        let transcript = self.session.run(pcm, &RunOptions::default()).map_err(|e| e.to_string())?;
        tracing::debug!(language = ?transcript.language, "gguf transcription done");
        Ok(transcript.text)
    }

    fn backend(&self) -> String {
        self.backend.clone()
    }
}

/// Whole-take resample to [`MODEL_RATE_HZ`] (rubato FFT, delay trimmed). The recorder already
/// produces 16 kHz, so this only runs for foreign WAV files.
pub fn resample_to_model_rate(samples: &[f32], from_hz: u32) -> Result<Vec<f32>, String> {
    if from_hz == MODEL_RATE_HZ {
        return Ok(samples.to_vec());
    }
    if samples.is_empty() || from_hz == 0 {
        return Err(format!("cannot resample {} samples at {from_hz} Hz", samples.len()));
    }
    let mut resampler =
        Fft::<f32>::new(from_hz as usize, MODEL_RATE_HZ as usize, 1024, 1, FixedSync::Input).map_err(|e| format!("resample {from_hz} Hz -> 16000 Hz: {e}"))?;
    let adapter = InterleavedSlice::new(samples, 1, samples.len()).map_err(|e| e.to_string())?;
    let out = resampler.process_all(&adapter, samples.len(), None).map_err(|e| e.to_string())?;
    Ok(out.take_data())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampling_to_the_model_rate_keeps_length_and_passes_16k_through() {
        let input: Vec<f32> = (0..48_000).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
        let out = resample_to_model_rate(&input, 48_000).unwrap();
        assert_eq!(out.len(), 16_000);
        assert!(out.iter().all(|s| s.abs() <= 0.6));
        assert_eq!(resample_to_model_rate(&input[..10], 16_000).unwrap(), &input[..10]);
        assert!(resample_to_model_rate(&[], 48_000).is_err());
        assert!(resample_to_model_rate(&input[..10], 0).is_err());
        assert_eq!(MODEL_RATE_HZ, 16_000);
        assert!(format!("{GgufLoader:?}").contains("GgufLoader"));
    }

    /// Regression (2026-09-26): the packages are built for the Haswell baseline, so a CPU without
    /// AVX2 / FMA / … must get a reason, never reach ggml (illegal instruction, the whole app gone).
    /// This host has them all; the refusal itself is checked on a synthetic list.
    #[test]
    fn regression_a_cpu_below_the_build_baseline_is_refused_with_a_reason() {
        let entry = crate::catalogue::entry("qwen3-asr-0.6b").unwrap();
        assert_eq!(cpu_refusal(entry, &[]), None);
        let reason = cpu_refusal(entry, &["AVX2", "FMA"]).unwrap();
        assert!(reason.contains("AVX2 / FMA") && reason.contains(entry.name) && reason.contains("轻量"), "{reason}");
        #[cfg(target_arch = "x86_64")]
        assert!(missing_cpu_features().is_empty(), "the test host is below the baseline the packages need: {:?}", missing_cpu_features());
    }

    /// A missing file is a load error with the entry's id, not a panic (no model needed).
    #[test]
    fn loading_from_an_empty_directory_fails_with_the_entry_name() {
        let dir = tempfile::tempdir().unwrap();
        let entry = crate::catalogue::entry("qwen3-asr-0.6b").unwrap();
        let Err(err) = GgufLoader.load(entry, dir.path(), None, &Compute::default()) else { panic!("loaded a model from an empty directory") };
        assert!(err.contains("qwen3-asr-0.6b"), "{err}");
    }
}
