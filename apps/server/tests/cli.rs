//! `voltip-server` (docs/dictation.md §23.5): the command line, `--check`, the token, the service
//! end to end in-process with fake engines, and the real binary's exit paths (the version, a usage
//! error, `--check`, and on Unix SIGTERM), which leave through `exit_process`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use voltip_core::BuiltIn;
use voltip_core::dictation::fakes::{FakeRefiner, FakeTranscriber};
use voltip_core::dictation::{EngineFactory, Refiner, Transcriber};
use voltip_server::cli::{Action, Cli, ServeOptions};
use voltip_server::{APP_VERSION, Wiring, check, run};

const BUILT_IN: BuiltIn = BuiltIn { asr_url: Some("https://asr.test"), ..BuiltIn::EMPTY };
const LIMIT: Duration = Duration::from_secs(60);

fn parse(args: &[&str]) -> Result<Action, String> {
    let mut argv = vec!["voltip-server"];
    argv.extend_from_slice(args);
    Cli::parse_args(argv).map_err(|e| e.to_string())?.action()
}

fn options(args: &[&str]) -> ServeOptions {
    match parse(args).unwrap() {
        Action::Serve(options) | Action::Check(options) => options,
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_command_line_names_the_service_and_its_one_shot_actions() {
    let defaults = options(&[]);
    assert_eq!((defaults.listen.to_string().as_str(), defaults.max_minutes, defaults.concurrency, defaults.preload), ("127.0.0.1:47840", 120, 2, true));
    let set = options(&[
        "--scene",
        "coding",
        "--preset",
        "prompt",
        "--refine",
        "off",
        "--language",
        "zh",
        "--script",
        "traditional",
        "--asr",
        "aliyun",
        "--threads",
        "4",
        "--no-preload",
    ]);
    assert_eq!((set.choices.scene.as_deref(), set.choices.preset.as_deref(), set.choices.refine), (Some("coding"), Some("prompt"), Some(false)));
    assert_eq!((set.choices.language.as_deref(), set.choices.script.map(|s| s.as_str())), (Some("zh"), Some("traditional")));
    assert_eq!((set.overrides.asr.map(|p| p.as_str()), set.overrides.threads, set.preload), (Some("aliyun"), Some(4), false));
    assert!(matches!(parse(&["--check"]).unwrap(), Action::Check(_)));
    assert!(matches!(parse(&["--print-token"]).unwrap(), Action::PrintToken { token_file: None }));
    assert!(matches!(parse(&["--list-models"]).unwrap(), Action::ListModels));
    assert!(matches!(parse(&["--download-model", "qwen3-asr-0.6b"]).unwrap(), Action::DownloadModel { .. }));
    assert!(matches!(parse(&["--list-compute"]).unwrap(), Action::ListCompute));
    for bad in [
        &["--refine", "maybe"][..],
        &["--asr", "nope"],
        &["--script", "x"],
        &["--max-minutes", "121"],
        &["--concurrency", "9"],
        &["--scene", "a", "--app", "b"],
        &["--check", "--print-token"],
    ] {
        assert!(parse(bad).is_err(), "{bad:?}");
    }
    let remote = parse(&["--listen", "0.0.0.0:47840"]).unwrap_err();
    assert!(remote.contains("--allow-remote"), "{remote}");
    assert!(matches!(parse(&["--listen", "0.0.0.0:47840", "--allow-remote"]).unwrap(), Action::Serve(_)));
    assert!(matches!(parse(&["--listen", "[::1]:47840"]).unwrap(), Action::Serve(_)), "IPv6 loopback is this computer");
}

fn wiring(transcriber: FakeTranscriber, refiner: Option<FakeRefiner>) -> Wiring {
    let transcriber = Arc::new(transcriber);
    let refiner = refiner.map(Arc::new);
    let factory: EngineFactory = Arc::new(move |_| (transcriber.clone() as Arc<dyn Transcriber>, refiner.clone().map(|r| r as Arc<dyn Refiner>)));
    Wiring { factory, models: None, segmenter: None, secrets: voltip_core::UserSecrets::default(), secrets_notice: None, built_in: BUILT_IN }
}

#[test]
fn check_reports_the_configuration_and_whether_recognition_can_run() {
    let dir = tempfile::tempdir().unwrap();
    let mut out = Vec::new();
    let code = check(&options(&["--scene", "coding", "--preset", "prompt", "--language", "zh"]), dir.path(), wiring(FakeTranscriber::ok("x"), None), &mut out);
    let text = String::from_utf8(out).unwrap();
    assert_eq!(code, 0, "{text}");
    for line in [
        "voltip-server",
        "地址：http://127.0.0.1:47840/v1（仅本机）",
        "语音识别：内置服务",
        "默认处理方式：场景 编程开发；预设 提示词优化",
        "语言：zh",
        "状态：可用",
    ] {
        assert!(text.contains(line), "{line}: {text}");
    }
    assert!(!text.contains("https://asr.test"), "no host is printed");
    assert!(!dir.path().join("serve").exists() && !dir.path().join("scenes.json").exists(), "a check writes nothing");
    let mut out = Vec::new();
    assert_eq!(check(&options(&["--scene", "nope"]), dir.path(), wiring(FakeTranscriber::ok("x"), None), &mut out), 1);
    assert!(String::from_utf8(out).unwrap().contains("启动参数无效：没有名为「nope」的场景"));
    let mut out = Vec::new();
    let not_ready = check(&options(&["--asr", "aliyun"]), dir.path(), wiring(FakeTranscriber::ok("x"), None), &mut out);
    let text = String::from_utf8(out).unwrap();
    assert_eq!(not_ready, 1);
    assert!(text.contains("状态：不可用：识别服务未配置"), "{text}");
}

#[test]
fn print_token_creates_the_token_once_and_prints_it_again() {
    let dir = tempfile::tempdir().unwrap();
    let action = parse(&["--print-token"]).unwrap();
    let (mut first, mut err) = (Vec::new(), Vec::new());
    assert_eq!(run(&action, dir.path(), &mut first, &mut err), 0);
    let (mut second, mut err) = (Vec::new(), Vec::new());
    assert_eq!(run(&action, dir.path(), &mut second, &mut err), 0);
    assert_eq!(first, second);
    assert_eq!(String::from_utf8(first).unwrap().trim().len(), 43, "32 bytes as base64url");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(std::fs::metadata(dir.path().join("serve/token")).unwrap().permissions().mode() & 0o777, 0o600);
    }
}

fn wav(seconds: f64) -> Vec<u8> {
    let spec = hound::WavSpec { channels: 1, sample_rate: 24_000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut w = hound::WavWriter::new(&mut buf, spec).unwrap();
        for i in 0..(24_000.0 * seconds) as usize {
            w.write_sample((8000.0 * (i as f64 * 0.07).sin()) as i16).unwrap();
        }
        w.finalize().unwrap();
    }
    buf.into_inner()
}

/// Serve on a free port in a thread; returns its address, the token, the stop switch and the thread.
fn start(
    dir: &Path,
    wiring: Wiring,
    args: &[&str],
    grace: Duration,
) -> (std::net::SocketAddr, String, tokio::sync::oneshot::Sender<()>, std::thread::JoinHandle<i32>) {
    let mut argv = vec!["--listen", "127.0.0.1:0", "--no-preload"];
    argv.extend_from_slice(args);
    let options = options(&argv);
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let (tx, rx) = std::sync::mpsc::channel();
    let dir_owned = dir.to_path_buf();
    let thread = std::thread::spawn(move || {
        voltip_server::serve_with_grace(
            &options,
            &dir_owned,
            wiring,
            move || async move {
                let _ = stopped.await;
            },
            Some(tx),
            grace,
        )
    });
    let addr = rx.recv_timeout(LIMIT).expect("the server listens");
    let token = std::fs::read_to_string(dir.join("serve/token")).unwrap().trim().to_owned();
    (addr, token, stop, thread)
}

fn form(model: &str) -> reqwest::multipart::Form {
    reqwest::multipart::Form::new()
        .part("file", reqwest::multipart::Part::bytes(wav(2.0)).file_name("audio.wav").mime_str("audio/wav").unwrap())
        .text("model", model.to_owned())
        .text("language", "zh")
        .text("prompt", "Transcribe only what the speaker says.")
        .text("response_format", "verbose_json")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_service_answers_with_the_pipelines_text_and_stops_on_request() {
    let dir = tempfile::tempdir().unwrap();
    let (addr, token, stop, thread) =
        start(dir.path(), wiring(FakeTranscriber::ok("你好世界"), Some(FakeRefiner::ok("你好，世界。"))), &["--scene", "coding"], Duration::from_secs(5));
    let response =
        reqwest::Client::new().post(format!("http://{addr}/v1/audio/transcriptions")).bearer_auth(&token).multipart(form("voltip")).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["text"], "你好，世界。");
    assert_eq!((body["voltip"]["raw_text"].as_str(), body["voltip"]["scene"]["builtin"].as_str()), (Some("你好世界"), Some("coding")));
    assert_eq!(body["language"], "zh", "the request's language reached the take");
    stop.send(()).unwrap();
    assert_eq!(thread.join().unwrap(), 0);
    assert_eq!(std::fs::read_dir(dir.path().join("serve/uploads")).unwrap().count(), 0);
    assert!(reqwest::get(format!("http://{addr}/healthz")).await.is_err(), "no longer listening");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_request_still_running_after_the_grace_period_is_abandoned() {
    let dir = tempfile::tempdir().unwrap();
    let (addr, token, stop, thread) =
        start(dir.path(), wiring(FakeTranscriber::slow("迟到的文字", Duration::from_secs(300)), None), &[], Duration::from_secs(1));
    let request = tokio::spawn(async move {
        reqwest::Client::new().post(format!("http://{addr}/v1/audio/transcriptions")).bearer_auth(&token).multipart(form("voltip")).send().await
    });
    let started = Instant::now();
    while std::fs::read_dir(dir.path().join("serve/uploads")).unwrap().count() == 0 {
        assert!(started.elapsed() < LIMIT, "the upload never arrived");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    stop.send(()).unwrap();
    let joined = tokio::task::spawn_blocking(move || thread.join().unwrap());
    assert_eq!(tokio::time::timeout(LIMIT, joined).await.unwrap().unwrap(), 0, "the server stops although the request never finished");
    assert!(started.elapsed() < Duration::from_secs(30), "it waited the grace period, not the request");
    assert_eq!(std::fs::read_dir(dir.path().join("serve/uploads")).unwrap().count(), 0, "the abandoned upload is removed");
    let _ = request.await;
}

// ---- the real binary ------------------------------------------------------------------------

fn binary() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_voltip-server"));
    command.env("VOLTIP_DEV_SECRET_STORE", "memory").env("RUST_LOG", "info");
    command
}

#[test]
fn the_binary_reports_its_version_usage_errors_and_its_check() {
    let version = binary().arg("--version").output().unwrap();
    assert!(version.status.success());
    assert_eq!(String::from_utf8(version.stdout).unwrap().trim(), format!("voltip-server {APP_VERSION}"));
    let package: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../package.json")).unwrap()).unwrap();
    assert_eq!(package["version"].as_str(), Some(APP_VERSION), "the version is package.json's");
    assert_eq!(binary().args(["--refine", "maybe"]).output().unwrap().status.code(), Some(2));
    assert_eq!(binary().args(["--listen", "0.0.0.0:1"]).output().unwrap().status.code(), Some(2));
    let dir = tempfile::tempdir().unwrap();
    let checked = binary().env("VOLTIP_DEV_DATA_DIR", dir.path()).args(["--check", "--local-model", "qwen3-asr-0.6b"]).output().unwrap();
    let stdout = String::from_utf8(checked.stdout).unwrap();
    assert_eq!(checked.status.code(), Some(1), "{stdout}");
    assert!(stdout.contains("本地模型未下载") && stdout.contains("--download-model qwen3-asr-0.6b"), "{stdout}");
    assert!(stdout.contains("（qwen3-asr-0.6b，未下载）"), "the id --local-model takes: {stdout}");
}

// Linux only: there the system's store is files (rustls-native-certs, which `SSL_CERT_FILE` and
// `SSL_CERT_DIR` redirect); Windows and macOS keep theirs in the operating system.
#[cfg(target_os = "linux")]
#[test]
fn regression_without_ca_certificates_cloud_recognition_is_unavailable_and_says_why() {
    use voltip_core::{ProviderId, ProviderSettings, Settings, SettingsStore};

    // In an ubuntu:24.04 container without ca-certificates every HTTPS client failed to build:
    // `--check` still said 可用, the service started, and each request failed with "builder error".
    let dir = tempfile::tempdir().unwrap();
    let mut settings = Settings { relay_enabled: false, ..Settings::default() };
    settings.engines.asr_provider = ProviderId::Custom;
    settings.engines.providers.insert(
        ProviderId::Custom,
        ProviderSettings { asr_url: Some("https://127.0.0.1:9/v1".into()), asr_model: Some("whisper-1".into()), ..ProviderSettings::default() },
    );
    SettingsStore::new(dir.path()).save(&settings).unwrap();
    // rustls-native-certs reads these first: an empty file and an empty directory are a system
    // without certificates.
    let certs = tempfile::tempdir().unwrap();
    std::fs::write(certs.path().join("none.pem"), "").unwrap();
    let without = |args: &[&str]| {
        binary()
            .env("VOLTIP_DEV_DATA_DIR", dir.path())
            .env("SSL_CERT_FILE", certs.path().join("none.pem"))
            .env("SSL_CERT_DIR", certs.path())
            .args(args)
            .output()
            .unwrap()
    };
    let checked = without(&["--check"]);
    let stdout = String::from_utf8(checked.stdout).unwrap();
    assert_eq!(checked.status.code(), Some(1), "{stdout}");
    assert!(stdout.contains("状态：不可用：无法建立 HTTPS 连接") && stdout.contains("ca-certificates"), "{stdout}");
    let served = without(&["--listen", "127.0.0.1:0", "--no-preload"]);
    assert_eq!(served.status.code(), Some(1), "the service does not start");
    assert!(String::from_utf8(served.stderr).unwrap().contains("语音识别不可用：无法建立 HTTPS 连接"));
    // With the system's certificates the same configuration is ready.
    let ready = binary().env("VOLTIP_DEV_DATA_DIR", dir.path()).arg("--check").output().unwrap();
    assert_eq!(ready.status.code(), Some(0), "{}", String::from_utf8_lossy(&ready.stdout));
}

#[cfg(unix)]
#[test]
fn the_binary_stops_cleanly_on_sigterm() {
    use std::io::{BufRead as _, BufReader};
    use std::process::Stdio;

    let dir = tempfile::tempdir().unwrap();
    let mut child = binary()
        .env("VOLTIP_DEV_DATA_DIR", dir.path())
        .args(["--listen", "127.0.0.1:0", "--local-model", "qwen3-asr-0.6b", "--no-preload"])
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let stderr = child.stderr.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    let started = Instant::now();
    loop {
        let line = rx.recv_timeout(LIMIT).expect("the server says where it listens");
        if line.contains("listening on http://") {
            break;
        }
        assert!(started.elapsed() < LIMIT);
    }
    let status = Command::new("kill").args(["-TERM", &child.id().to_string()]).status().unwrap();
    assert!(status.success());
    let started = Instant::now();
    let exit = loop {
        if let Some(exit) = child.try_wait().unwrap() {
            break exit;
        }
        assert!(started.elapsed() < LIMIT, "the server did not stop");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(exit.code(), Some(0));
    assert!(dir.path().join("serve/uploads").is_dir());
    assert_eq!(std::fs::read_dir(dir.path().join("serve/uploads")).unwrap().count(), 0);
}
