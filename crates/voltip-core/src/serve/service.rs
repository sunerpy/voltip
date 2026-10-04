//! What the local speech service does with one take (docs/dictation.md §23.4): resolve the
//! request's processing, recognise the audio — a take of up to two minutes whole, a longer one in
//! segments as the app's long takes are (§22) — and run the dictionary, the clean-up and the rules
//! with the shared [`steps`]. The state a request runs with comes from a [`StateSource`]: the
//! headless server reads the app's files ([`FileSource`]), the app pushes its own ([`PushedState`]).

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use parking_lot::{Mutex, RwLock};
use serde::Serialize;
use voltip_protocol::Platform;

use super::profile::{Catalog, Defaults, ModelInfo, ProfileError, Recipe, model_list, parse_model, resolve_recipe};
use crate::dictation::fallback::QuotaLedger;
use crate::dictation::long::{self, EnergySegmenter, SEGMENT_ATTEMPTS};
use crate::dictation::ports::{DictationError, MIN_RECORDING, PCM_SAMPLE_RATE_HZ, Refiner, Segmenter, SegmenterFactory, Transcriber};
use crate::dictation::{EngineFactory, engine_clients, steps, wav};
use crate::engines::{BuiltIn, EngineIssue, EngineSettings, LocalDevice, ResolvedEngines, UserSecrets};
use crate::models::{CancelToken, ModelManager, ModelState};
use crate::presets::{CustomPreset, PresetRef, PresetStore};
use crate::providers::{ProviderId, ServiceKind};
use crate::scenes::{Scene, SceneRef, SceneStore};
use crate::settings::{Settings, SettingsStore};
use crate::vocabulary::{DictionaryEntry, DictionaryStore, ReplacementRule, RuleStore, Vocabulary};

/// Samples per second of the audio the service processes (the app's recording rate).
pub const RATE: u64 = PCM_SAMPLE_RATE_HZ as u64;
/// Longest audio one request may carry, in minutes, as the app's longest take (§22).
pub const MAX_MINUTES: u16 = 120;

/// One take as the HTTP layer hands it over: 16 kHz mono 16-bit little-endian PCM, no header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PcmFile {
    /// The file.
    pub path: PathBuf,
    /// How many samples it holds.
    pub samples: u64,
}

/// The fields of a request that decide its processing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServeRequest {
    /// `model`.
    pub model: Option<String>,
    /// `language`.
    pub language: Option<String>,
}

/// What came of one take: the `text` a client reads and what the app's history would record.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ServeOutcome {
    /// The text after the dictionary, the clean-up and the rules; empty when nothing was said.
    pub text: String,
    /// The recogniser's text (in the take's script).
    pub raw_text: String,
    /// The clean-up's text is in `text`.
    pub refined: bool,
    /// Why a requested clean-up's text is not in `text`.
    pub refine_error: Option<String>,
    /// The preset of a requested clean-up.
    pub preset: Option<PresetRef>,
    /// The scene the take ran with.
    pub scene: Option<SceneRef>,
    /// The language hint the take ran with (`None`: none).
    pub language: Option<String>,
    /// The model that recognised it, when the client says.
    pub asr_model: Option<String>,
    /// The model whose text is in `text`.
    pub refine_model: Option<String>,
    /// Recognition time, milliseconds (summed over a long take's segments).
    pub asr_ms: u64,
    /// Clean-up time, milliseconds.
    pub refine_ms: Option<u64>,
    /// Length of the audio, milliseconds.
    pub duration_ms: u64,
    /// How many pieces were recognised: 1 for a take of up to two minutes, the segments of a
    /// longer one, 0 when nothing was sent.
    pub segments: u32,
}

/// Why a request was not answered with a text.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ServeError {
    /// The request asks for something that does not exist or is not valid (400).
    #[error("{0}")]
    Invalid(String),
    /// Recognition cannot run now: not configured, its model not downloaded (503).
    #[error("{0}")]
    NotReady(String),
    /// The recognition service's quota is used up (429).
    #[error("{0}")]
    Quota(String),
    /// The recognition service failed (502).
    #[error("{0}")]
    Upstream(String),
    /// The audio file could not be read (500).
    #[error("{0}")]
    Internal(String),
    /// The client went away.
    #[error("已取消")]
    Cancelled,
}

impl From<ProfileError> for ServeError {
    fn from(error: ProfileError) -> Self {
        Self::Invalid(error.0)
    }
}

fn from_dictation(error: DictationError) -> ServeError {
    match error {
        DictationError::QuotaExhausted { .. } => ServeError::Quota(error.to_string()),
        DictationError::Audio(reason) => ServeError::Internal(reason),
        other => ServeError::Upstream(other.to_string()),
    }
}

/// Everything one request is processed with, as one snapshot.
#[derive(Clone)]
pub struct ServeState {
    /// The settings (engines, context sharing).
    pub settings: Settings,
    /// The custom presets.
    pub presets: Arc<Vec<CustomPreset>>,
    /// The scenes, with the built-in ones.
    pub scenes: Arc<Vec<Scene>>,
    /// The dictionary and the rules.
    pub vocabulary: Arc<Vocabulary>,
    /// What the clients were built from.
    pub engines: ResolvedEngines,
    /// Recognition (behind its fallback chain).
    pub transcriber: Arc<dyn Transcriber>,
    /// The clean-up, when one is configured.
    pub refiner: Option<Arc<dyn Refiner>>,
    /// Where a long take is cut; `None`: the core's own cutter.
    pub segmenter: Option<Arc<dyn SegmenterFactory>>,
}

impl std::fmt::Debug for ServeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServeState")
            .field("engines", &self.engines)
            .field("presets", &self.presets.len())
            .field("scenes", &self.scenes.len())
            .finish_non_exhaustive()
    }
}

/// Where the service gets its state.
pub trait StateSource: Send + Sync {
    /// The state the next request runs with.
    fn current(&self) -> Arc<ServeState>;
}

/// The state the app pushes whenever its settings, lists or clients change (§23.6).
pub struct PushedState(RwLock<Arc<ServeState>>);

impl PushedState {
    /// Start from `state`.
    pub fn new(state: ServeState) -> Self {
        Self(RwLock::new(Arc::new(state)))
    }

    /// Requests from now on run with `state`.
    pub fn set(&self, state: ServeState) {
        *self.0.write() = Arc::new(state);
    }
}

impl StateSource for PushedState {
    fn current(&self) -> Arc<ServeState> {
        self.0.read().clone()
    }
}

/// What the headless server changes in the engine settings it read (its command line).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EngineOverrides {
    /// `--asr`.
    pub asr: Option<ProviderId>,
    /// `--llm`.
    pub llm: Option<ProviderId>,
    /// `--local-model`: also selects on-device recognition unless `--asr` names another provider.
    pub local_model: Option<String>,
    /// `--device`.
    pub device: Option<LocalDevice>,
    /// `--gpu`.
    pub gpu: Option<String>,
    /// `--threads`.
    pub threads: Option<u16>,
}

impl EngineOverrides {
    /// Apply the overrides to `engines`.
    pub fn apply(&self, engines: &mut EngineSettings) {
        if let Some(model) = &self.local_model {
            engines.local_model = Some(model.clone());
            engines.asr_provider = ProviderId::Local;
        }
        if let Some(provider) = self.asr {
            engines.asr_provider = provider;
        }
        if let Some(provider) = self.llm {
            engines.llm_provider = provider;
        }
        if let Some(device) = self.device {
            engines.local_device = device;
        }
        if let Some(gpu) = &self.gpu {
            engines.local_gpu = Some(gpu.clone());
        }
        if let Some(threads) = self.threads {
            engines.local_threads = Some(threads);
        }
    }
}

/// What the headless server's [`FileSource`] is built from.
pub struct FileSourceConfig {
    /// The app's data directory (`settings.json`, the lists, the model library).
    pub data_dir: PathBuf,
    /// Whose built-in scene applications the scene list gets (this host's).
    pub platform: Platform,
    /// The command line's engine overrides.
    pub overrides: EngineOverrides,
    /// The engine keys, read once when the server starts ([`crate::providers::peek_user_secrets`]).
    pub secrets: UserSecrets,
    /// The service compiled into the build.
    pub built_in: BuiltIn,
    /// The local model library.
    pub models: Option<Arc<dyn ModelManager>>,
    /// Builds the clients for a configuration.
    pub factory: EngineFactory,
    /// Where long takes are cut.
    pub segmenter: Option<Arc<dyn SegmenterFactory>>,
}

/// A file's identity as far as reloading goes: its modification time and length.
type Stamp = Option<(SystemTime, u64)>;

fn stamp(path: &Path) -> Stamp {
    std::fs::metadata(path).ok().map(|m| (m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len()))
}

const FILES: [&str; 5] = [
    crate::settings::SETTINGS_FILE_NAME,
    crate::presets::PRESETS_FILE_NAME,
    crate::scenes::SCENES_FILE_NAME,
    crate::vocabulary::DICTIONARY_FILE_NAME,
    crate::vocabulary::RULES_FILE_NAME,
];

struct Loaded {
    stamps: [Stamp; 5],
    settings: Settings,
    presets: Arc<Vec<CustomPreset>>,
    scenes: Arc<Vec<Scene>>,
    dictionary: Vec<DictionaryEntry>,
    rules: Vec<ReplacementRule>,
    library: Vec<ModelState>,
    state: Arc<ServeState>,
}

/// The headless server's state (§23.5): the app's files, read with no side effect before each
/// request when one of them changed. A file that turns unusable keeps the last content that was
/// usable and logs; nothing is ever renamed, moved or written.
pub struct FileSource {
    config: FileSourceConfig,
    quota: QuotaLedger,
    loaded: Mutex<Loaded>,
}

impl std::fmt::Debug for FileSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileSource").field("data_dir", &self.config.data_dir).finish_non_exhaustive()
    }
}

impl FileSource {
    /// Read the files once. An unusable `settings.json` is an error (the server does not start); an
    /// unusable list is empty for now, and its reason is returned for the log.
    pub fn open(config: FileSourceConfig) -> Result<(Self, Vec<String>), String> {
        let dir = &config.data_dir;
        let settings = SettingsStore::new(dir).load().map_err(|e| format!("settings.json 无法使用（{e}）"))?;
        let mut notices = Vec::new();
        let presets = usable(PresetStore::read(dir), &mut notices);
        let scenes = usable(SceneStore::read_on(dir, config.platform, crate::now_ms()), &mut notices);
        let dictionary = usable(DictionaryStore::read(dir), &mut notices);
        let rules = usable(RuleStore::read(dir), &mut notices);
        let library = config.models.as_ref().map(|m| m.scan()).unwrap_or_default();
        let stamps = FILES.map(|name| stamp(&dir.join(name)));
        let quota = QuotaLedger::default();
        let state = build_state(&config, &quota, &settings, Arc::new(presets.clone()), Arc::new(scenes.clone()), &dictionary, &rules, &library, None);
        let loaded = Loaded { stamps, settings, presets: Arc::new(presets), scenes: Arc::new(scenes), dictionary, rules, library, state: Arc::new(state) };
        Ok((Self { config, quota, loaded: Mutex::new(loaded) }, notices))
    }

    /// Re-read what changed since the last request.
    fn refresh(&self) {
        let dir = &self.config.data_dir;
        let mut loaded = self.loaded.lock();
        let stamps = FILES.map(|name| stamp(&dir.join(name)));
        let library = self.config.models.as_ref().map(|m| m.scan()).unwrap_or_default();
        if stamps == loaded.stamps && library == loaded.library {
            return;
        }
        let before = loaded.stamps;
        let changed = |i: usize| stamps[i] != before[i];
        let keep = |what: &str, why: &str| tracing::warn!(file = what, %why, "a file the service reads is unusable; keeping its last usable content");
        if changed(0) {
            match SettingsStore::new(dir).load() {
                Ok(settings) => loaded.settings = settings,
                Err(e) => keep("settings.json", &e.to_string()),
            }
        }
        if changed(1) {
            match PresetStore::read(dir) {
                Ok(presets) => loaded.presets = Arc::new(presets),
                Err(why) => keep("presets.json", &why),
            }
        }
        if changed(2) {
            match SceneStore::read_on(dir, self.config.platform, crate::now_ms()) {
                Ok(scenes) => loaded.scenes = Arc::new(scenes),
                Err(why) => keep("scenes.json", &why),
            }
        }
        if changed(3) {
            match DictionaryStore::read(dir) {
                Ok(dictionary) => loaded.dictionary = dictionary,
                Err(why) => keep("dictionary.json", &why),
            }
        }
        if changed(4) {
            match RuleStore::read(dir) {
                Ok(rules) => loaded.rules = rules,
                Err(why) => keep("rules.json", &why),
            }
        }
        let previous = loaded.state.clone();
        let state = build_state(
            &self.config,
            &self.quota,
            &loaded.settings,
            loaded.presets.clone(),
            loaded.scenes.clone(),
            &loaded.dictionary,
            &loaded.rules,
            &library,
            Some(&previous),
        );
        loaded.stamps = stamps;
        loaded.library = library;
        loaded.state = Arc::new(state);
        tracing::info!("the service re-read the settings and lists that changed");
    }
}

impl StateSource for FileSource {
    fn current(&self) -> Arc<ServeState> {
        self.refresh();
        self.loaded.lock().state.clone()
    }
}

/// A list that could be read, or an empty one with the reason noted.
fn usable<T>(result: Result<Vec<T>, String>, notices: &mut Vec<String>) -> Vec<T> {
    result.unwrap_or_else(|why| {
        notices.push(why);
        Vec::new()
    })
}

/// The state for the files' contents; the clients of `previous` are kept when the resolved engines
/// did not change (a loaded local model stays loaded).
#[allow(clippy::too_many_arguments)]
fn build_state(
    config: &FileSourceConfig,
    quota: &QuotaLedger,
    settings: &Settings,
    presets: Arc<Vec<CustomPreset>>,
    scenes: Arc<Vec<Scene>>,
    dictionary: &[DictionaryEntry],
    rules: &[ReplacementRule],
    library: &[ModelState],
    previous: Option<&ServeState>,
) -> ServeState {
    let mut settings = settings.clone();
    config.overrides.apply(&mut settings.engines);
    let engines = ResolvedEngines::resolve_with_models(&settings.engines, &config.secrets, &config.built_in, library);
    let (transcriber, refiner) = match previous.filter(|p| p.engines == engines) {
        Some(p) => (p.transcriber.clone(), p.refiner.clone()),
        None => engine_clients(&config.factory, &engines, quota),
    };
    ServeState {
        settings,
        presets,
        scenes,
        vocabulary: Arc::new(Vocabulary::compile(dictionary, rules)),
        engines,
        transcriber,
        refiner,
        segmenter: config.segmenter.clone(),
    }
}

/// Why recognition cannot run with `engines`, when it cannot.
pub fn not_ready(engines: &ResolvedEngines) -> Option<String> {
    match engines.asr_issue {
        None => None,
        Some(EngineIssue::ModelNotInstalled) => {
            let (id, name) = engines.local_model.as_ref().map_or(("", ""), |m| (m.id.as_str(), m.name.as_str()));
            Some(format!("本地模型未下载：{name}（可运行 voltip-server --download-model {id}）"))
        }
        Some(issue) => Some(format!("识别服务未配置：{}", issue.message(ServiceKind::Asr))),
    }
}

/// The local speech service: the defaults a host started with over a [`StateSource`].
pub struct Service {
    source: Arc<dyn StateSource>,
    defaults: RwLock<Defaults>,
}

impl std::fmt::Debug for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Service").field("defaults", &*self.defaults.read()).finish_non_exhaustive()
    }
}

impl Service {
    /// A service over `source` with `defaults`.
    pub fn new(source: Arc<dyn StateSource>, defaults: Defaults) -> Self {
        Self { source, defaults: RwLock::new(defaults) }
    }

    /// The defaults the next requests run with (the app's service settings changed).
    pub fn set_defaults(&self, defaults: Defaults) {
        *self.defaults.write() = defaults;
    }

    /// The defaults now.
    pub fn defaults(&self) -> Defaults {
        self.defaults.read().clone()
    }

    /// The state the next request runs with.
    pub fn state(&self) -> Arc<ServeState> {
        self.source.current()
    }

    /// `GET /v1/models`.
    pub fn models(&self) -> Vec<ModelInfo> {
        let state = self.state();
        model_list(&state.presets, &state.scenes)
    }

    /// Whether a request can be recognised now; the reason when it cannot.
    pub fn ready(&self) -> Result<(), String> {
        not_ready(&self.state().engines).map_or(Ok(()), Err)
    }

    /// Start loading what recognition needs (a local model) in the background.
    pub fn warm(&self) {
        let state = self.state();
        let language = self.defaults.read().language.clone().or_else(|| state.settings.engines.language.clone());
        state.transcriber.warm(language.as_deref());
    }

    /// Process one take (§23.4). Nothing said is an empty text, not an error.
    pub async fn transcribe(&self, audio: &PcmFile, request: &ServeRequest, cancel: &CancelToken) -> Result<ServeOutcome, ServeError> {
        let state = self.state();
        let defaults = self.defaults();
        let profile = parse_model(request.model.as_deref())?;
        let catalog = Catalog { settings: &state.settings, presets: &state.presets, scenes: &state.scenes, vocabulary: &state.vocabulary };
        let recipe = resolve_recipe(&defaults, &profile, request.language.as_deref(), catalog)?;
        if let Some(reason) = not_ready(&state.engines) {
            return Err(ServeError::NotReady(reason));
        }
        if cancel.is_cancelled() {
            return Err(ServeError::Cancelled);
        }
        let duration_ms = audio.samples * 1000 / RATE;
        let quiet = ServeOutcome { scene: recipe.scene.clone(), language: recipe.language.clone(), duration_ms, ..ServeOutcome::default() };
        let long_take = audio.samples > long::IN_MEMORY;
        let recognised = if long_take { recognize_long(&state, &recipe, audio, cancel).await? } else { recognize_whole(&state, &recipe, audio).await? };
        let Some(Recognised { raw_text, asr_ms, asr_model, segments }) = recognised else {
            return Ok(ServeOutcome { segments: 0, ..quiet });
        };
        if cancel.is_cancelled() {
            return Err(ServeError::Cancelled);
        }
        let corrected = steps::correct(&recipe.vocabulary, &raw_text);
        let refiner = state.refiner.as_ref().filter(|_| recipe.refine);
        let cleaned = match steps::plan_refine(refiner.is_some(), recipe.refine, long_take, &corrected.text) {
            steps::RefinePlan::Run => match refiner {
                Some(refiner) => steps::run_refine(refiner.as_ref(), &corrected.text, &recipe.hints).await,
                None => steps::CleanUp::skipped(&corrected.text, None),
            },
            steps::RefinePlan::Skip { error } => steps::CleanUp::skipped(&corrected.text, error),
        };
        let ruled = steps::apply_rules(&recipe.vocabulary, &cleaned.text);
        let text = if ruled.text.trim().is_empty() { String::new() } else { ruled.text };
        Ok(ServeOutcome {
            text,
            raw_text,
            refined: cleaned.refined,
            refine_error: cleaned.refine_error,
            preset: recipe.preset_ref(),
            refine_model: cleaned.refine_model,
            refine_ms: cleaned.refine_ms,
            asr_ms,
            asr_model,
            segments,
            ..quiet
        })
    }
}

/// What recognition gave a take.
struct Recognised {
    raw_text: String,
    asr_ms: u64,
    asr_model: Option<String>,
    segments: u32,
}

async fn read_wav(path: &Path, start: u64, end: u64) -> Result<Vec<u8>, ServeError> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || long::read_segment(&path, start, end))
        .await
        .map_err(|e| ServeError::Internal(format!("音频读取任务失败：{e}")))?
        .map_err(|e| ServeError::Internal(format!("音频文件无法读取：{e}")))
}

/// A take of up to two minutes, whole; `None` when it is too short or silent (never sent, as the
/// app's whole takes, §2).
async fn recognize_whole(state: &ServeState, recipe: &Recipe, audio: &PcmFile) -> Result<Option<Recognised>, ServeError> {
    let wav = read_wav(&audio.path, 0, audio.samples).await?;
    let min_samples = u64::try_from(MIN_RECORDING.as_millis()).unwrap_or(u64::MAX) * RATE / 1000;
    if audio.samples < min_samples || wav::is_silent(&wav) {
        return Ok(None);
    }
    match steps::recognize(state.transcriber.as_ref(), &wav, recipe.language.as_deref(), recipe.vocabulary.glossary(), recipe.script).await {
        Ok(r) if r.text.is_empty() => Ok(None),
        Ok(r) => Ok(Some(Recognised { raw_text: r.text, asr_ms: r.asr_ms, asr_model: r.model, segments: 1 })),
        Err(DictationError::NoSpeech) => Ok(None),
        Err(e) => Err(from_dictation(e)),
    }
}

/// Where a long take's segments end, cut by the shell's segmenter (or the core's own when it has
/// none or it cannot run), reading the file in blocks. Blocking.
fn cut_points(path: &Path, samples: u64, factory: Option<&Arc<dyn SegmenterFactory>>) -> std::io::Result<Vec<u64>> {
    let mut segmenter: Box<dyn Segmenter> = match factory.map(|f| f.create()) {
        Some(Ok(segmenter)) => segmenter,
        Some(Err(why)) => {
            tracing::info!(%why, "cutting the long take with the core's fallback");
            Box::new(EnergySegmenter::new())
        }
        None => Box::new(EnergySegmenter::new()),
    };
    let mut file = std::fs::File::open(path)?;
    let mut bytes = vec![0u8; 8192];
    let mut floats = Vec::with_capacity(4096);
    let (mut ends, mut read) = (Vec::new(), 0u64);
    while read < samples {
        let want = usize::try_from((samples - read).min(4096) * 2).unwrap_or(8192);
        file.read_exact(&mut bytes[..want])?;
        floats.clear();
        floats.extend(bytes[..want].as_chunks::<2>().0.iter().map(|b| f32::from(i16::from_le_bytes(*b)) / 32_768.0));
        ends.extend(segmenter.push(&floats));
        read += (want / 2) as u64;
    }
    ends.extend(segmenter.finish());
    Ok(ends)
}

/// A take past two minutes in segments (§22): each recognised in order, a silent one skipped, a
/// failing one tried [`SEGMENT_ATTEMPTS`] times and then left as 「[未识别 …]」. `None` when nothing
/// was recognised and nothing failed.
async fn recognize_long(state: &ServeState, recipe: &Recipe, audio: &PcmFile, cancel: &CancelToken) -> Result<Option<Recognised>, ServeError> {
    let (path, samples, factory) = (audio.path.clone(), audio.samples, state.segmenter.clone());
    let ends = tokio::task::spawn_blocking(move || cut_points(&path, samples, factory.as_ref()))
        .await
        .map_err(|e| ServeError::Internal(format!("分段任务失败：{e}")))?
        .map_err(|e| ServeError::Internal(format!("音频文件无法读取：{e}")))?;
    let mut pieces: Vec<(u64, u64, Option<String>)> = Vec::with_capacity(ends.len());
    let (mut start, mut asr_ms, mut asr_model, mut last_error) = (0u64, 0u64, None, None);
    for end in ends {
        if end <= start {
            continue;
        }
        let wav = read_wav(&audio.path, start, end).await?;
        let mut text = None;
        for _ in 0..SEGMENT_ATTEMPTS {
            if cancel.is_cancelled() {
                return Err(ServeError::Cancelled);
            }
            match long::recognize_segment(state.transcriber.as_ref(), &wav, recipe.language.as_deref(), recipe.vocabulary.glossary(), recipe.script).await {
                Ok(t) => {
                    asr_ms += t.latency_ms;
                    if !t.text.trim().is_empty()
                        && let Some(model) = t.model
                    {
                        asr_model = Some(model);
                    }
                    text = Some(t.text);
                    break;
                }
                Err(e) => {
                    tracing::warn!(start, end, error = %e, "a segment of the long take was not recognised");
                    last_error = Some(e);
                }
            }
        }
        pieces.push((start, end, text));
        start = end;
    }
    let recognised = pieces.iter().any(|p| p.2.as_deref().is_some_and(|t| !t.trim().is_empty()));
    if !recognised {
        return match last_error {
            Some(e) => Err(from_dictation(e)),
            None => Ok(None),
        };
    }
    let segments = u32::try_from(pieces.len()).unwrap_or(u32::MAX);
    Ok(Some(Recognised { raw_text: long::assemble(&pieces, &[]), asr_ms, asr_model, segments }))
}
