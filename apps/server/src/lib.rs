//! `voltip-server`: the local speech service without the app (docs/dictation.md §23.5), for a
//! server with no desktop. It reads the app's data directory — settings, presets, scenes,
//! dictionary, rules, the model library — without ever writing to it, reads the engine keys from
//! the system keychain without moving one, and serves `voltip-serve`'s endpoints until SIGINT or
//! SIGTERM. Nothing here touches a window, an audio device, the device identity or the relay.

pub mod cli;
pub mod exit;

use std::future::Future;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use voltip_asr_local::{Compute, LocalTranscriber, ModelStore, VadSegmenterFactory};
use voltip_core::dictation::{EngineFactory, Refiner, SegmenterFactory, Transcriber};
use voltip_core::serve::{
    Defaults, FileSource, FileSourceConfig, ListenConfig, Service, StateSource, default_token_path, load_or_create_token, not_ready, uploads_dir,
};
use voltip_core::{BuiltIn, CoreConfig, ModelManager, ProviderId, ResolvedEngines, UserSecrets};
use voltip_identity::SecretStore;
use voltip_serve::ServeHandle;

use crate::cli::{Action, Cli, ServeOptions};
use crate::exit::exit_process;

/// The release version (the root `package.json`'s, build.rs).
pub const APP_VERSION: &str = env!("VOLTIP_APP_VERSION");
/// The keychain service the app keeps its secrets under (the desktop shell's `KEYCHAIN_SERVICE`).
pub const KEYCHAIN_SERVICE: &str = "dev.voltip.desktop";
/// Debug builds only: another data directory (tests), as the desktop shell's.
pub const DEV_DATA_DIR_ENV: &str = "VOLTIP_DEV_DATA_DIR";
/// Debug builds only: `memory` reads no keychain (tests), as the desktop shell's.
pub const DEV_SECRET_STORE_ENV: &str = "VOLTIP_DEV_SECRET_STORE";
/// How long the requests in progress may take to finish once a signal asked the server to stop.
pub const GRACE: Duration = Duration::from_secs(30);

/// The app's data directory (the desktop app's: `ProjectDirs` dev / voltip / Voltip).
pub fn data_dir() -> PathBuf {
    if cfg!(debug_assertions)
        && let Some(path) = std::env::var_os(DEV_DATA_DIR_ENV).filter(|path| !path.is_empty())
    {
        return path.into();
    }
    directories::ProjectDirs::from("dev", "voltip", "Voltip").map(|d| d.data_dir().to_path_buf()).unwrap_or_else(|| std::env::temp_dir().join("voltip"))
}

/// What the server is wired with: real engines in production, fakes in tests.
pub struct Wiring {
    /// Builds the clients for a configuration.
    pub factory: EngineFactory,
    /// The local model library.
    pub models: Option<Arc<dyn ModelManager>>,
    /// Where long takes are cut.
    pub segmenter: Option<Arc<dyn SegmenterFactory>>,
    /// The engine keys.
    pub secrets: UserSecrets,
    /// Why the keys could not be read, when they could not.
    pub secrets_notice: Option<String>,
    /// The service compiled into the build.
    pub built_in: BuiltIn,
}

/// Where the local recogniser runs, from the engine settings (as the desktop shell's `compute_of`).
fn compute_of(engines: &ResolvedEngines) -> Compute {
    Compute { device: engines.local_device, gpu: engines.local_gpu.clone(), threads: engines.local_threads.map(usize::from) }
}

/// The clients for a configuration, built as the desktop shell's `build_clients` builds them: the
/// one local recogniser pointed at the selected model, or the cloud client; the cloud clean-up.
pub fn engine_factory(local: LocalTranscriber) -> EngineFactory {
    Arc::new(move |engines: &ResolvedEngines| {
        let transcriber: Arc<dyn Transcriber> = if engines.is_local() {
            let id = engines.local_model.as_ref().map_or(voltip_asr_local::DEFAULT_MODEL_ID, |m| m.id.as_str());
            Arc::new(local.select(id).with_vad_trim(engines.vad_trim).with_compute(compute_of(engines)))
        } else {
            voltip_cloud::remote_transcriber(engines)
        };
        let refiner: Option<Arc<dyn Refiner>> = voltip_cloud::refiner(engines);
        (transcriber, refiner)
    })
}

/// The keychain store, read-only from here (`SecretStore::peek`); a memory store in a debug build
/// asked for one.
fn secret_store() -> Arc<dyn SecretStore> {
    if cfg!(debug_assertions) && std::env::var(DEV_SECRET_STORE_ENV).ok().as_deref() == Some("memory") {
        return Arc::new(voltip_identity::MemorySecretStore::new());
    }
    let user = std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_else(|_| "default".into());
    Arc::new(voltip_identity::KeyringSecretStore::new(KEYCHAIN_SERVICE, user))
}

/// The engine keys from `store`, read once and without moving any; a keychain that cannot be read
/// at all (no session on a headless server) gives none, with one notice.
pub fn read_secrets(store: &dyn SecretStore) -> (UserSecrets, Option<String>) {
    let probe = voltip_core::key_entries().into_iter().next().map(|entry| store.peek(entry));
    match probe {
        Some(Err(e)) => (UserSecrets::default(), Some(format!("系统钥匙串无法读取（{e}）：需要 API 密钥的服务商视为未配置，内置服务与本地模型不受影响"))),
        _ => (voltip_core::peek_user_secrets(store), None),
    }
}

/// The production wiring over `data_dir`'s model library.
pub fn production_wiring(data_dir: &Path) -> Wiring {
    let models_root = CoreConfig::new(data_dir.to_path_buf()).models_root;
    let store = ModelStore::new(models_root.clone());
    let (secrets, secrets_notice) = read_secrets(secret_store().as_ref());
    Wiring {
        factory: engine_factory(LocalTranscriber::new(models_root.clone())),
        models: Some(Arc::new(store)),
        // No store: a missing VAD model is never downloaded; the core's own cutter runs instead.
        segmenter: Some(Arc::new(VadSegmenterFactory::new(models_root, None))),
        secrets,
        secrets_notice,
        built_in: BuiltIn::from_build(),
    }
}

/// The service, ready to serve, and what to warn about.
pub struct Prepared {
    /// The service.
    pub service: Arc<Service>,
    /// The defaults it runs with.
    pub defaults: Defaults,
    /// What the log and `--check` should say.
    pub notices: Vec<String>,
    /// The token file.
    pub token_file: PathBuf,
    /// Why recognition cannot run although it is configured: the cloud service it uses cannot be
    /// reached over HTTPS from this system ([`https_unavailable`]).
    pub unavailable: Option<String>,
}

/// Why this system cannot make HTTPS requests, when it cannot. Cloud recognition and clean-up (the
/// built-in service too) verify the server's certificate with the system's own store, and a minimal
/// system — a container image without ca-certificates — has none: every client then fails to
/// build, and each request with it.
pub fn https_unavailable() -> Option<String> {
    voltip_cloud::http_client_builder()
        .build()
        .err()
        .map(|e| format!("无法建立 HTTPS 连接（{e}）：系统中没有可用的 CA 证书，请安装 ca-certificates（例如 sudo apt-get install ca-certificates）后重新启动"))
}

/// Read the data directory and check the options against it.
pub fn prepare(options: &ServeOptions, data_dir: &Path, wiring: Wiring) -> Result<Prepared, String> {
    let Wiring { factory, models, segmenter, secrets, secrets_notice, built_in } = wiring;
    let config = FileSourceConfig {
        data_dir: data_dir.to_path_buf(),
        platform: voltip_protocol::Platform::current(),
        overrides: options.overrides.clone(),
        secrets,
        built_in,
        models,
        factory,
        segmenter,
    };
    let (source, mut notices) = FileSource::open(config)?;
    notices.extend(secrets_notice);
    let state = source.current();
    let defaults = options.choices.resolve(&state.presets, &state.scenes).map_err(|e| format!("启动参数无效：{e}"))?;
    if state.engines.asr_streams {
        notices.push("所选识别模型只支持实时流式：整段上传的音频会按实时速度处理，等待时间约等于音频时长；建议改用整段识别的模型".to_owned());
    }
    let (cloud_asr, cloud_refine) = (!state.engines.is_local(), state.engines.refine.is_some());
    let https = if cloud_asr || cloud_refine { https_unavailable() } else { None };
    let unavailable = https.clone().filter(|_| cloud_asr);
    if let Some(reason) = https.filter(|_| !cloud_asr) {
        notices.push(format!("{reason}；在此之前 AI 润色不可用，识别结果按原文返回"));
    }
    let service = Arc::new(Service::new(Arc::new(source) as Arc<dyn StateSource>, defaults.clone()));
    let token_file = options.token_file.clone().unwrap_or_else(|| default_token_path(data_dir));
    Ok(Prepared { service, defaults, notices, token_file, unavailable })
}

fn provider(id: ProviderId) -> &'static str {
    match id {
        ProviderId::Builtin => "内置服务",
        ProviderId::Local => "本地模型",
        other => other.as_str(),
    }
}

/// `--check`: what the service would run with; 0 when recognition can run.
pub fn check(options: &ServeOptions, data_dir: &Path, wiring: Wiring, out: &mut dyn Write) -> i32 {
    let prepared = match prepare(options, data_dir, wiring) {
        Ok(p) => p,
        Err(e) => {
            let _ = writeln!(out, "voltip-server: {e}");
            return 1;
        }
    };
    let state = prepared.service.state();
    let engines = &state.engines;
    let _ = writeln!(out, "voltip-server {APP_VERSION}");
    let _ = writeln!(out, "数据目录：{}", data_dir.display());
    let _ = writeln!(out, "地址：http://{}/v1{}", options.listen, if options.listen.ip().is_loopback() { "（仅本机）" } else { "（允许其他电脑访问）" });
    let _ = writeln!(out, "令牌文件：{}", prepared.token_file.display());
    let asr = match &engines.local_model {
        Some(m) => format!("{} · {}（{}，{}）", provider(engines.asr_provider), m.name, m.id, if m.installed { "已下载" } else { "未下载" }),
        None => format!("{} · {}", provider(engines.asr_provider), engines.asr_model),
    };
    let _ = writeln!(out, "语音识别：{asr}");
    let llm = match (&engines.refine, engines.llm_provider) {
        (Some(remote), Some(id)) => format!("{} · {}", provider(id), remote.model),
        _ => "未配置".to_owned(),
    };
    let _ = writeln!(out, "AI 润色：{llm}");
    let defaults = &prepared.defaults;
    let scene = match (defaults.scene, &defaults.app) {
        (Some(id), _) => state
            .scenes
            .iter()
            .find(|s| s.id == id)
            .map_or_else(|| id.to_string(), |s| s.builtin.map_or_else(|| s.name.clone(), |c| c.display_name().to_owned())),
        (None, Some(app)) => format!("按应用 {app} 匹配"),
        (None, None) => "无".to_owned(),
    };
    let preset = match defaults.preset {
        Some(id) => voltip_core::presets::resolve(id, &state.presets).0.to_ref().name,
        None => "跟随设置".to_owned(),
    };
    let refine = match defaults.refine {
        Some(true) => "开",
        Some(false) => "关",
        None => "跟随设置",
    };
    let _ = writeln!(out, "默认处理方式：场景 {scene}；预设 {preset}；AI 润色 {refine}");
    let language = defaults.language.as_deref().map_or("跟随场景、请求与设置", |l| if l == "auto" { "自动识别" } else { l });
    let script = defaults.script.map_or("跟随设置", |s| s.as_str());
    let _ = writeln!(out, "语言：{language}；中文字形：{script}");
    let _ = writeln!(out, "单次音频上限：{} 分钟；同时处理：{} 个", options.max_minutes, options.concurrency);
    if defaults.language.is_none() {
        let _ = writeln!(out, "提示：Paseo 未设置 language 时会发送 en；可在 Paseo 中设置 language，或用 --language 指定");
    }
    for notice in &prepared.notices {
        let _ = writeln!(out, "警告：{notice}");
    }
    if prepared.token_file.exists()
        && let Ok((_, Some(warning))) = load_or_create_token(&prepared.token_file)
    {
        let _ = writeln!(out, "警告：{warning}");
    }
    match not_ready(engines).or_else(|| prepared.unavailable.clone()) {
        None => {
            let _ = writeln!(out, "状态：可用");
            0
        }
        Some(reason) => {
            let _ = writeln!(out, "状态：不可用：{reason}");
            1
        }
    }
}

/// Run the service until `shutdown` (made inside the runtime) resolves; `listening` learns the
/// bound address. The requests in progress then have [`GRACE`]. Returns the exit status.
pub fn serve<F, S>(
    options: &ServeOptions,
    data_dir: &Path,
    wiring: Wiring,
    shutdown: S,
    listening: Option<std::sync::mpsc::Sender<std::net::SocketAddr>>,
) -> i32
where
    S: FnOnce() -> F,
    F: Future<Output = ()> + Send + 'static,
{
    serve_with_grace(options, data_dir, wiring, shutdown, listening, GRACE)
}

/// [`serve`] with another grace period (tests).
pub fn serve_with_grace<F, S>(
    options: &ServeOptions,
    data_dir: &Path,
    wiring: Wiring,
    shutdown: S,
    listening: Option<std::sync::mpsc::Sender<std::net::SocketAddr>>,
    grace: Duration,
) -> i32
where
    S: FnOnce() -> F,
    F: Future<Output = ()> + Send + 'static,
{
    let prepared = match prepare(options, data_dir, wiring) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("{e}");
            eprintln!("voltip-server: {e}");
            return 1;
        }
    };
    for notice in &prepared.notices {
        tracing::warn!("{notice}");
    }
    // Every request would fail: say why and stop, so that a supervisor shows it at once.
    if let Some(reason) = &prepared.unavailable {
        tracing::error!("{reason}");
        eprintln!("voltip-server: 语音识别不可用：{reason}");
        return 1;
    }
    let token = match load_or_create_token(&prepared.token_file) {
        Ok((token, warning)) => {
            if let Some(warning) = warning {
                tracing::warn!("{warning}");
            }
            token
        }
        Err(e) => {
            eprintln!("voltip-server: {e}");
            return 1;
        }
    };
    if !options.listen.ip().is_loopback() {
        tracing::warn!(listen = %options.listen, "listening beyond this computer: the service speaks plain HTTP and the token travels unencrypted");
    }
    let uploads = uploads_dir(data_dir);
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("voltip-server: runtime: {e}");
            return 1;
        }
    };
    let config = ListenConfig { addr: options.listen, token, concurrency: options.concurrency, max_minutes: options.max_minutes, uploads_dir: uploads.clone() };
    let service = prepared.service.clone();
    let preload = options.preload;
    let code = runtime.block_on(async move {
        let handle = match ServeHandle::new(&config, service.clone()) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("voltip-server: {}: {e}", config.uploads_dir.display());
                return 1;
            }
        };
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let (addr, task) = match handle
            .clone()
            .serve(config.addr, async move {
                let _ = stopped.await;
            })
            .await
        {
            Ok(bound) => bound,
            Err(e) => {
                eprintln!("voltip-server: 无法监听 {}：{e}", config.addr);
                return 1;
            }
        };
        eprintln!("voltip-server: listening on http://{addr}/v1 (token in {})", prepared.token_file.display());
        if let Some(listening) = listening {
            let _ = listening.send(addr);
        }
        if preload {
            service.warm();
        }
        shutdown().await;
        tracing::info!("stopping: no new requests; the ones in progress have {} s", grace.as_secs());
        handle.close_queue();
        let _ = stop.send(());
        if tokio::time::timeout(grace, task).await.is_err() {
            tracing::warn!(abandoned = handle.active(), "requests still in progress were abandoned");
            handle.cancel_all();
        }
        0
    });
    // A recognition still running on a blocking thread cannot be stopped: it is left behind.
    runtime.shutdown_timeout(Duration::from_secs(1));
    let _ = voltip_serve::clear_uploads(&uploads);
    code
}

/// SIGINT or SIGTERM (Ctrl+C where there is no SIGTERM).
pub async fn signal() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
        let terminate = async move {
            match term.as_mut() {
                Some(t) => {
                    t.recv().await;
                }
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            () = terminate => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// Run `action` over `data_dir`; the exit status.
pub fn run(action: &Action, data_dir: &Path, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let models_root = CoreConfig::new(data_dir.to_path_buf()).models_root;
    match action {
        Action::ListModels => voltip_asr_local::cli::list_models(&models_root, out).code(),
        Action::DownloadModel { id } => voltip_asr_local::cli::download_model(&ModelStore::new(models_root), id, out, err).code(),
        Action::ListCompute => voltip_asr_local::cli::list_compute(out).code(),
        Action::PrintToken { token_file } => {
            let path = token_file.clone().unwrap_or_else(|| default_token_path(data_dir));
            match load_or_create_token(&path) {
                Ok((token, warning)) => {
                    if let Some(warning) = warning {
                        let _ = writeln!(err, "voltip-server: {warning}");
                    }
                    let _ = writeln!(out, "{token}");
                    0
                }
                Err(e) => {
                    let _ = writeln!(err, "voltip-server: {e}");
                    1
                }
            }
        }
        Action::Check(options) => check(options, data_dir, production_wiring(data_dir), out),
        Action::Serve(options) => serve(options, data_dir, production_wiring(data_dir), signal, None),
    }
}

/// The program: parse, run, leave through [`exit_process`].
pub fn main_entry() -> ! {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let ansi = std::io::IsTerminal::is_terminal(&std::io::stderr()) && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty());
    let _ = tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).with_ansi(ansi).try_init();
    let cli = match Cli::parse_args(std::env::args_os()) {
        Ok(cli) => cli,
        Err(e) => {
            let _ = e.print();
            exit_process(e.exit_code());
        }
    };
    let action = match cli.action() {
        Ok(action) => action,
        Err(message) => {
            eprintln!("voltip-server: {message}");
            exit_process(2);
        }
    };
    let code = run(&action, &data_dir(), &mut std::io::stdout(), &mut std::io::stderr());
    exit_process(code)
}
