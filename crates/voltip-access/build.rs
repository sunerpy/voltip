//! The build key (docs/pairing.md 「设备票据」): CI derives `K_build(v)` for the version in the root
//! `package.json` and hands it in as `VOLTIP_BUILD_KEY` (hex) with `VOLTIP_BUILD_KEY_VERSION`. The
//! key is stored XOR-split with a pad derived from it, so its bytes never appear whole in the
//! binary (`scripts/lib/artefact-checks.sh` checks), together with a marker that says the build
//! is keyed. Without the variables the build has no key. `VOLTIP_REQUIRE_BUILD_KEY=1` (release
//! builds) refuses to build without one.

use std::fmt::Write as _;
use std::path::PathBuf;

use sha2::{Digest as _, Sha256};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    for var in ["VOLTIP_BUILD_KEY", "VOLTIP_BUILD_KEY_VERSION", "VOLTIP_REQUIRE_BUILD_KEY", "VOLTIP_EXPECT_PROOF"] {
        println!("cargo:rerun-if-env-changed={var}");
    }
    let package = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../package.json");
    println!("cargo:rerun-if-changed={}", package.display());
    let text = std::fs::read_to_string(&package).unwrap_or_else(|e| panic!("{}: {e}", package.display()));
    let value: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", package.display()));
    let Some(version) = value.get("version").and_then(serde_json::Value::as_str) else { panic!("{}: no version", package.display()) };
    println!("cargo:rustc-env=VOLTIP_APP_VERSION={version}");

    let key = std::env::var("VOLTIP_BUILD_KEY").ok().filter(|k| !k.trim().is_empty());
    let key_version = std::env::var("VOLTIP_BUILD_KEY_VERSION").ok().filter(|v| !v.trim().is_empty());
    let required = std::env::var("VOLTIP_REQUIRE_BUILD_KEY").is_ok_and(|v| v.trim() == "1");
    let keyed = match (key, key_version) {
        (Some(key), Some(key_version)) => {
            assert!(
                key_version.trim() == version,
                "VOLTIP_BUILD_KEY_VERSION is {key_version:?}, but package.json says {version:?}: derive the key for this version"
            );
            let bytes = hex::decode(key.trim()).unwrap_or_else(|_| panic!("VOLTIP_BUILD_KEY is not hex"));
            let bytes: [u8; 32] = bytes.try_into().unwrap_or_else(|b: Vec<u8>| panic!("VOLTIP_BUILD_KEY is {} bytes, not 32", b.len()));
            Some(bytes)
        }
        (None, None) => None,
        _ => panic!("set both VOLTIP_BUILD_KEY and VOLTIP_BUILD_KEY_VERSION, or neither"),
    };
    assert!(keyed.is_some() || !required, "VOLTIP_REQUIRE_BUILD_KEY=1 but no VOLTIP_BUILD_KEY: a release build must carry the build key");

    let mut out = String::new();
    match keyed {
        Some(key) => {
            let pad: [u8; 32] = Sha256::new().chain_update(b"voltip-access/pad/v1").chain_update(key).chain_update(version.as_bytes()).finalize().into();
            let masked: Vec<u8> = key.iter().zip(pad).map(|(k, p)| k ^ p).collect();
            let _ = writeln!(out, "static MASKED: [u8; 32] = {masked:?};");
            let _ = writeln!(out, "static PAD: [u8; 32] = {pad:?};");
            let _ = writeln!(out, "const KEYED: bool = true;");
            let _ = writeln!(out, "static MARKER: &str = \"voltip-access:keyed:{version}\";");
        }
        None => {
            let _ = writeln!(out, "static MASKED: [u8; 32] = [0; 32];");
            let _ = writeln!(out, "static PAD: [u8; 32] = [0; 32];");
            let _ = writeln!(out, "const KEYED: bool = false;");
            let _ = writeln!(out, "static MARKER: &str = \"\";");
        }
    }
    let dest = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_else(|_| panic!("OUT_DIR"))).join("build_key.rs");
    std::fs::write(&dest, out).unwrap_or_else(|e| panic!("{}: {e}", dest.display()));
}
