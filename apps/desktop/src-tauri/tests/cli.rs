#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The desktop command line (docs/dictation.md §13): flag parsing, the remote controls the
//! single-instance plugin forwards, and the headless `--transcribe-file` / `--list-models` paths
//! through the library functions (not a subprocess) with a fake recogniser. The one real run is
//! `#[ignore]` and needs a downloaded model.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use voltip_asr_local::{
    CATALOGUE, CATALOGUE_VERSION, Capability, Compute, Engine, LocalDevice, LocalTranscriber, MANIFEST_FILE, Manifest, ModelEntry, ModelFile, ModelStore,
    Recognizer, RecognizerLoader, Source, Tier, entry,
};
use voltip_core::dictation::fakes::speech_recording;
use voltip_core::{ChineseScript, DEFAULT_LOCAL_MODEL_ID, EdgeSource, EngineSettings, ProviderId, Settings, SettingsStore, TakeKind};
use voltip_desktop_lib::cli::{
    Action, Cli, ExitCode, Remote, TranscribeOutput, compute_for, default_model, download_model, list_compute, list_devices, list_models, remote_from_args,
    run_headless, transcribe_file,
};
use voltip_tauri_bridge::UiCommand;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
    Cli::parse_args(std::iter::once("voltip").chain(args.iter().copied()))
}

#[test]
fn cli_parses_flags_into_actions_and_remotes() {
    assert_eq!(parse(&[]).unwrap().action(), Action::Gui { start_hidden: false, remote: None });
    assert_eq!(parse(&["--toggle"]).unwrap().action(), Action::Gui { start_hidden: false, remote: Some(Remote::Toggle) });
    assert_eq!(parse(&["--cancel"]).unwrap().action(), Action::Gui { start_hidden: false, remote: Some(Remote::Cancel) });
    assert_eq!(parse(&["--start-hidden"]).unwrap().action(), Action::Gui { start_hidden: true, remote: None });
    assert_eq!(parse(&["--start-hidden", "--toggle"]).unwrap().action(), Action::Gui { start_hidden: true, remote: Some(Remote::Toggle) });
    assert_eq!(parse(&["--list-devices"]).unwrap().action(), Action::ListDevices);
    assert_eq!(parse(&["--list-models"]).unwrap().action(), Action::ListModels);
    assert_eq!(
        parse(&["--transcribe-file", "a.wav"]).unwrap().action(),
        Action::TranscribeFile { path: PathBuf::from("a.wav"), model: None, json: false, device: None, gpu: None, threads: None }
    );
    assert_eq!(
        parse(&["--transcribe-file", "/tmp/a.wav", "--model", "sense-voice-small", "--json"]).unwrap().action(),
        Action::TranscribeFile {
            path: PathBuf::from("/tmp/a.wav"),
            model: Some("sense-voice-small".into()),
            json: true,
            device: None,
            gpu: None,
            threads: None
        }
    );
    // docs/dictation.md §10.6: the compute flags ride on --transcribe-file and override the settings.
    assert_eq!(parse(&["--list-compute"]).unwrap().action(), Action::ListCompute);
    assert_eq!(
        parse(&["--transcribe-file", "a.wav", "--device", "gpu", "--gpu", "Vulkan0", "--threads", "8"]).unwrap().action(),
        Action::TranscribeFile {
            path: PathBuf::from("a.wav"),
            model: None,
            json: false,
            device: Some(LocalDevice::Gpu),
            gpu: Some("Vulkan0".into()),
            threads: Some(8)
        }
    );
    for bad in [
        &["--transcribe-file", "a.wav", "--device", "npu"][..],
        &["--transcribe-file", "a.wav", "--threads", "0"],
        &["--device", "cpu"],
        &["--list-compute", "--list-models"],
    ] {
        assert!(parse(bad).is_err(), "{bad:?}");
    }
    let settings =
        voltip_core::EngineSettings { local_device: LocalDevice::Cpu, local_gpu: Some("Metal".into()), local_threads: Some(6), ..Default::default() };
    assert_eq!(compute_for(&settings, None, None, None), Compute { device: LocalDevice::Cpu, gpu: Some("Metal".into()), threads: Some(6) });
    assert_eq!(
        compute_for(&settings, Some(LocalDevice::Gpu), Some("Vulkan1"), Some(2)),
        Compute { device: LocalDevice::Gpu, gpu: Some("Vulkan1".into()), threads: Some(2) }
    );
    let mut out = Vec::new();
    assert_eq!(list_compute(&mut out), ExitCode::Ok);
    let listing = String::from_utf8(out).unwrap();
    assert!(listing.starts_with("cpu\t"), "{listing}");
    assert!(listing.lines().skip(1).all(|l| l.starts_with("gpu\t") && l.split('\t').count() == 5), "{listing}");
    // Conflicts and dependencies are usage errors, returned rather than printed.
    for bad in [
        &["--toggle", "--cancel"][..],
        &["--toggle", "--list-models"],
        &["--list-devices", "--list-models"],
        &["--json"],
        &["--model", "x"],
        &["--transcribe-file"],
        &["--transcribe-file", "a.wav", "--start-hidden"],
        &["--no-such-flag"],
    ] {
        assert!(parse(bad).is_err(), "{bad:?}");
    }
    // `--help` / `--version` are clap's own exits, still errors from `parse_args`.
    assert_eq!(parse(&["--help"]).unwrap_err().kind(), clap::error::ErrorKind::DisplayHelp);
    assert_eq!(parse(&["--version"]).unwrap_err().kind(), clap::error::ErrorKind::DisplayVersion);
    // What the running instance does with a second invocation's argv.
    let argv = |a: &[&str]| std::iter::once("voltip").chain(a.iter().copied()).map(str::to_owned).collect::<Vec<_>>();
    assert_eq!(remote_from_args(&argv(&["--toggle"])), Some(Remote::Toggle));
    assert_eq!(remote_from_args(&argv(&["--cancel"])), Some(Remote::Cancel));
    assert_eq!(remote_from_args(&argv(&[])), None, "a plain second launch focuses the window");
    assert_eq!(remote_from_args(&argv(&["--start-hidden"])), None);
    assert_eq!(remote_from_args(&argv(&["--bogus"])), None, "garbage is ignored, never a panic");
    assert!(matches!(Remote::Toggle.command(), UiCommand::HotkeyEdge { pressed: true, source: EdgeSource::Cli, purpose: TakeKind::Dictation, .. }));
    assert!(matches!(Remote::Cancel.command(), UiCommand::DictationCancel));
    assert_eq!((ExitCode::Ok.code(), ExitCode::Failed.code(), ExitCode::BadInput.code()), (0, 1, 2));
}

/// docs/dictation.md §19: `--edit-toggle` is the voice-edit key's remote control — a CLI press
/// with `purpose: edit` — and conflicts with the other actions like `--toggle` does.
#[test]
fn edit_toggle_flag_parses_into_an_edit_edge_and_conflicts_with_the_other_actions() {
    assert_eq!(parse(&["--edit-toggle"]).unwrap().action(), Action::Gui { start_hidden: false, remote: Some(Remote::EditToggle) });
    assert_eq!(parse(&["--start-hidden", "--edit-toggle"]).unwrap().action(), Action::Gui { start_hidden: true, remote: Some(Remote::EditToggle) });
    for bad in [
        &["--edit-toggle", "--toggle"][..],
        &["--edit-toggle", "--cancel"],
        &["--edit-toggle", "--list-models"],
        &["--edit-toggle", "--transcribe-file", "a.wav"],
    ] {
        assert!(parse(bad).is_err(), "{bad:?}");
    }
    let argv = |a: &[&str]| std::iter::once("voltip").chain(a.iter().copied()).map(str::to_owned).collect::<Vec<_>>();
    assert_eq!(remote_from_args(&argv(&["--edit-toggle"])), Some(Remote::EditToggle));
    assert!(matches!(Remote::EditToggle.command(), UiCommand::HotkeyEdge { pressed: true, source: EdgeSource::Cli, purpose: TakeKind::Edit, .. }));
    let help = parse(&["--help"]).unwrap_err().to_string();
    assert!(help.contains("--edit-toggle"), "{help}");
}

/// Echoes the model id and the sample count; fails on silence like a real engine would on garbage.
struct Echo {
    id: String,
}

impl Recognizer for Echo {
    fn transcribe(&mut self, sample_rate: u32, samples: &[f32]) -> Result<String, String> {
        if samples.iter().all(|s| *s == 0.0) {
            return Err("all zero".into());
        }
        Ok(format!(" {}@{sample_rate}:{} ", self.id, samples.len()))
    }
}

struct FakeLoader;

impl RecognizerLoader for FakeLoader {
    fn load(&self, entry: &ModelEntry, dir: &Path, _language: Option<&str>, _compute: &Compute) -> Result<Box<dyn Recognizer>, String> {
        assert!(dir.ends_with(entry.id));
        if entry.engine == Engine::ZipformerStreaming {
            return Err("streaming model".into());
        }
        Ok(Box::new(Echo { id: entry.id.to_owned() }))
    }
}

/// Pretend `id` is installed under `root` (manifest + right-sized files), as the store would.
fn install(root: &Path, id: &str) {
    let e = entry(id).unwrap();
    let dir = root.join(e.id);
    std::fs::create_dir_all(&dir).unwrap();
    for f in e.files() {
        std::fs::File::create(dir.join(f.name)).unwrap().set_len(f.size).unwrap();
    }
    let manifest = Manifest {
        id: e.id.into(),
        version: CATALOGUE_VERSION,
        downloaded_at: 1,
        files: e.files().iter().map(|f| (f.name.to_owned(), f.sha256.to_owned())).collect(),
    };
    std::fs::write(dir.join(MANIFEST_FILE), serde_json::to_vec(&manifest).unwrap()).unwrap();
}

fn run(transcriber: &LocalTranscriber, path: &Path, json: bool) -> (ExitCode, String, String) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = transcribe_file(transcriber, path, json, Some("zh"), ChineseScript::Simplified, &mut out, &mut err);
    (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
}

/// Answers in Traditional characters, like Qwen3-ASR does for some Mandarin (docs/dictation.md §17).
struct TraditionalLoader;

impl RecognizerLoader for TraditionalLoader {
    fn load(&self, _entry: &ModelEntry, _dir: &Path, _language: Option<&str>, _compute: &Compute) -> Result<Box<dyn Recognizer>, String> {
        struct Traditional;
        impl Recognizer for Traditional {
            fn transcribe(&mut self, _sample_rate: u32, _samples: &[f32]) -> Result<String, String> {
                Ok("開放時間：早上九點至下午五點。".into())
            }
        }
        Ok(Box::new(Traditional))
    }
}

/// `--transcribe-file` prints the text in the script the settings ask for, like the pipeline
/// injects it (docs/dictation.md §17): Simplified by default, untouched with `as_is`.
#[test]
fn transcribe_file_normalises_the_chinese_script() {
    let dir = tempfile::tempdir().unwrap();
    let models = dir.path().join("models");
    let wav = dir.path().join("sample.wav");
    std::fs::write(&wav, speech_recording(1500).wav).unwrap();
    install(&models, "sense-voice-small");
    let t = LocalTranscriber::with_loader(&models, CATALOGUE, Arc::new(TraditionalLoader)).select("sense-voice-small");
    for (script, expect) in [(ChineseScript::Simplified, "开放时间：早上九点至下午五点。"), (ChineseScript::AsIs, "開放時間：早上九點至下午五點。")]
    {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(transcribe_file(&t, &wav, false, None, script, &mut out, &mut err), ExitCode::Ok);
        assert_eq!(String::from_utf8(out).unwrap(), format!("{expect}\n"), "{script:?}");
    }
}

/// `voltip --transcribe-file` without a window: the text (or the JSON) on stdout and exit 0 with an
/// installed model; a missing model or an unreadable file is a non-zero exit with an empty stdout
/// and the reason on stderr.
#[test]
fn transcribe_file_headless_prints_text_or_json_and_fails_with_an_empty_stdout_without_the_model() {
    let dir = tempfile::tempdir().unwrap();
    let models = dir.path().join("models");
    let wav = dir.path().join("sample.wav");
    std::fs::write(&wav, speech_recording(1500).wav).unwrap();
    let t = LocalTranscriber::with_loader(&models, CATALOGUE, Arc::new(FakeLoader)).select("sense-voice-small");

    // Not installed: exit 1, nothing on stdout, the documented reason on stderr.
    let (code, out, err) = run(&t, &wav, false);
    assert_eq!(code, ExitCode::Failed);
    assert!(out.is_empty(), "stdout must stay empty: {out:?}");
    assert!(err.contains("本地模型未下载"), "{err}");

    install(&models, "sense-voice-small");
    let (code, out, err) = run(&t, &wav, false);
    assert_eq!(code, ExitCode::Ok, "{err}");
    assert_eq!(out, "sense-voice-small@16000:24000\n", "trimmed text plus a newline");
    assert!(err.is_empty());

    let (code, out, _) = run(&t, &wav, true);
    assert_eq!(code, ExitCode::Ok);
    let parsed: TranscribeOutput = serde_json::from_str(out.trim_end()).unwrap();
    assert_eq!(parsed.text, "sense-voice-small@16000:24000");
    assert_eq!(parsed.model, "sense-voice-small");
    assert!(parsed.latency_ms < 10_000);
    assert!(out.starts_with(r#"{"text":"sense-voice-small@16000:24000","model":"sense-voice-small","latency_ms":"#), "{out}");

    // Unreadable input: exit 2, empty stdout.
    let (code, out, err) = run(&t, &dir.path().join("missing.wav"), true);
    assert_eq!(code, ExitCode::BadInput);
    assert!(out.is_empty());
    assert!(err.contains("missing.wav"), "{err}");
    // Garbage input is refused by the decoder: exit 1, empty stdout.
    let garbage = dir.path().join("garbage.wav");
    std::fs::write(&garbage, b"not a wav").unwrap();
    let (code, out, _) = run(&t, &garbage, false);
    assert_eq!(code, ExitCode::Failed);
    assert!(out.is_empty());
    // A model the catalogue does not know: exit 1.
    let ghost = LocalTranscriber::with_loader(&models, CATALOGUE, Arc::new(FakeLoader)).select("ghost");
    let (code, out, err) = run(&ghost, &wav, false);
    assert_eq!(code, ExitCode::Failed);
    assert!(out.is_empty());
    assert!(err.contains("ghost"), "{err}");

    // `--model` absent: the settings' local model when in local mode, else the catalogue default.
    assert_eq!(default_model(dir.path()), DEFAULT_LOCAL_MODEL_ID, "no settings file yet");
    let store = SettingsStore::new(dir.path());
    store
        .save(&Settings { engines: EngineSettings { local_model: Some("paraformer-zh".into()), ..EngineSettings::default() }, ..Settings::default() })
        .unwrap();
    assert_eq!(default_model(dir.path()), DEFAULT_LOCAL_MODEL_ID, "cloud mode ignores the local selection");
    store
        .save(&Settings {
            engines: EngineSettings { asr_provider: ProviderId::Local, local_model: Some("paraformer-zh".into()), ..EngineSettings::default() },
            ..Settings::default()
        })
        .unwrap();
    assert_eq!(default_model(dir.path()), "paraformer-zh");

    // `--list-models` over the same library: every catalogue entry with its install state.
    let mut out = Vec::new();
    assert_eq!(list_models(&models, &mut out), ExitCode::Ok);
    let listing = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = listing.lines().collect();
    // One line per model the store lists (the catalogue may carry implicit entries it hides).
    assert_eq!(lines.len(), voltip_asr_local::ModelStore::new(models.clone()).scan().len());
    assert!(lines.len() >= 5, "{listing}");
    assert!(lines.iter().any(|l| l.starts_with("sense-voice-small\tinstalled\t")), "{listing}");
    assert!(lines.iter().any(|l| l.starts_with("qwen3-asr-0.6b\tnot_installed\t")), "{listing}");
    // The dispatcher the binary uses, over the app data dir (`models/` under it).
    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert_eq!(run_headless(&Action::ListModels, dir.path(), &mut out, &mut err), ExitCode::Ok);
    assert_eq!(String::from_utf8(out).unwrap(), listing);
    // The dispatcher's real-engine path over a model that is not installed: non-zero, empty stdout.
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let action = Action::TranscribeFile { path: wav.clone(), model: Some("qwen3-asr-0.6b".into()), json: true, device: None, gpu: None, threads: None };
    assert_eq!(run_headless(&action, dir.path(), &mut out, &mut err), ExitCode::Failed);
    assert!(out.is_empty());
    assert!(String::from_utf8(err).unwrap().contains("本地模型未下载"));
    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert_eq!(run_headless(&Action::Gui { start_hidden: false, remote: None }, dir.path(), &mut out, &mut err), ExitCode::Ok);
    assert!(out.is_empty() && err.is_empty());
    // `--list-devices` asks the real sound system: on a runner without one it is an honest error
    // (nothing on stdout), otherwise one `id<TAB>name` line per device.
    let (mut out, mut err) = (Vec::new(), Vec::new());
    match list_devices(&mut out, &mut err) {
        ExitCode::Ok => assert!(String::from_utf8(out).unwrap().lines().all(|l| l.contains('\t'))),
        ExitCode::Failed => {
            assert!(out.is_empty());
            assert!(String::from_utf8(err).unwrap().starts_with("voltip: audio:"));
        }
        ExitCode::BadInput => panic!("not an input error"),
    }
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run_headless(&Action::ListDevices, dir.path(), &mut out, &mut err);
    assert!(matches!(code, ExitCode::Ok | ExitCode::Failed));
}

/// The real thing: `voltip --transcribe-file <wav> --json` against a downloaded model staged as the
/// store would install it (docs/dictation.md §13). Prints valid JSON with Chinese text and exits 0.
///
/// ```text
/// VOLTIP_LOCAL_MODEL_DIR=/path/to/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17 \
/// VOLTIP_LOCAL_SAMPLE_WAV=/tmp/voltip-sample.wav \
///   cargo test -p voltip-desktop --test cli -- --ignored --nocapture
/// ```
#[test]
#[ignore = "needs VOLTIP_LOCAL_MODEL_DIR (a downloaded sherpa-onnx model) and VOLTIP_LOCAL_SAMPLE_WAV"]
fn real_transcribe_file_prints_json_and_exits_zero() {
    let model_dir = PathBuf::from(std::env::var("VOLTIP_LOCAL_MODEL_DIR").expect("VOLTIP_LOCAL_MODEL_DIR"));
    let id = if model_dir.to_string_lossy().contains("paraformer") { "paraformer-zh" } else { "sense-voice-small" };
    let e = entry(id).unwrap();
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join(e.id);
    std::fs::create_dir_all(&dir).unwrap();
    for f in e.files() {
        let src = model_dir.join(f.name);
        std::fs::hard_link(&src, dir.join(f.name)).or_else(|_| std::fs::copy(&src, dir.join(f.name)).map(|_| ())).unwrap();
    }
    let manifest = Manifest {
        id: e.id.into(),
        version: CATALOGUE_VERSION,
        downloaded_at: 1,
        files: e.files().iter().map(|f| (f.name.to_owned(), f.sha256.to_owned())).collect(),
    };
    std::fs::write(dir.join(MANIFEST_FILE), serde_json::to_vec(&manifest).unwrap()).unwrap();
    let wav = PathBuf::from(std::env::var("VOLTIP_LOCAL_SAMPLE_WAV").unwrap_or_else(|_| "/tmp/voltip-sample.wav".into()));
    let t = LocalTranscriber::new(root.path()).select(e.id);
    let (code, out, err) = run(&t, &wav, true);
    assert_eq!(code, ExitCode::Ok, "{err}");
    let parsed: TranscribeOutput = serde_json::from_str(out.trim_end()).unwrap();
    println!("model={} latency_ms={} text={}", parsed.model, parsed.latency_ms, parsed.text);
    assert_eq!(parsed.model, e.id);
    assert!(parsed.text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)), "expected Chinese text: {}", parsed.text);
}

// --- `--download-model` -------------------------------------------------------------------------

const TINY_REPO: &str = "voltip/cli-tiny";
const TINY_TOKENS: &[u8] = b"<blk> 0\n<sos/eos> 1\n";

fn tiny_model_bytes() -> Vec<u8> {
    b"voltip-cli-test-model-bytes!".repeat(40)[..1000].to_vec()
}

/// sha256 of `tiny_model_bytes()` and `TINY_TOKENS` (computed once with `sha256sum`).
static TINY_FILES: [ModelFile; 2] = [
    ModelFile { name: "model.int8.onnx", size: 1000, sha256: "163c12248c53557c557648df8d33968697059f7909a2750a173b5e1ae44a4929" },
    ModelFile { name: "tokens.txt", size: 20, sha256: "a20039fabfb5e4c829d52e551804b74224aad4420b5e664248ac5ae6b3187bae" },
];

static TINY_CATALOGUE: [ModelEntry; 1] = [ModelEntry {
    id: "tiny",
    name: "Tiny",
    engine: Engine::SenseVoice,
    tier: Tier::Light,
    capabilities: &[Capability::Offline],
    languages: &["zh"],
    description: "test",
    recommended: true,
    repo: TINY_REPO,
    files: &TINY_FILES,
}];

fn download(store: &ModelStore, id: &str) -> (ExitCode, String, String) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = download_model(store, id, &mut out, &mut err);
    (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
}

#[test]
fn download_model_flag_parses_and_conflicts_with_the_other_actions() {
    assert_eq!(parse(&["--download-model", "sense-voice-small"]).unwrap().action(), Action::DownloadModel { id: "sense-voice-small".into() });
    for bad in [
        &["--download-model"][..],
        &["--download-model", "x", "--transcribe-file", "a.wav"],
        &["--download-model", "x", "--list-models"],
        &["--download-model", "x", "--list-devices"],
        &["--download-model", "x", "--toggle"],
        &["--download-model", "x", "--start-hidden"],
    ] {
        assert!(parse(bad).is_err(), "{bad:?}");
    }
}

/// `voltip --download-model <id>` prepares a model headless with the app's own downloader (what a
/// fresh machine or CI runner needs before `--transcribe-file`): a dead first source falls through
/// to the next, progress goes to stderr, stdout carries only `id<TAB>installed<TAB>dir`, and a
/// second run over the verified files prints the same line.
#[test]
fn download_model_installs_through_the_source_chain_and_keeps_stdout_to_the_result_line() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (dead, live) = rt.block_on(async {
        let dead = MockServer::start().await; // nothing mounted: every GET is a 404
        let live = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/{TINY_REPO}/model.int8.onnx")))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(tiny_model_bytes()))
            .mount(&live)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/{TINY_REPO}/tokens.txt")))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(TINY_TOKENS))
            .mount(&live)
            .await;
        (dead, live)
    });
    let root = tempfile::tempdir().unwrap();
    let store = ModelStore::with_sources(root.path(), &TINY_CATALOGUE, vec![Source::Base(dead.uri()), Source::Base(live.uri())]);

    let (code, out, err) = download(&store, "tiny");
    assert_eq!(code, ExitCode::Ok, "{err}");
    let dir = root.path().join("tiny");
    assert_eq!(out, format!("tiny\tinstalled\t{}\n", dir.display()));
    assert!(err.contains("model.int8.onnx") && err.contains("(100%)"), "progress belongs on stderr: {err}");
    assert!(dir.join(MANIFEST_FILE).is_file());
    assert_eq!(std::fs::read(dir.join("model.int8.onnx")).unwrap(), tiny_model_bytes());

    let (again, out_again, _) = download(&store, "tiny");
    assert_eq!((again, out_again), (ExitCode::Ok, out));
}

/// An id that is not in the catalogue is a usage error; a chain where every source fails is a
/// failure with the reason on stderr, an empty stdout and nothing installed.
#[test]
fn download_model_rejects_an_unknown_id_and_reports_a_dead_chain_with_an_empty_stdout() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dead = rt.block_on(MockServer::start());
    let root = tempfile::tempdir().unwrap();
    let store = ModelStore::with_sources(root.path(), &TINY_CATALOGUE, vec![Source::Base(dead.uri())]);

    let (code, out, err) = download(&store, "no-such-model");
    assert_eq!(code, ExitCode::BadInput);
    assert!(out.is_empty(), "{out}");
    assert!(err.contains("no-such-model"), "{err}");

    let (code, out, err) = download(&store, "tiny");
    assert_eq!(code, ExitCode::Failed);
    assert!(out.is_empty(), "{out}");
    assert!(err.contains("tiny"), "{err}");
    assert!(!root.path().join("tiny").join(MANIFEST_FILE).exists());
}

// --- the real binary ----------------------------------------------------------------------------

/// Point every per-user directory the app could resolve at `dir` (Linux XDG, macOS HOME, Windows).
fn isolated(command: &mut std::process::Command, dir: &Path) {
    command.env("XDG_DATA_HOME", dir).env("XDG_CONFIG_HOME", dir).env("HOME", dir).env("APPDATA", dir).env("LOCALAPPDATA", dir);
}

/// Regression (2026-09-26): `run()` logged to stdout and held the stdout lock across a headless
/// action, so `--transcribe-file` with a real model deadlocked on the recogniser's first log line,
/// and any log line would have been mixed into `--json`. Headless stdout carries the command's
/// output only; logs (debug level here) go to stderr.
#[test]
fn regression_headless_stdout_carries_only_the_output_and_logs_go_to_stderr() {
    let data = tempfile::tempdir().unwrap();
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_voltip-desktop"));
    command.arg("--list-models").env("RUST_LOG", "debug");
    isolated(&mut command, data.path());
    let output = command.output().unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    let rows: Vec<&str> = stdout.lines().collect();
    assert_eq!(rows.len(), CATALOGUE.iter().filter(|e| e.tier.is_visible()).count(), "stdout must be the listing only:\n{stdout}");
    for row in &rows {
        let cols: Vec<&str> = row.split('\t').collect();
        assert_eq!(cols.len(), 3, "{row:?}");
        assert!(CATALOGUE.iter().any(|e| e.id == cols[0]) && cols[1] == "not_installed", "{row:?}");
    }
    assert!(stderr.contains("headless action"), "the debug log line belongs on stderr:\n{stderr}");
    // Piped stderr is a log file or a script's capture: no ANSI colour codes (2026-09-26, every
    // smoke summary carried `\x1b[2m…` around each field).
    assert!(!stderr.contains('\u{1b}'), "ANSI escape codes on a non-terminal stderr:\n{stderr:?}");
}

/// Regression (2026-09-27): on Linux a headless run leaves through `_exit` (src/exit.rs; the NVIDIA
/// Vulkan driver's teardown aborted a normal exit). The output must still arrive in full and the
/// exit status must still be the command's own.
#[test]
fn regression_a_headless_run_keeps_its_output_and_its_exit_status() {
    let data = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_voltip-desktop"));
        command.args(args).env("RUST_LOG", "off");
        isolated(&mut command, data.path());
        command.output().unwrap()
    };
    let compute = run(&["--list-compute"]);
    assert_eq!(compute.status.code(), Some(ExitCode::Ok.code()), "{}", String::from_utf8_lossy(&compute.stderr));
    let first = String::from_utf8(compute.stdout).unwrap();
    assert!(first.lines().next().is_some_and(|l| l.starts_with("cpu\t")), "{first:?}");
    let unknown = run(&["--download-model", "no-such-model"]);
    assert_eq!(unknown.status.code(), Some(ExitCode::BadInput.code()), "{}", String::from_utf8_lossy(&unknown.stderr));
    assert!(unknown.stdout.is_empty());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("no-such-model"), "the reason reaches stderr before the exit");
}

/// The shipped binary end to end with a real model and debug logging on: `--transcribe-file
/// --json` must finish (the deadlock above hung it forever) and print one JSON object.
/// ```text
/// VOLTIP_LOCAL_MODEL_DIR=/path/to/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-int8-2024-07-17 \
/// VOLTIP_LOCAL_SAMPLE_WAV=/tmp/voltip-sample.wav \
///   cargo test -p voltip-desktop --test cli -- --ignored real_binary --nocapture
/// ```
#[test]
#[ignore = "needs VOLTIP_LOCAL_MODEL_DIR (a downloaded sherpa-onnx model) and VOLTIP_LOCAL_SAMPLE_WAV"]
fn real_binary_transcribe_file_with_debug_logs_finishes_and_prints_json() {
    let model_dir = PathBuf::from(std::env::var("VOLTIP_LOCAL_MODEL_DIR").expect("VOLTIP_LOCAL_MODEL_DIR"));
    let id = if model_dir.to_string_lossy().contains("paraformer") { "paraformer-zh" } else { "sense-voice-small" };
    let e = entry(id).unwrap();
    let data = tempfile::tempdir().unwrap();
    // `data_dir()` on Linux is `$XDG_DATA_HOME/voltip`; the model library is its `models/`.
    let dir = data.path().join("voltip").join("models").join(e.id);
    std::fs::create_dir_all(&dir).unwrap();
    for f in e.files() {
        let src = model_dir.join(f.name);
        std::fs::hard_link(&src, dir.join(f.name)).or_else(|_| std::fs::copy(&src, dir.join(f.name)).map(|_| ())).unwrap();
    }
    let manifest = Manifest {
        id: e.id.into(),
        version: CATALOGUE_VERSION,
        downloaded_at: 1,
        files: e.files().iter().map(|f| (f.name.to_owned(), f.sha256.to_owned())).collect(),
    };
    std::fs::write(dir.join(MANIFEST_FILE), serde_json::to_vec(&manifest).unwrap()).unwrap();
    let wav = PathBuf::from(std::env::var("VOLTIP_LOCAL_SAMPLE_WAV").unwrap_or_else(|_| "/tmp/voltip-sample.wav".into()));

    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_voltip-desktop"));
    command.arg("--transcribe-file").arg(&wav).args(["--model", e.id, "--json"]).env("RUST_LOG", "debug");
    command.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    isolated(&mut command, data.path());
    let child = command.spawn().unwrap();
    let pid = child.id();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    let Ok(output) = rx.recv_timeout(std::time::Duration::from_secs(180)) else {
        let _ = std::process::Command::new("kill").args(["-9", &pid.to_string()]).status();
        panic!("voltip --transcribe-file did not finish within 180 s (deadlock?)");
    };
    let output = output.unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1, "exactly one JSON line on stdout:\n{stdout}");
    let parsed: TranscribeOutput = serde_json::from_str(stdout.trim_end()).unwrap();
    println!("model={} latency_ms={} text={}", parsed.model, parsed.latency_ms, parsed.text);
    assert_eq!(parsed.model, e.id);
    assert!(parsed.text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)), "expected Chinese text: {}", parsed.text);
    assert!(stderr.contains("local transcription done"), "the recogniser's own log line reaches stderr:\n{stderr}");
}
