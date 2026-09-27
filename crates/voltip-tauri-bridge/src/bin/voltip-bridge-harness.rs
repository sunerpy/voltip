//! `voltip-bridge-harness`: the Tauri command surface of the shells over stdio JSON lines, so a
//! TypeScript test can drive the real Rust core without a webview.
//!
//! ```text
//! stdin : {"id":1,"cmd":"core_state"}
//!         {"id":2,"cmd":"pairing_join_code","args":{"code":"483 921"}}
//! stdout: {"id":1,"ok":{...UiState...}}      {"id":2,"ok":null}      {"id":3,"err":"..."}
//!         {"event":{"type":"pairing",...}}    (every UiEvent, as the webview bus would carry it)
//! ```
//!
//! Exits on stdin EOF. The secret store is in-memory; settings are written from the flags.

#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::io::{BufRead as _, Write as _};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use voltip_core::ui::UiEvent;
use voltip_core::{CoreConfig, Settings, SettingsStore};
use voltip_identity::MemorySecretStore;
use voltip_tauri_bridge::{Bridge, UiCommand};

const STATE_COMMAND: &str = "core_state";
const USAGE: &str = "usage: voltip-bridge-harness --data-dir <dir> [--relay-url <ws url>] [--device-name <name>] [--direct-bind <ip:port>]";
const REQUEST_QUEUE: usize = 64;
const EXIT_USAGE: u8 = 2;
const EXIT_START: u8 = 1;

#[derive(Debug, PartialEq, Eq)]
struct Options {
    data_dir: PathBuf,
    relay_url: Option<String>,
    device_name: Option<String>,
    direct_bind: Option<std::net::SocketAddr>,
}

fn parse_options(args: impl IntoIterator<Item = String>) -> Result<Options, String> {
    let (mut data_dir, mut relay_url, mut device_name, mut direct_bind) = (None, None, None, None);
    let mut args = args.into_iter();
    while let Some(flag) = args.next() {
        let value = args.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--data-dir" => data_dir = Some(PathBuf::from(value)),
            "--relay-url" => relay_url = Some(value),
            "--device-name" => device_name = Some(value),
            "--direct-bind" => direct_bind = Some(value.parse().map_err(|e| format!("--direct-bind: {e}"))?),
            other => return Err(format!("unknown flag {other}")),
        }
    }
    Ok(Options { data_dir: data_dir.ok_or("--data-dir is required")?, relay_url, device_name, direct_bind })
}

#[derive(Debug, Deserialize)]
struct Request {
    id: u64,
    cmd: String,
    #[serde(default)]
    args: Option<Map<String, Value>>,
}

#[derive(Debug, Serialize, PartialEq)]
struct Response {
    id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    ok: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    err: Option<String>,
}

impl Response {
    fn ok(id: u64, value: Value) -> Self {
        Self { id, ok: Some(value), err: None }
    }
    fn err(id: u64, message: impl Into<String>) -> Self {
        Self { id, ok: None, err: Some(message.into()) }
    }
}

#[derive(Serialize)]
struct EventLine<'a> {
    event: &'a UiEvent,
}

/// `invoke(name, args)` → the tagged form `UiCommand` deserializes, exactly like a `#[tauri::command]`
/// binding its parameters.
fn to_command(cmd: &str, args: Option<Map<String, Value>>) -> Result<UiCommand, String> {
    let mut tagged = args.unwrap_or_default();
    tagged.insert("command".into(), Value::String(cmd.to_owned()));
    serde_json::from_value(Value::Object(tagged)).map_err(|e| format!("invalid args for {cmd}: {e}"))
}

fn execute(bridge: &Bridge, req: Request) -> Response {
    if req.cmd == STATE_COMMAND {
        return match serde_json::to_value(bridge.state()) {
            Ok(state) => Response::ok(req.id, state),
            Err(e) => Response::err(req.id, e.to_string()),
        };
    }
    match to_command(&req.cmd, req.args).and_then(|c| bridge.dispatch(c).map_err(String::from)) {
        Ok(()) => Response::ok(req.id, Value::Null),
        Err(e) => Response::err(req.id, e),
    }
}

/// One complete JSON line per call: serialize first so a value serde rejects never leaves a partial
/// object on the stream.
fn write_line<T: Serialize>(out: &Mutex<std::io::Stdout>, value: &T) {
    let mut line = match serde_json::to_string(value) {
        Ok(text) => text,
        Err(e) => {
            tracing::warn!(error = %e, "value does not serialize; line dropped");
            return;
        }
    };
    line.push('\n');
    let mut out = out.lock();
    if let Err(e) = out.write_all(line.as_bytes()).and_then(|()| out.flush()) {
        tracing::warn!(error = %e, "stdout write failed");
    }
}

fn start(opts: Options) -> Result<Bridge, String> {
    let settings = Settings { relay_url: opts.relay_url.clone(), relay_enabled: opts.relay_url.is_some(), ..Settings::default() };
    SettingsStore::new(&opts.data_dir).save(&settings).map_err(|e| e.to_string())?;
    let mut config = CoreConfig::new(opts.data_dir);
    if let Some(name) = opts.device_name {
        config.default_device_name = name;
    }
    if let Some(bind) = opts.direct_bind {
        config.direct_bind = bind;
    }
    Bridge::start(config, Arc::new(MemorySecretStore::new())).map_err(|e| e.to_string())
}

#[tokio::main]
async fn main() -> ExitCode {
    let opts = match parse_options(std::env::args().skip(1)) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}\n{USAGE}");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).try_init();
    let bridge = match start(opts) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("harness: {e}");
            return ExitCode::from(EXIT_START);
        }
    };
    let out = Arc::new(Mutex::new(std::io::stdout()));

    let mut events = bridge.events();
    let event_out = out.clone();
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(ev) => write_line(&event_out, &EventLine { event: &ev }),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => tracing::warn!(skipped = n, "reader lagged"),
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // Blocking stdin on its own thread; the async side only sees complete lines.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(REQUEST_QUEUE);
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if tx.blocking_send(line).is_err() {
                break;
            }
        }
    });
    while let Some(line) = rx.recv().await {
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(req) => execute(&bridge, req),
            Err(e) => Response::err(0, format!("bad request: {e}")),
        };
        write_line(&out, &response);
    }
    bridge.shutdown();
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn options_parse_every_flag_and_reject_bad_input() {
        let o = parse_options(args(&["--data-dir", "/tmp/x", "--relay-url", "ws://127.0.0.1:1/ws", "--device-name", "Desk", "--direct-bind", "127.0.0.1:0"]))
            .unwrap();
        assert_eq!(o.data_dir, PathBuf::from("/tmp/x"));
        assert_eq!(o.relay_url.as_deref(), Some("ws://127.0.0.1:1/ws"));
        assert_eq!(o.device_name.as_deref(), Some("Desk"));
        assert_eq!(o.direct_bind, Some("127.0.0.1:0".parse().unwrap()));
        let o = parse_options(args(&["--data-dir", "d"])).unwrap();
        assert_eq!(o, Options { data_dir: "d".into(), relay_url: None, device_name: None, direct_bind: None });
        assert!(parse_options(args(&[])).unwrap_err().contains("--data-dir"));
        assert!(parse_options(args(&["--data-dir"])).unwrap_err().contains("needs a value"));
        assert!(parse_options(args(&["--data-dir", "d", "--nope", "1"])).unwrap_err().contains("unknown flag"));
        assert!(parse_options(args(&["--data-dir", "d", "--direct-bind", "nope"])).unwrap_err().contains("--direct-bind"));
    }

    #[test]
    fn requests_parse_and_map_onto_ui_commands() {
        let req: Request = serde_json::from_str(r#"{"id":7,"cmd":"pairing_join_code","args":{"code":"483 921"}}"#).unwrap();
        assert_eq!(req.id, 7);
        assert!(matches!(to_command(&req.cmd, req.args).unwrap(), UiCommand::PairingJoinCode { code } if code == "483 921"));
        let req: Request = serde_json::from_str(r#"{"id":8,"cmd":"pairing_start"}"#).unwrap();
        assert!(req.args.is_none());
        assert!(matches!(to_command(&req.cmd, req.args).unwrap(), UiCommand::PairingStart));
        let err = to_command("send_text", Some(Map::new())).unwrap_err();
        assert!(err.contains("invalid args for send_text"), "{err}");
        assert!(to_command("no_such", None).unwrap_err().contains("no_such"));
        assert!(serde_json::from_str::<Request>(r#"{"cmd":"x"}"#).is_err(), "id is required");
    }

    #[test]
    fn responses_serialize_in_the_documented_shape() {
        assert_eq!(serde_json::to_string(&Response::ok(1, Value::Null)).unwrap(), r#"{"id":1,"ok":null}"#);
        assert_eq!(serde_json::to_string(&Response::ok(2, serde_json::json!({"a": 1}))).unwrap(), r#"{"id":2,"ok":{"a":1}}"#);
        assert_eq!(serde_json::to_string(&Response::err(3, "boom")).unwrap(), r#"{"id":3,"err":"boom"}"#);
        let ev = UiEvent::Error { message: "x".into() };
        assert_eq!(serde_json::to_string(&EventLine { event: &ev }).unwrap(), r#"{"event":{"type":"error","message":"x"}}"#);
    }

    #[tokio::test]
    async fn execute_answers_state_and_dispatch_results() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = start(Options {
            data_dir: dir.path().to_path_buf(),
            relay_url: None,
            device_name: Some("Harness".into()),
            direct_bind: Some("127.0.0.1:0".parse().unwrap()),
        })
        .unwrap();
        let saved = SettingsStore::new(dir.path()).load().unwrap();
        assert!(!saved.relay_enabled, "no relay flag → relay disabled");
        let mut events = bridge.events();
        // Wait for the core to be up before asserting on state.
        loop {
            let ev = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv()).await.unwrap().unwrap();
            if matches!(ev, UiEvent::State(_)) {
                break;
            }
        }
        let resp = execute(&bridge, Request { id: 1, cmd: STATE_COMMAND.into(), args: None });
        assert_eq!(resp.ok.as_ref().unwrap()["identity"]["name"], "Harness");
        let resp = execute(&bridge, Request { id: 2, cmd: "device_rename".into(), args: serde_json::from_str(r#"{"name":"Renamed"}"#).unwrap() });
        assert_eq!(resp, Response::ok(2, Value::Null));
        let resp = execute(&bridge, Request { id: 3, cmd: "device_forget".into(), args: serde_json::from_str(r#"{"publicKey":"zz"}"#).unwrap() });
        assert!(resp.err.unwrap().contains("64 hex"));
        let resp = execute(&bridge, Request { id: 4, cmd: "bogus".into(), args: None });
        assert!(resp.err.unwrap().contains("bogus"));
        bridge.shutdown();
    }
}
