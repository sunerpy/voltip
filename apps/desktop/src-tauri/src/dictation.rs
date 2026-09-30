//! The real dictation ports (docs/dictation.md §1) the desktop plugs into the core: cpal
//! capture through `voltip-audio` (with the live 16 kHz tap of §11), HTTP speech-to-text through
//! `voltip-asr` or offline speech-to-text through `voltip-asr-local` (docs/dictation.md §10), the
//! streaming preview through `voltip-asr-local`'s Zipformer (§11), the Silero VAD trim in front
//! of the local recogniser (§12 `vad_trim`), HTTP clean-up through `voltip-refine`, clipboard +
//! paste through `voltip-inject`, and the foreground-application probe of §18 (Win32 / X11 /
//! AppKit, `crate::platform::PlatformProbe`) — and, for voice edit (§19), the selection copy
//! through the same inject crate and the rewrite through the same `RefineClient`. The core calls
//! [`AudioSource::start`], [`Capture::stop`], [`Injector::inject`], [`Injector::copy_selection`],
//! the probe and the streaming session on blocking threads. The output mode (§12) and the scenes
//! (§18) are the core's business: the shell only passes the recording cap the core asks for to the
//! recorder and the hints it builds to the refiner.

use std::path::PathBuf;
use std::sync::Arc;

use crate::audio::AudioHub;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use voltip_asr::{AsrClient, AsrConfig};
use voltip_asr_local::{LocalStreamingTranscriber, LocalTranscriber, ModelStore, VadSegmenterFactory};
use voltip_audio::{Backend, CaptureSource, CpalBackend, LiveConsumer, LiveTapConfig, PcmConsumer, PcmStreamConfig, Recorder, RecorderConfig};
use voltip_core::dictation::{
    AudioSource, Capture, CaptureOptions, ClipboardCode, DictationError, DictationPorts, EngineFactory, InjectNote, Injection, Injector, LevelFrame, LivePcm,
    MAX_RECORDING, PcmStream, Recording, RefineHints, Refined, Refiner, SelectionTiming, ServiceProbe, Transcriber, Transcript, Via,
};
use voltip_core::{BuiltinPreset, InjectMode, Modifier, ProbeError, ProbeFailure, ProviderId, RecordingSource, ResolvedEngines, ServiceKind, TakePreset};
use voltip_inject::{ClipboardOnlyInjector, CopyOptions, FallbackCode, PasteOptions, SelectionSource};
use voltip_platform::{HostOs, InjectDecision, InjectPreflight};
use voltip_refine::{PromptContext, PromptHints, RefineClient, RefineConfig};

/// A level frame as the core broadcasts it, from the audio crate's meter frame (same fields).
pub fn to_core_frame(f: voltip_audio::LevelFrame) -> LevelFrame {
    LevelFrame { rms_dbfs: f.rms_dbfs, peak_dbfs: f.peak_dbfs, clipping: f.clipping, sample_rate_hz: f.sample_rate_hz, channels: f.channels, seq: f.seq }
}

/// The reverse: what the webview's meter channel carries.
pub fn to_audio_frame(f: LevelFrame) -> voltip_audio::LevelFrame {
    voltip_audio::LevelFrame {
        rms_dbfs: f.rms_dbfs,
        peak_dbfs: f.peak_dbfs,
        clipping: f.clipping,
        sample_rate_hz: f.sample_rate_hz,
        channels: f.channels,
        seq: f.seq,
    }
}

/// Capture through [`voltip_audio::Recorder`] (mono 16 kHz WAV, the core's cap, 30 Hz levels): the
/// microphone, the computer's sound or both (docs/dictation.md §22). A long take keeps its first
/// [`MAX_RECORDING`] in memory and streams the whole take to the core.
/// Coordinates with the [`AudioHub`]: the hub releases its device meter before the recorder opens
/// the microphone and takes the recorder's level frames meanwhile, so the webview meters keep
/// moving and the device is never opened twice.
pub struct RecorderAudioSource {
    backend: Arc<dyn Backend + Send + Sync>,
    hub: Arc<AudioHub>,
}

impl RecorderAudioSource {
    /// The platform sound system (WASAPI / CoreAudio / ALSA).
    pub fn cpal(hub: Arc<AudioHub>) -> Self {
        Self::with_backend(Arc::new(CpalBackend::new()), hub)
    }

    /// Any backend (tests use `voltip_audio::FakeBackend`).
    pub fn with_backend(backend: Arc<dyn Backend + Send + Sync>, hub: Arc<AudioHub>) -> Self {
        Self { backend, hub }
    }
}

impl AudioSource for RecorderAudioSource {
    fn start(
        &self,
        device_id: Option<&str>,
        on_level: Box<dyn Fn(LevelFrame) + Send>,
        on_ready: Box<dyn FnOnce() + Send>,
        options: CaptureOptions,
    ) -> Result<Box<dyn Capture>, DictationError> {
        let live = options.live;
        let backend = self.backend.as_ref();
        let output_id = || output_or_default(backend, options.output_device.as_deref());
        let source = match options.source {
            RecordingSource::Microphone => CaptureSource::Microphone,
            RecordingSource::System => CaptureSource::System { output_id: output_id() },
            RecordingSource::Mixed => CaptureSource::Mixed { output_id: output_id() },
        };
        let config = RecorderConfig {
            device_id: if options.source.uses_microphone() { connected_or_default(backend, device_id) } else { None },
            live_tap: live.then(LiveTapConfig::default),
            // A long take (§22) keeps its first two minutes in memory; the core writes the whole
            // take to its recording file from the stream.
            max_duration: if options.long { options.max_duration.min(MAX_RECORDING) } else { options.max_duration },
            source,
            pcm_stream: options.long.then(PcmStreamConfig::default),
            ..RecorderConfig::default()
        };
        // Take the microphone from the level meter first; the hub fans our frames out from now on.
        self.hub.enter_capture();
        let hub = self.hub.clone();
        let recorder = Recorder::start_with_ready(
            self.backend.as_ref(),
            config,
            move |frame| {
                hub.push(frame);
                on_level(to_core_frame(frame));
            },
            on_ready,
        );
        let recorder = match recorder {
            Ok(r) => r,
            Err(e) => {
                // Give the microphone back to the meter subscribers.
                self.hub.leave_capture();
                return Err(DictationError::Audio(e.to_string()));
            }
        };
        tracing::info!(
            device = %recorder.device().name,
            output = ?recorder.output_device().map(|d| d.name.as_str()),
            source = options.source.as_str(),
            live,
            long = options.long,
            max_duration = ?options.max_duration,
            "dictation capture started"
        );
        Ok(Box::new(RecorderCapture { recorder, hub: self.hub.clone() }))
    }
}

/// The device a take records from: the chosen one while it is connected, the system default once
/// it is not (unplugged, or a settings file from another machine). An enumeration that fails
/// leaves the choice to the recorder, which reports the real error.
pub fn connected_or_default(backend: &dyn Backend, device_id: Option<&str>) -> Option<String> {
    let id = device_id?;
    match backend.input_devices() {
        Ok(devices) if !devices.iter().any(|d| d.id == id) => {
            tracing::warn!(device = %id, "the chosen microphone is not connected; recording from the default input");
            None
        }
        _ => Some(id.to_owned()),
    }
}

/// The output device a take records the computer's sound from (docs/dictation.md §22): the chosen
/// one while it is there, the system default output once it is not.
pub fn output_or_default(backend: &dyn Backend, device_id: Option<&str>) -> Option<String> {
    let id = device_id?;
    match backend.output_devices() {
        Ok(devices) if !devices.iter().any(|d| d.id == id) => {
            tracing::warn!(device = %id, "the chosen output device is not connected; recording the default output");
            None
        }
        _ => Some(id.to_owned()),
    }
}

struct RecorderCapture {
    recorder: Recorder,
    hub: Arc<AudioHub>,
}

impl Capture for RecorderCapture {
    fn stop(self: Box<Self>) -> Result<Recording, DictationError> {
        let result = self.recorder.stop().map_err(|e| DictationError::Audio(e.to_string()));
        // The device is free again whatever happened to the recording.
        self.hub.leave_capture();
        let recording = result?;
        tracing::info!(duration_ms = recording.duration_ms, peak_dbfs = recording.peak_dbfs, truncated = recording.truncated, "dictation capture stopped");
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
pub struct TakeStream(pub PcmConsumer);

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

/// The recorder's rtrb consumer as the core's [`LivePcm`] (the core never sees rtrb).
pub struct LiveTap(pub LiveConsumer);

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

/// Speech-to-text through [`voltip_asr::AsrClient`].
pub struct HttpTranscriber {
    client: AsrClient,
}

impl HttpTranscriber {
    /// Build the client; fails on an unusable configuration (bad URL, empty model).
    pub fn new(config: AsrConfig) -> Result<Self, DictationError> {
        Ok(Self { client: AsrClient::new(config).map_err(|e| DictationError::Asr(e.to_string()))? })
    }
}

#[async_trait]
impl Transcriber for HttpTranscriber {
    /// The glossary goes out as the OpenAI `prompt` field when there is one (docs/dictation.md §16.3).
    async fn transcribe(&self, wav: &[u8], language: Option<&str>, glossary: &[String]) -> Result<Transcript, DictationError> {
        let prompt = voltip_core::vocabulary::glossary_prompt(glossary);
        let t = self.client.transcribe_with_prompt(wav, language, prompt.as_deref()).await.map_err(|e| DictationError::Asr(e.to_string()))?;
        Ok(Transcript { text: t.text, latency_ms: t.latency_ms })
    }
}

/// Clean-up through [`voltip_refine::RefineClient`].
pub struct HttpRefiner {
    client: RefineClient,
}

impl HttpRefiner {
    /// Build the client (the take's language, style and context arrive with every request).
    pub fn new(config: RefineConfig) -> Result<Self, DictationError> {
        Ok(Self { client: RefineClient::new(config).map_err(|e| DictationError::Refine(e.to_string()))? })
    }
}

/// The take's preset as the refine crate names it (docs/dictation.md §21).
pub fn refine_preset(preset: &TakePreset) -> voltip_refine::Preset<'_> {
    use voltip_refine::Preset;
    match preset {
        TakePreset::Builtin(builtin) => match builtin {
            BuiltinPreset::Proofread => Preset::Proofread,
            BuiltinPreset::Prompt => Preset::Prompt,
            BuiltinPreset::Intent => Preset::Intent,
            BuiltinPreset::Chat => Preset::Chat,
            BuiltinPreset::Translate => Preset::Translate,
            BuiltinPreset::Notes => Preset::Notes,
            BuiltinPreset::Punctuation => Preset::Punctuation,
            BuiltinPreset::Formal => Preset::Formal,
        },
        TakePreset::Custom { prompt, .. } => Preset::Custom(prompt),
    }
}

/// The built-in preset's own text (task, rules, examples; the output contract is added to every
/// preset): what 复制为自定义 starts from.
pub fn builtin_preset_body(preset: BuiltinPreset) -> &'static str {
    refine_preset(&TakePreset::Builtin(preset)).builtin_body().unwrap_or_default()
}

/// One built-in preset's own text as `presets_builtin` answers it.
#[derive(Clone, Debug, serde::Serialize)]
pub struct BuiltinPresetText {
    /// Which preset.
    pub id: BuiltinPreset,
    /// Its task, rules and examples ([`builtin_preset_body`]).
    pub prompt: &'static str,
}

/// Every built-in preset's text, in the order the interface lists them (`presets_builtin`; the
/// preview serves the same list from `packages/shared/src/fixtures/ipc/presets-builtin.json`).
pub fn builtin_preset_texts() -> Vec<BuiltinPresetText> {
    BuiltinPreset::ALL.into_iter().map(|id| BuiltinPresetText { id, prompt: builtin_preset_body(id) }).collect()
}

/// The core's hints as the refine crate's prompt input, one-to-one (the core already filtered the
/// context by the privacy switches, docs/dictation.md §18.5).
pub fn prompt_hints(hints: &RefineHints) -> PromptHints<'_> {
    PromptHints {
        preset: refine_preset(&hints.preset),
        language: hints.language.as_deref(),
        glossary: &hints.glossary,
        context: PromptContext {
            app_name: hints.context.app_name.as_deref(),
            window_title: hints.context.window_title.as_deref(),
            instruction: hints.context.instruction.as_deref(),
        },
    }
}

#[async_trait]
impl Refiner for HttpRefiner {
    /// The glossary joins the system prompt as the user-dictionary block (docs/dictation.md §16.3),
    /// the take's context as the scene blocks (§18.5); the take's language and style shape the rest.
    async fn refine(&self, text: &str, hints: &RefineHints) -> Result<Refined, DictationError> {
        let r = self.client.refine_with(text, &prompt_hints(hints)).await.map_err(|e| DictationError::Refine(e.to_string()))?;
        Ok(Refined { text: r.text, latency_ms: r.latency_ms, model: r.model })
    }

    /// The voice edit's rewrite (docs/dictation.md §19) through the same client and the same hints
    /// (the edit prompt uses the glossary and the app block); a cut-off or empty answer is an
    /// error, so nothing is pasted.
    async fn edit(&self, selection: &str, instruction: &str, hints: &RefineHints) -> Result<Refined, DictationError> {
        let r = self.client.edit(selection, instruction, &prompt_hints(hints)).await.map_err(|e| DictationError::Refine(e.to_string()))?;
        Ok(Refined { text: r.text, latency_ms: r.latency_ms, model: r.model })
    }
}

/// Stands in for a client that could not be built: every call fails with the reason, so the
/// pill and the history say what is wrong instead of hanging.
pub struct Unconfigured(pub String);

#[async_trait]
impl Transcriber for Unconfigured {
    async fn transcribe(&self, _wav: &[u8], _language: Option<&str>, _glossary: &[String]) -> Result<Transcript, DictationError> {
        Err(DictationError::Asr(self.0.clone()))
    }
}

#[async_trait]
impl Refiner for Unconfigured {
    async fn refine(&self, _text: &str, _hints: &RefineHints) -> Result<Refined, DictationError> {
        Err(DictationError::Refine(self.0.clone()))
    }

    async fn edit(&self, _selection: &str, _instruction: &str, _hints: &RefineHints) -> Result<Refined, DictationError> {
        Err(DictationError::Refine(self.0.clone()))
    }
}

/// Where one text goes (docs/dictation.md §15.3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InjectRoute {
    /// Clipboard + paste chord.
    Paste,
    /// Clipboard only; `note` is why a requested paste was not attempted.
    Clipboard {
        /// For the history (`Outcome::Clipboard { reason, code }`).
        note: Option<voltip_inject::InjectNote>,
    },
}

/// The route for `mode` and the foreground preflight: a paste only when one is asked for and the
/// target can receive synthetic input. Windows refuses input into a higher-integrity window (UIPI)
/// and while the secure desktop (UAC prompt, lock screen) owns the input, so the text stays on the
/// clipboard with the reason; `unknown` proceeds and the outcome is reported as it happens.
pub fn inject_route(mode: InjectMode, preflight: &InjectPreflight) -> InjectRoute {
    let target = preflight.target_process.as_deref().map(|p| format!("（{p}）")).unwrap_or_default();
    match (mode, preflight.decision) {
        (InjectMode::ClipboardOnly, _) => InjectRoute::Clipboard { note: None },
        (InjectMode::Paste, InjectDecision::ElevatedTarget) => InjectRoute::Clipboard {
            note: Some(voltip_inject::InjectNote::new(
                FallbackCode::ElevatedTarget,
                format!("前台窗口{target}以更高权限运行，Windows 不允许向它粘贴；文本已留在剪贴板，请手动粘贴"),
            )),
        },
        (InjectMode::Paste, InjectDecision::SecureDesktop) => InjectRoute::Clipboard {
            note: Some(voltip_inject::InjectNote::new(FallbackCode::SecureInput, "安全桌面（UAC 提示或锁屏）正在前台，无法粘贴；文本已留在剪贴板")),
        },
        (InjectMode::Paste, InjectDecision::Proceed | InjectDecision::Unknown) => InjectRoute::Paste,
    }
}

/// Clipboard + paste (or clipboard only) through `voltip-inject`, switched by the mode the
/// engine factory last saw — so `settings_set_engines` takes effect without rebuilding the ports —
/// and by the foreground preflight ([`inject_route`]) before every paste. For voice edit
/// (docs/dictation.md §19) it also copies the selection, with the session's copy timing.
pub struct NativeInjector {
    mode: Arc<Mutex<InjectMode>>,
    paste: Box<dyn voltip_inject::Injector>,
    clipboard: Box<dyn voltip_inject::Injector>,
    preflight: Box<dyn Fn() -> InjectPreflight + Send + Sync>,
    selection: Option<Box<dyn SelectionSource>>,
    timing: SelectionTiming,
}

impl NativeInjector {
    /// The real system clipboard and key synthesis: enigo on Windows / macOS, the Linux tool chain
    /// (docs/dictation.md §14) on Linux, with this platform's paste timings; the selection copy of
    /// the same ports, copying after the key-up on X11 (§19).
    pub fn system(mode: Arc<Mutex<InjectMode>>) -> Self {
        let paste = voltip_inject::system_paste_injector(PasteOptions::for_os(std::env::consts::OS));
        let timing = crate::hotkey::selection_timing_for(crate::hotkey::linux_session().map(|s| s.kind));
        tracing::info!(backend = %voltip_inject::backend_summary(), options = ?paste.options(), selection_timing = ?timing, "text injection backend");
        Self::with(mode, Box::new(paste), Box::new(ClipboardOnlyInjector::new()))
            .with_preflight(crate::platform::inject_preflight)
            .with_selection(Box::new(voltip_inject::system_selection(CopyOptions::default())), timing)
    }

    /// Explicit backends (tests); the preflight always answers `proceed` until
    /// [`NativeInjector::with_preflight`] replaces it, and there is no selection copy until
    /// [`NativeInjector::with_selection`] adds one.
    pub fn with(mode: Arc<Mutex<InjectMode>>, paste: Box<dyn voltip_inject::Injector>, clipboard: Box<dyn voltip_inject::Injector>) -> Self {
        Self {
            mode,
            paste,
            clipboard,
            preflight: Box::new(|| InjectPreflight::not_applicable(HostOs::current())),
            selection: None,
            timing: SelectionTiming::AtPress,
        }
    }

    /// The foreground check asked before every paste (`crate::platform::inject_preflight` in production).
    pub fn with_preflight(self, preflight: impl Fn() -> InjectPreflight + Send + Sync + 'static) -> Self {
        Self { preflight: Box::new(preflight), ..self }
    }

    /// The selection copy of voice edit and when it may be sent (docs/dictation.md §19).
    pub fn with_selection(self, selection: Box<dyn SelectionSource>, timing: SelectionTiming) -> Self {
        Self { selection: Some(selection), timing, ..self }
    }
}

/// A clipboard fallback of the inject crate as the core records it (docs/dictation.md §4.2).
pub fn core_note(note: voltip_inject::InjectNote) -> InjectNote {
    let code = match note.code {
        FallbackCode::NoPermission => ClipboardCode::NoPermission,
        FallbackCode::NoTool => ClipboardCode::NoTool,
        FallbackCode::NoDisplay => ClipboardCode::NoDisplay,
        FallbackCode::SecureInput => ClipboardCode::SecureInput,
        FallbackCode::ElevatedTarget => ClipboardCode::ElevatedTarget,
        FallbackCode::Other => ClipboardCode::Other,
    };
    InjectNote::new(code, note.detail)
}

/// The core's hotkey modifier as the inject crate names it (`Ctrl` presses `Control`).
pub fn inject_modifier(modifier: Modifier) -> voltip_inject::Modifier {
    match modifier {
        Modifier::Ctrl => voltip_inject::Modifier::Control,
        Modifier::Alt => voltip_inject::Modifier::Alt,
        Modifier::Shift => voltip_inject::Modifier::Shift,
        Modifier::Meta => voltip_inject::Modifier::Meta,
    }
}

impl Injector for NativeInjector {
    fn inject(&self, text: &str) -> Result<Injection, DictationError> {
        let mode = *self.mode.lock();
        // The preflight only runs when a paste is wanted (on Windows it opens the foreground token).
        let route = match mode {
            InjectMode::Paste => inject_route(mode, &(self.preflight)()),
            InjectMode::ClipboardOnly => InjectRoute::Clipboard { note: None },
        };
        let (backend, blocked) = match route {
            InjectRoute::Paste => (&self.paste, None),
            InjectRoute::Clipboard { note } => (&self.clipboard, note),
        };
        if let Some(note) = &blocked {
            tracing::warn!(reason = %note.detail, code = ?note.code, "paste not attempted; the text goes to the clipboard");
        }
        let out = backend.inject(text).map_err(|e| DictationError::Inject(e.to_string()))?;
        let note = out.note.or(blocked).map(core_note);
        tracing::info!(via = %out.via, chars = out.chars, note = ?note, injector = backend.describe(), "text delivered");
        let via = match out.via {
            voltip_inject::Via::Paste => Via::Paste,
            voltip_inject::Via::Clipboard => Via::Clipboard,
        };
        Ok(Injection { via, note })
    }

    /// The history paste's copy (`voltip_core::paste`): the clipboard backend, whatever the mode;
    /// never the paste chord.
    fn copy(&self, text: &str) -> Result<(), DictationError> {
        let out = self.clipboard.inject(text).map_err(|e| DictationError::Inject(e.to_string()))?;
        tracing::info!(chars = out.chars, injector = self.clipboard.describe(), "text copied for a paste from the history");
        Ok(())
    }

    /// docs/dictation.md §19: the copy chord after releasing the edit hotkey's modifiers. Sent even
    /// with `inject = clipboard_only` (the rewrite then stays on the clipboard for a manual paste).
    fn copy_selection(&self, held: &[Modifier]) -> Result<Option<String>, DictationError> {
        let Some(selection) = &self.selection else { return Err(DictationError::EditUnavailable("this build has no selection copy".to_owned())) };
        let held: Vec<voltip_inject::Modifier> = held.iter().copied().map(inject_modifier).collect();
        selection.copy_selection(&held).map_err(|e| DictationError::Selection(e.to_string()))
    }

    fn selection_timing(&self) -> SelectionTiming {
        self.timing
    }

    /// docs/dictation.md §19.2: the terminal table of the host this build runs on
    /// (`voltip_platform::foreground::terminal_ids`; empty on macOS).
    fn is_terminal_app(&self, app_id: &str) -> bool {
        voltip_platform::foreground::is_terminal(voltip_platform::HostOs::current(), app_id)
    }
}

/// HTTP request deadline for one transcription (long recordings on a slow link).
pub const ASR_TIMEOUT: Duration = Duration::from_secs(90);
/// HTTP request deadline for one refinement.
pub const REFINE_TIMEOUT: Duration = Duration::from_secs(30);

/// Clients for a resolved configuration. On-device recognition hands out the shared
/// [`LocalTranscriber`] pointed at the selected model (the loaded recogniser survives settings
/// changes that keep the model) with `vad_trim` applied (docs/dictation.md §12). A remote provider
/// that is not ready, or whose configuration the client refuses, becomes an [`Unconfigured`] client
/// that reports the problem on use. No ready clean-up provider means no refiner (the core then
/// explains "润色未配置").
pub fn build_clients(engines: &ResolvedEngines, local: &LocalTranscriber) -> (Arc<dyn Transcriber>, Option<Arc<dyn Refiner>>) {
    let transcriber: Arc<dyn Transcriber> = if engines.is_local() {
        let id = engines.local_model.as_ref().map_or(voltip_asr_local::DEFAULT_MODEL_ID, |m| m.id.as_str());
        Arc::new(local.select(id).with_vad_trim(engines.vad_trim).with_compute(compute_of(engines)))
    } else {
        match &engines.asr_remote {
            None => Arc::new(Unconfigured(match engines.asr_issue {
                Some(issue) => format!("识别服务未配置：{}", issue.message(ServiceKind::Asr)),
                None => "识别服务未配置".to_owned(),
            })),
            Some(remote) => match HttpTranscriber::new(AsrConfig::new(&remote.url, &remote.model).with_token(remote.key.clone()).with_timeout(ASR_TIMEOUT)) {
                Ok(t) => Arc::new(t),
                Err(e) => {
                    tracing::warn!(error = %e, "ASR client not built");
                    Arc::new(Unconfigured(format!("识别服务配置无效：{e}")))
                }
            },
        }
    };
    let refiner: Option<Arc<dyn Refiner>> = engines.refine.as_ref().map(|remote| {
        // The built-in service stays under its free tier's output limit; a service the user
        // configured may answer a long translation or notes in full (docs/dictation.md §21).
        let cap = if engines.llm_provider == Some(ProviderId::Builtin) { voltip_refine::BUILTIN_OUTPUT_CAP } else { voltip_refine::USER_OUTPUT_CAP };
        let config = RefineConfig::new(&remote.url, &remote.model).with_api_key(remote.key.clone()).with_timeout(REFINE_TIMEOUT).with_output_cap(cap);
        match HttpRefiner::new(config) {
            Ok(r) => Arc::new(r) as Arc<dyn Refiner>,
            Err(e) => {
                tracing::warn!(error = %e, "refine client not built");
                Arc::new(Unconfigured(format!("润色服务配置无效：{e}")))
            }
        }
    });
    (transcriber, refiner)
}

/// Where the local recogniser runs, from `EngineSettings.local_device / local_gpu / local_threads`
/// (docs/dictation.md §10.6).
pub fn compute_of(engines: &ResolvedEngines) -> voltip_asr_local::Compute {
    voltip_asr_local::Compute { device: engines.local_device, gpu: engines.local_gpu.clone(), threads: engines.local_threads.map(usize::from) }
}

/// The machine as the settings page shows it: the CPU's threads and the GPUs this build drives.
pub fn hardware_status(machine: &voltip_asr_local::HardwareInfo) -> voltip_core::ui::HardwareStatus {
    voltip_core::ui::HardwareStatus {
        cpu_threads: u32::try_from(machine.cpu_threads).unwrap_or(u32::MAX),
        gpus: machine
            .gpus
            .iter()
            .map(|g| voltip_core::ui::GpuDevice {
                name: g.name.clone(),
                description: g.description.clone(),
                kind: g.kind.clone(),
                memory_mb: g.memory_total / (1024 * 1024),
                integrated: g.integrated,
            })
            .collect(),
    }
}

/// Deadline of one provider probe (the engines pane's 测试连接).
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// The engines pane's 测试连接 over HTTP (docs/dictation.md §3.3): `GET {base}/models` with the
/// key, answered with ids or a host-free failure. Recognition and clean-up endpoints are
/// normalised the same way (`…/v1`), so one request serves both kinds.
#[derive(Clone, Copy, Debug, Default)]
pub struct HttpServiceProbe;

#[async_trait]
impl ServiceProbe for HttpServiceProbe {
    async fn list_models(&self, base_url: &str, key: Option<&str>) -> Result<Vec<String>, ProbeError> {
        voltip_refine::list_models(base_url, key, PROBE_TIMEOUT).await.map_err(|e| {
            use voltip_refine::RefineError as E;
            match e {
                E::InvalidConfig(_) => ProbeError::new(ProbeFailure::InvalidUrl),
                E::Unauthorized => ProbeError::new(ProbeFailure::Unauthorized),
                E::RateLimited { .. } => ProbeError { reason: ProbeFailure::HttpStatus, status: Some(429) },
                E::Server { status, .. } => ProbeError { reason: ProbeFailure::HttpStatus, status: Some(status) },
                E::Network(_) => ProbeError::new(ProbeFailure::Unreachable),
                E::Timeout => ProbeError::new(ProbeFailure::Timeout),
                _ => ProbeError::new(ProbeFailure::BadResponse),
            }
        })
    }
}

/// The factory the core calls at start and after every settings / secret change. It also
/// records the injection mode for the [`NativeInjector`] sharing `mode`, hands the one
/// [`LocalTranscriber`] (with its loaded recogniser) to every local-mode configuration, and — with
/// a `store` — tells the model library whether the VAD is wanted (`vad_trim`, docs/dictation.md
/// §12): it then rides along with the next model download, or is fetched right away in the
/// background when the setting turns on later.
pub fn engine_factory(mode: Arc<Mutex<InjectMode>>, local: LocalTranscriber, store: Option<ModelStore>) -> EngineFactory {
    Arc::new(move |engines: &ResolvedEngines| {
        *mode.lock() = engines.inject;
        if let Some(store) = &store {
            store.set_auxiliary_wanted(engines.vad_trim);
            if engines.vad_trim && store.spawn_auxiliary_download() {
                tracing::info!("vad_trim is on and the VAD model is missing; downloading it in the background");
            }
        }
        build_clients(engines, &local)
    })
}

/// Everything the shell wires besides the core: the dictation ports and the microphone hub the
/// recorder shares with the webview level meters.
pub struct ShellPorts {
    /// Ports handed to the core.
    pub dictation: DictationPorts,
    /// Owner of the microphone level stream (managed as Tauri state).
    pub hub: Arc<AudioHub>,
}

impl ShellPorts {
    /// Any core ports (tests hand in `voltip_core::dictation::fakes::ports()`) with a hub whose
    /// device meter cannot open (no microphone on the mock runtime).
    pub fn headless(dictation: DictationPorts) -> Self {
        Self { dictation, hub: Arc::new(AudioHub::with_opener(Box::new(|_, _| Err("no audio device on this runtime".into())))) }
    }
}

/// Production wiring: cpal microphone, HTTP clients, the local model library under
/// `models_root` (`CoreConfig::models_root`), the streaming preview over the same library,
/// system clipboard + paste, the platform's foreground probe.
pub fn production_ports(models_root: PathBuf) -> ShellPorts {
    ports_with_backend(Arc::new(CpalBackend::new()), Arc::new(AudioHub::default()), models_root)
}

/// Wiring over any audio backend and hub (tests use `voltip_audio::FakeBackend`); the model
/// library lives under `models_root`. The streaming transcriber is always plugged in: the core
/// only opens it when `live_preview` is on and the streaming model is installed (docs/dictation.md
/// §11), and it loads nothing until then. A long take is cut where the Silero VAD hears a pause
/// (§22); the first long take without the model fetches it.
pub fn ports_with_backend(backend: Arc<dyn Backend + Send + Sync>, hub: Arc<AudioHub>, models_root: PathBuf) -> ShellPorts {
    let mode = Arc::new(Mutex::new(InjectMode::default()));
    let store = ModelStore::new(models_root.clone());
    let dictation = DictationPorts {
        audio: Arc::new(RecorderAudioSource::with_backend(backend, hub.clone())),
        injector: Arc::new(NativeInjector::system(mode.clone())),
        factory: engine_factory(mode, LocalTranscriber::new(models_root.clone()), Some(store.clone())),
        segmenter: Some(Arc::new(VadSegmenterFactory::new(models_root.clone(), Some(store.clone())))),
        models: Some(Arc::new(store)),
        streaming: Some(Arc::new(LocalStreamingTranscriber::new(models_root))),
        probe: Some(Arc::new(crate::platform::PlatformProbe::new())),
        service_probe: Some(Arc::new(HttpServiceProbe)),
    };
    ShellPorts { dictation, hub }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// Regression (2026-09-28): a chosen microphone that is gone (unplugged, a settings file from
    /// another machine) must not break dictation; the take records from the default input.
    #[test]
    fn a_chosen_microphone_that_is_gone_falls_back_to_the_default_input() {
        let backend = voltip_audio::FakeBackend::new();
        assert_eq!(connected_or_default(&backend, None), None);
        assert_eq!(connected_or_default(&backend, Some(voltip_audio::FAKE_USB_ID)).as_deref(), Some(voltip_audio::FAKE_USB_ID));
        assert_eq!(connected_or_default(&backend, Some("fake:unplugged")), None);
    }

    /// docs/dictation.md §22: the capture options choose the recorder's source; a long take
    /// streams the whole take to the core; an output device that is gone falls back to the default
    /// output, as a microphone does.
    #[test]
    fn the_capture_follows_the_source_and_streams_a_long_take() {
        use voltip_audio::FAKE_SPEAKERS_ID;
        let backend = Arc::new(voltip_audio::FakeBackend::new());
        let source = RecorderAudioSource::with_backend(backend.clone(), Arc::new(AudioHub::default()));
        let options = |source, output: Option<&str>, long| CaptureOptions {
            live: false,
            max_duration: if long { Duration::from_secs(600) } else { MAX_RECORDING },
            long,
            source,
            output_device: output.map(str::to_owned),
        };
        let mut capture = source.start(None, Box::new(|_| {}), Box::new(|| {}), options(RecordingSource::System, Some(FAKE_SPEAKERS_ID), true)).unwrap();
        let mut stream = capture.pcm_stream().expect("a long take streams");
        assert!(capture.pcm_stream().is_none(), "handed out once");
        let (mut got, mut buf, started) = (0, vec![0.0f32; 1024], std::time::Instant::now());
        while got < 1600 && started.elapsed() < Duration::from_secs(10) {
            got += stream.read(&mut buf);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(got >= 1600, "the computer's sound reaches the stream: {got}");
        capture.stop().unwrap();
        assert_eq!(backend.opened_outputs(), vec![Some(FAKE_SPEAKERS_ID.to_owned())]);
        assert!(backend.opened_with().is_empty(), "no microphone for the computer's sound");
        let mut mixed = source.start(None, Box::new(|_| {}), Box::new(|| {}), options(RecordingSource::Mixed, Some("fake:gone"), false)).unwrap();
        assert!(mixed.pcm_stream().is_none(), "a short take has no stream");
        mixed.stop().unwrap();
        // The recorder opens the default output by its id.
        assert_eq!(backend.opened_outputs(), vec![Some(FAKE_SPEAKERS_ID.to_owned()); 2], "the default output");
        assert_eq!(backend.opened_with(), vec![None], "and the microphone");
        assert_eq!(output_or_default(backend.as_ref(), Some("fake:gone")), None);
        assert_eq!(output_or_default(backend.as_ref(), Some(FAKE_SPEAKERS_ID)).as_deref(), Some(FAKE_SPEAKERS_ID));
    }

    fn preflight(decision: InjectDecision, target: Option<&str>) -> InjectPreflight {
        InjectPreflight { decision, target_process: target.map(str::to_owned), checked: true, ..InjectPreflight::not_applicable(HostOs::Windows) }
    }

    #[test]
    fn route_table() {
        use InjectDecision::{ElevatedTarget, Proceed, SecureDesktop, Unknown};
        assert_eq!(inject_route(InjectMode::Paste, &preflight(Proceed, None)), InjectRoute::Paste);
        assert_eq!(inject_route(InjectMode::Paste, &preflight(Unknown, None)), InjectRoute::Paste, "unknown proceeds and reports honestly");
        assert_eq!(inject_route(InjectMode::Paste, &InjectPreflight::not_applicable(HostOs::Linux)), InjectRoute::Paste, "Linux / macOS: unchanged");
        let InjectRoute::Clipboard { note: Some(elevated) } = inject_route(InjectMode::Paste, &preflight(ElevatedTarget, Some("regedit.exe"))) else {
            panic!("elevated target must not paste")
        };
        assert_eq!(elevated.code, FallbackCode::ElevatedTarget);
        assert!(elevated.detail.contains("（regedit.exe）") && elevated.detail.contains("更高权限") && elevated.detail.contains("剪贴板"), "{elevated}");
        let InjectRoute::Clipboard { note: Some(unnamed) } = inject_route(InjectMode::Paste, &preflight(ElevatedTarget, None)) else { panic!() };
        assert!(unnamed.detail.starts_with("前台窗口以更高权限运行"), "{unnamed}");
        let InjectRoute::Clipboard { note: Some(secure) } = inject_route(InjectMode::Paste, &preflight(SecureDesktop, None)) else {
            panic!("secure desktop must not paste")
        };
        assert_eq!(secure.code, FallbackCode::SecureInput);
        assert!(secure.detail.contains("安全桌面") && secure.detail.contains("剪贴板"), "{secure}");
        for decision in [Proceed, Unknown, ElevatedTarget, SecureDesktop] {
            assert_eq!(inject_route(InjectMode::ClipboardOnly, &preflight(decision, None)), InjectRoute::Clipboard { note: None }, "{decision:?}");
        }
    }

    /// docs/dictation.md §4.2: every fallback kind of the inject crate reaches the core as the same
    /// kind, with its message.
    #[test]
    fn every_fallback_code_reaches_the_core() {
        let codes = [
            (FallbackCode::NoPermission, ClipboardCode::NoPermission),
            (FallbackCode::NoTool, ClipboardCode::NoTool),
            (FallbackCode::NoDisplay, ClipboardCode::NoDisplay),
            (FallbackCode::SecureInput, ClipboardCode::SecureInput),
            (FallbackCode::ElevatedTarget, ClipboardCode::ElevatedTarget),
            (FallbackCode::Other, ClipboardCode::Other),
        ];
        for (from, to) in codes {
            assert_eq!(core_note(voltip_inject::InjectNote::new(from, "detail")), InjectNote::new(to, "detail"));
        }
    }

    /// Records the texts it was handed and answers with `via`.
    struct Recording {
        via: voltip_inject::Via,
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl Recording {
        fn boxed(via: voltip_inject::Via) -> (Box<dyn voltip_inject::Injector>, Arc<Mutex<Vec<String>>>) {
            let calls = Arc::new(Mutex::new(Vec::new()));
            (Box::new(Self { via, calls: calls.clone() }), calls)
        }
    }

    impl voltip_inject::Injector for Recording {
        fn inject(&self, text: &str) -> Result<voltip_inject::Injection, voltip_inject::InjectError> {
            self.calls.lock().push(text.to_owned());
            Ok(voltip_inject::Injection { via: self.via, chars: text.chars().count(), note: None })
        }
        fn describe(&self) -> &'static str {
            "recording"
        }
    }

    /// Plays the foreground application for the selection copy: answers `selection`, records `held`.
    struct Selected {
        selection: Result<Option<String>, voltip_inject::InjectError>,
        held: Arc<Mutex<Vec<Vec<voltip_inject::Modifier>>>>,
    }

    impl SelectionSource for Selected {
        fn copy_selection(&self, held: &[voltip_inject::Modifier]) -> Result<Option<String>, voltip_inject::InjectError> {
            self.held.lock().push(held.to_vec());
            self.selection.clone()
        }
    }

    /// docs/dictation.md §19: the native injector copies through its selection source with the
    /// core's modifiers mapped, reports its timing, maps a copy error to `selection`, and without a
    /// source refuses with `edit_unavailable`.
    #[test]
    fn the_native_injector_copies_the_selection_with_the_held_modifiers() {
        let mode = Arc::new(Mutex::new(InjectMode::Paste));
        let (paste, _) = Recording::boxed(voltip_inject::Via::Paste);
        let (clipboard, _) = Recording::boxed(voltip_inject::Via::Clipboard);
        let plain = NativeInjector::with(mode.clone(), paste, clipboard);
        assert!(matches!(plain.copy_selection(&[]), Err(DictationError::EditUnavailable(_))));
        assert_eq!(plain.selection_timing(), SelectionTiming::AtPress);
        let held = Arc::new(Mutex::new(Vec::new()));
        let source = Selected { selection: Ok(Some("选中".into())), held: held.clone() };
        let injector = plain.with_selection(Box::new(source), SelectionTiming::AfterKeyUp);
        assert_eq!(injector.copy_selection(&[Modifier::Ctrl, Modifier::Alt, Modifier::Shift, Modifier::Meta]).unwrap().as_deref(), Some("选中"));
        assert_eq!(
            *held.lock(),
            vec![vec![voltip_inject::Modifier::Control, voltip_inject::Modifier::Alt, voltip_inject::Modifier::Shift, voltip_inject::Modifier::Meta]]
        );
        assert_eq!(injector.selection_timing(), SelectionTiming::AfterKeyUp);
        let (paste, _) = Recording::boxed(voltip_inject::Via::Paste);
        let (clipboard, _) = Recording::boxed(voltip_inject::Via::Clipboard);
        let broken = NativeInjector::with(mode, paste, clipboard).with_selection(
            Box::new(Selected { selection: Err(voltip_inject::InjectError::Keystroke("no copy tool".into())), held: Arc::new(Mutex::new(Vec::new())) }),
            SelectionTiming::AtPress,
        );
        assert_eq!(broken.copy_selection(&[]).unwrap_err(), DictationError::Selection("keystroke: no copy tool".into()));
        // §19.2: the terminal table of the host this build runs on.
        assert_eq!(broken.is_terminal_app("gnome-terminal-server"), cfg!(target_os = "linux"));
        assert_eq!(broken.is_terminal_app("windowsterminal"), cfg!(target_os = "windows"));
        assert!(!broken.is_terminal_app("com.apple.terminal"), "Cmd+C copies in macOS terminals");
        assert!(!broken.is_terminal_app("code"));
    }

    /// Regression (history paste, 2026-09-29): the paste button's copy-only answers (no window came
    /// up, the window changed, pure Wayland) go through `Injector::copy`; the native injector had no
    /// copy of its own, so every one of them failed instead of leaving the text on the clipboard.
    /// The copy uses the clipboard backend whatever the mode, and never the paste chord.
    #[test]
    fn regression_the_native_injector_copies_for_the_history_paste() {
        let mode = Arc::new(Mutex::new(InjectMode::Paste));
        let (paste, pasted) = Recording::boxed(voltip_inject::Via::Paste);
        let (clipboard, copied) = Recording::boxed(voltip_inject::Via::Clipboard);
        let injector = NativeInjector::with(mode.clone(), paste, clipboard);
        assert_eq!(injector.copy("只复制"), Ok(()));
        *mode.lock() = InjectMode::ClipboardOnly;
        assert_eq!(injector.copy("仍然只复制"), Ok(()));
        assert!(pasted.lock().is_empty(), "a copy never presses the paste chord");
        assert_eq!(*copied.lock(), vec!["只复制".to_string(), "仍然只复制".to_string()]);
    }

    #[test]
    fn blocked_preflight_uses_the_clipboard_backend_with_the_reason() {
        let mode = Arc::new(Mutex::new(InjectMode::Paste));
        let (paste, pasted) = Recording::boxed(voltip_inject::Via::Paste);
        let (clipboard, copied) = Recording::boxed(voltip_inject::Via::Clipboard);
        let decision = Arc::new(Mutex::new(InjectDecision::ElevatedTarget));
        let asked = decision.clone();
        let injector = NativeInjector::with(mode.clone(), paste, clipboard).with_preflight(move || preflight(*asked.lock(), Some("taskmgr.exe")));

        let out = injector.inject("管理员窗口").unwrap();
        assert_eq!(out.via, Via::Clipboard);
        let note = out.note.clone().unwrap();
        assert_eq!(note.code, ClipboardCode::ElevatedTarget, "the code reaches the core");
        assert!(note.detail.contains("（taskmgr.exe）以更高权限运行"), "{note:?}");
        assert!(pasted.lock().is_empty(), "no paste attempted into an elevated window");
        assert_eq!(*copied.lock(), vec!["管理员窗口".to_string()]);

        *decision.lock() = InjectDecision::SecureDesktop;
        let note = injector.inject("锁屏").unwrap().note.unwrap();
        assert_eq!(note.code, ClipboardCode::SecureInput);
        assert!(note.detail.contains("安全桌面"));
        assert!(pasted.lock().is_empty());

        *decision.lock() = InjectDecision::Proceed;
        let out = injector.inject("普通窗口").unwrap();
        assert_eq!((out.via, out.note), (Via::Paste, None));
        assert_eq!(*pasted.lock(), vec!["普通窗口".to_string()]);

        // clipboard_only never consults the preflight and carries no reason.
        *mode.lock() = InjectMode::ClipboardOnly;
        *decision.lock() = InjectDecision::ElevatedTarget;
        let out = injector.inject("只复制").unwrap();
        assert_eq!((out.via, out.note), (Via::Clipboard, None));
        assert_eq!(copied.lock().last().map(String::as_str), Some("只复制"));

        // `with` alone proceeds, which is the Linux / macOS behaviour.
        let (paste, pasted) = Recording::boxed(voltip_inject::Via::Paste);
        let (clipboard, _) = Recording::boxed(voltip_inject::Via::Clipboard);
        let plain = NativeInjector::with(Arc::new(Mutex::new(InjectMode::Paste)), paste, clipboard);
        assert_eq!(plain.inject("x").unwrap().via, Via::Paste);
        assert_eq!(*pasted.lock(), vec!["x".to_string()]);
    }
}
