//! The version (`VOLTIP_APP_VERSION`, from the root `package.json` that release-please bumps; the
//! Cargo version stays `0.0.0`) and where the binary finds the sherpa-onnx runtime: next to it for
//! `cargo run`, in `../lib` inside the package (`scripts/build-server-linux-x64.sh`).

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let package = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../package.json");
    println!("cargo:rerun-if-changed={}", package.display());
    let text = std::fs::read_to_string(&package).unwrap_or_else(|e| panic!("{}: {e}", package.display()));
    let value: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", package.display()));
    let Some(version) = value.get("version").and_then(serde_json::Value::as_str) else { panic!("{}: no version", package.display()) };
    println!("cargo:rustc-env=VOLTIP_APP_VERSION={version}");
    match std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default().as_str() {
        "linux" => println!("cargo:rustc-link-arg-bins=-Wl,-rpath,$ORIGIN:$ORIGIN/../lib"),
        "macos" => println!("cargo:rustc-link-arg-bins=-Wl,-rpath,@loader_path"),
        _ => {}
    }
}
