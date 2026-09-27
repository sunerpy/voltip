//! Where a local recogniser runs (docs/dictation.md §10.6): the device choice, the GPU and the
//! inference threads from `EngineSettings.local_device / local_gpu / local_threads`, and the
//! compute devices this build can drive ([`hardware`]).
//!
//! Only the transcribe.cpp tiers (Qwen3-ASR GGUF) run on a GPU, and only in a build with a GPU
//! backend: Metal on macOS, Vulkan where the `vulkan` feature is on. sherpa-onnx (SenseVoice,
//! Paraformer, the streaming preview) is CPU only; it takes the thread count. A GPU that is asked
//! for but absent, or that fails to initialise, falls back to the CPU and says so in the log.

pub use voltip_core::LocalDevice;

/// The compute choice a recogniser is loaded with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Compute {
    /// Auto (the fastest device this build can drive), CPU only, or a GPU.
    pub device: LocalDevice,
    /// The GPU `device = gpu` asks for, by [`GpuInfo::name`]; `None` = the first one.
    pub gpu: Option<String>,
    /// Inference threads; `None` = the engine's own default.
    pub threads: Option<usize>,
}

impl Compute {
    /// Threads for a CPU engine whose own default is `default`.
    pub fn threads_or(&self, default: usize) -> usize {
        self.threads.unwrap_or(default).max(1)
    }
}

/// A GPU this build can run the transcribe.cpp tiers on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpuInfo {
    /// Backend device name (`Metal`, `Vulkan0`): what `EngineSettings.local_gpu` stores.
    pub name: String,
    /// What the driver calls it (`Apple M4 Max`, `NVIDIA L40S`).
    pub description: String,
    /// Backend family: `metal`, `vulkan`, `cuda`, … .
    pub kind: String,
    /// Device memory in bytes (`0` when the backend does not report it).
    pub memory_total: u64,
    /// An integrated GPU (shares system memory).
    pub integrated: bool,
}

/// The machine as the local engines see it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HardwareInfo {
    /// Logical CPUs.
    pub cpu_threads: usize,
    /// GPUs this build has a backend for (empty in a CPU-only build).
    pub gpus: Vec<GpuInfo>,
}

/// Enumerate the machine (blocking: the first call initialises the build's GPU backends).
pub fn hardware() -> HardwareInfo {
    HardwareInfo { cpu_threads: std::thread::available_parallelism().map_or(1, std::num::NonZero::get), gpus: crate::gguf::gpus() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threads_fall_back_to_the_engine_default_and_never_reach_zero() {
        assert_eq!(Compute::default().threads_or(4), 4);
        assert_eq!(Compute { threads: Some(12), ..Compute::default() }.threads_or(4), 12);
        assert_eq!(Compute { threads: Some(0), ..Compute::default() }.threads_or(4), 1);
        let machine = hardware();
        assert!(machine.cpu_threads >= 1);
        // The test build has no GPU backend on Linux / Windows; every GPU it reports is a real one.
        assert!(machine.gpus.iter().all(|g| !g.name.is_empty()));
    }
}
