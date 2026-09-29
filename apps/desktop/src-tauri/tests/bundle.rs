//! Packaging and native-build configuration (docs/dictation.md §15.5). The merged
//! `tauri.conf.json` + `tauri.<platform>.conf.json` of every desktop target goes through Tauri's
//! own `Config` schema (`deny_unknown_fields`, the same parse `tauri-build` and the CLI run), and
//! the macOS / Windows bundle settings B6 needs are asserted field by field. A Linux host cannot
//! bundle a `.app` or an NSIS installer; this is what it can prove about their configuration, plus
//! the build prerequisites a native Windows build tripped over (the `regression_*` tests).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use tauri_utils::config::{Config, NSISInstallerMode, WebviewInstallMode};
use tauri_utils::platform::Target;

fn tauri_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn merged(target: Target) -> Config {
    let (value, paths) = tauri_utils::config::parse::read_from(target, &tauri_dir()).unwrap_or_else(|e| panic!("{target:?}: {e}"));
    assert!(paths.iter().any(|p| p.ends_with("tauri.conf.json")), "{paths:?}");
    serde_json::from_value(value).unwrap_or_else(|e| panic!("{target:?} config rejected by the Tauri schema: {e}"))
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The value right after `<key>{key}</key>` in an XML plist: the `<string>` text, or `true` /
/// `false` for the boolean elements.
fn plist_value(xml: &str, key: &str) -> Option<String> {
    let rest = &xml[xml.find(&format!("<key>{key}</key>"))? + key.len() + 11..];
    let rest = rest.trim_start();
    if let Some(body) = rest.strip_prefix("<string>") {
        return body.split_once("</string>").map(|(v, _)| v.to_owned());
    }
    ["true", "false"].into_iter().find(|b| rest.starts_with(&format!("<{b}/>"))).map(str::to_owned)
}

#[test]
fn every_desktop_target_config_parses_with_the_tauri_schema() {
    for target in [Target::MacOS, Target::Windows, Target::Linux] {
        let config = merged(target);
        assert_eq!(config.identifier, "dev.voltip.desktop", "{target:?}");
        assert_eq!(config.product_name.as_deref(), Some("Voltip"), "{target:?}");
    }
}

#[test]
fn macos_bundle_carries_the_dylibs_entitlements_minimum_version_and_microphone_string() {
    let config = merged(Target::MacOS);
    let mac = &config.bundle.macos;
    assert_eq!(mac.minimum_system_version.as_deref(), Some("11.0"), "the sherpa-onnx dylibs are built for minos 11.0");
    // No Developer ID yet (docs/roadmap.md): the bundle is signed ad hoc (`-`), which Apple
    // silicon requires of anything downloaded and which gives Gatekeeper a sealed bundle to offer
    // "Open Anyway" for instead of calling it damaged. The hardened runtime stays off with it: it
    // buys nothing without notarization, and its library validation refuses ad-hoc dylibs.
    assert_eq!(mac.signing_identity.as_deref(), Some("-"));
    assert!(!mac.hardened_runtime);
    // Exactly the two dylibs build.rs stages (MACOS_RUNTIME_DYLIBS), from the staging directory.
    let frameworks = mac.frameworks.clone().unwrap_or_default();
    assert_eq!(frameworks, ["./resources/macos/libsherpa-onnx-c-api.dylib", "./resources/macos/libonnxruntime.dylib"]);
    let build_rs = read(&tauri_dir().join("build.rs"));
    assert!(build_rs.contains(r#"const MACOS_RUNTIME_DYLIBS: [&str; 2] = ["libsherpa-onnx-c-api.dylib", "libonnxruntime.dylib"];"#));
    assert!(build_rs.contains(r#""macos" => stage_runtime_libs("macos", &MACOS_RUNTIME_DYLIBS)"#));

    // A hardened runtime (with a Developer ID) needs the audio-input entitlement to open the
    // microphone; it is kept for that day and is harmless without one. Nothing JITs in
    // this process (ggml / ONNX Runtime CPU kernels are precompiled, WebKit JITs in its own
    // WebContent process) and the bundler signs the dylibs with the app's identity, so none of the
    // weakening exceptions is present, and the app is not sandboxed.
    let entitlements = mac.entitlements.as_deref().expect("entitlements path");
    let entitlements = read(&tauri_dir().join(entitlements));
    assert_eq!(plist_value(&entitlements, "com.apple.security.device.audio-input").as_deref(), Some("true"));
    for absent in [
        "com.apple.security.cs.allow-unsigned-executable-memory",
        "com.apple.security.cs.allow-jit",
        "com.apple.security.cs.disable-library-validation",
        "com.apple.security.app-sandbox",
    ] {
        assert!(!entitlements.contains(absent), "{absent} must not be granted");
    }

    // `src-tauri/Info.plist` is merged by the bundler and embedded by `generate_context!` in dev
    // builds, so `cargo run` / `tauri dev` also carry the usage string TCC requires.
    let info = read(&tauri_dir().join("Info.plist"));
    let usage = plist_value(&info, "NSMicrophoneUsageDescription").expect("NSMicrophoneUsageDescription");
    assert!(usage.len() > 20, "{usage}");
    assert!(mac.info_plist.is_none(), "the Info.plist next to tauri.conf.json is picked up without a path");
    // Localised prompt strings: English and Simplified Chinese, copied into Contents/Resources/.
    let mut localised: Vec<_> = mac.files.iter().collect();
    localised.sort();
    assert_eq!(localised.len(), 2, "{localised:?}");
    for (dest, src) in localised {
        assert!(dest.starts_with("Resources") && dest.ends_with("InfoPlist.strings"), "{dest:?}");
        let strings = read(&tauri_dir().join(src));
        assert!(strings.contains(r#""NSMicrophoneUsageDescription" = ""#), "{src:?}");
    }
}

/// docs/dictation.md §22: recording the computer's sound on macOS goes through a Core Audio process
/// tap, which TCC allows only with a usage string; like the microphone's it is in `Info.plist` and
/// localised in both prompt languages.
#[test]
fn macos_bundle_asks_before_recording_the_computers_sound() {
    let mac = merged(Target::MacOS).bundle.macos;
    let info = read(&tauri_dir().join("Info.plist"));
    let usage = plist_value(&info, "NSAudioCaptureUsageDescription").expect("NSAudioCaptureUsageDescription");
    assert!(usage.contains("computer"), "{usage}");
    for src in mac.files.values() {
        let strings = read(&tauri_dir().join(src));
        assert!(strings.contains(r#""NSAudioCaptureUsageDescription" = ""#), "{src:?}");
    }
}

#[test]
fn windows_bundle_installs_per_user_refuses_downgrades_and_names_the_webview_mode() {
    let config = merged(Target::Windows);
    let win = &config.bundle.windows;
    assert!(!win.allow_downgrades);
    // Embedded, not downloaded (2026-09-26): the NSIS download step (`NSISdl`) of an installer
    // cross-built on Linux cannot fetch the https bootstrapper URL, so on a machine without WebView2
    // (Windows Server 2022 in CI) the silent install aborted with exit code 2 before copying a file.
    assert_eq!(win.webview_install_mode, WebviewInstallMode::EmbedBootstrapper { silent: true });
    // tauri.conf.json says `both`; the Windows file narrows it to the per-user install the updater
    // can replace without an elevation prompt.
    assert_eq!(win.nsis.as_ref().map(|n| n.install_mode), Some(NSISInstallerMode::CurrentUser));
    let resources = serde_json::to_value(&config.bundle.resources).unwrap();
    for dll in ["sherpa-onnx-c-api.dll", "onnxruntime.dll", "onnxruntime_providers_shared.dll"] {
        assert_eq!(resources[format!("resources/windows/{dll}")], dll, "{resources}");
    }
}

/// Regression (2026-09-27, native Windows gate): `tauri-build` checks every listed resource on every
/// build, so listing the Vulkan loader in tauri.windows.conf.json broke a plain native `cargo build`
/// with nothing staged. The Windows file lists only what build.rs stages itself; the package script
/// and the release re-bundle merge tauri.package-windows.conf.json on top for the loader and the
/// third-party notices.
#[test]
fn regression_a_plain_windows_build_needs_no_vulkan_file_the_package_overlay_adds_it() {
    let plain = serde_json::to_value(&merged(Target::Windows).bundle.resources).unwrap();
    let listed: Vec<&str> = plain.as_object().unwrap().keys().map(String::as_str).collect();
    assert!(listed.iter().all(|k| !k.contains("vulkan")), "{listed:?}");
    let (mut value, _) = tauri_utils::config::parse::read_from(Target::Windows, &tauri_dir()).unwrap();
    let overlay: serde_json::Value = serde_json::from_str(&read(&tauri_dir().join("tauri.package-windows.conf.json"))).unwrap();
    json_patch::merge(&mut value, &overlay);
    let packaged: Config = serde_json::from_value(value).unwrap();
    let resources = serde_json::to_value(&packaged.bundle.resources).unwrap();
    for (file, name) in [("vulkan-1.dll", "vulkan-1.dll"), ("vulkan-1-LICENSE.txt", "vulkan-1-LICENSE.txt"), ("sherpa-onnx-c-api.dll", "sherpa-onnx-c-api.dll")]
    {
        assert_eq!(resources[format!("resources/windows/{file}")], name, "{resources}");
    }
    // The third-party licence texts ship with every package (scripts/release/third-party-notices.py).
    assert_eq!(resources["resources/THIRD-PARTY-NOTICES.txt"], "THIRD-PARTY-NOTICES.txt", "{resources}");
    let linux = merged(Target::Linux);
    for files in [&linux.bundle.linux.deb.files, &linux.bundle.linux.rpm.files, &linux.bundle.linux.appimage.files] {
        assert_eq!(files.get(Path::new("/usr/share/doc/voltip/THIRD-PARTY-NOTICES.txt")), Some(&PathBuf::from("resources/THIRD-PARTY-NOTICES.txt")));
    }
    let script = read(&tauri_dir().join("../../../scripts/build-windows-x64.sh"));
    assert!(script.contains("--config src-tauri/tauri.package-windows.conf.json"), "the package script merges the overlay");
    // The release packages are built once, by the candidate workflow (release.yml only promotes).
    let release = read(&tauri_dir().join("../../../.github/workflows/release-candidate.yml"));
    assert!(release.contains("--config src-tauri/tauri.package-windows.conf.json"), "the release re-bundle merges it too");
}

/// The macOS release leg (2026-09-28) merges tauri.package-macos.conf.json the same way, so the
/// .app carries the third-party licence texts while a plain `cargo tauri build` needs no notices
/// file.
#[test]
fn the_macos_release_bundle_carries_the_notices_through_its_package_overlay() {
    let plain = serde_json::to_value(&merged(Target::MacOS).bundle.resources).unwrap();
    assert!(plain.get("resources/THIRD-PARTY-NOTICES.txt").is_none(), "{plain}");
    let (mut value, _) = tauri_utils::config::parse::read_from(Target::MacOS, &tauri_dir()).unwrap();
    let overlay: serde_json::Value = serde_json::from_str(&read(&tauri_dir().join("tauri.package-macos.conf.json"))).unwrap();
    json_patch::merge(&mut value, &overlay);
    let packaged: Config = serde_json::from_value(value).unwrap();
    let resources = serde_json::to_value(&packaged.bundle.resources).unwrap();
    assert_eq!(resources["resources/THIRD-PARTY-NOTICES.txt"], "THIRD-PARTY-NOTICES.txt", "{resources}");
    assert_eq!(packaged.bundle.macos.signing_identity.as_deref(), Some("-"), "the overlay keeps the ad-hoc signature");
    let release = read(&tauri_dir().join("../../../.github/workflows/release-candidate.yml"));
    assert!(release.contains("--config src-tauri/tauri.package-macos.conf.json"), "the release macOS leg merges it");
}

#[test]
fn the_macos_and_windows_bundle_settings_stay_in_their_own_platform_files() {
    let linux = merged(Target::Linux);
    assert!(linux.bundle.macos.frameworks.is_none());
    assert!(linux.bundle.macos.entitlements.is_none());
    assert!(linux.bundle.windows.allow_downgrades, "Tauri's default outside the Windows file");
    let windows = merged(Target::Windows);
    assert!(windows.bundle.macos.frameworks.is_none());
    let macos = merged(Target::MacOS);
    assert_eq!(macos.bundle.windows.nsis.as_ref().map(|n| n.install_mode), Some(NSISInstallerMode::Both));
}

/// A fresh build has no sherpa-onnx runtime in the profile directory until sherpa-onnx-sys's build
/// script downloads and copies it, and build.rs stages it from there. Cargo runs one build script
/// before another only for an immediate dependency with a `links` key, so the desktop shell names
/// the `-sys` crate itself. Without it (2026-09-26, native Windows, slow GitHub download) build.rs
/// ran first and `tauri_build` failed on the missing `resources/windows/*.dll`.
///
/// `--no-deps`: the workspace's own declarations only, nothing resolved or downloaded (a full
/// `cargo metadata` needs every platform's crates, which a Windows checkout has not fetched).
#[test]
fn regression_the_sherpa_onnx_build_script_runs_before_build_rs_stages_its_runtime() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let out = std::process::Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--no-deps", "--offline", "--manifest-path"])
        .arg(tauri_dir().join("Cargo.toml"))
        .output()
        .expect("cargo metadata");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let meta: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let packages = meta["packages"].as_array().unwrap();
    let dependency = |package: &str, name: &str| {
        let package = packages.iter().find(|p| p["name"] == package).unwrap_or_else(|| panic!("{package} not in the workspace"));
        package["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["name"] == name && d["kind"].is_null() && d["target"].is_null())
            .cloned()
            .unwrap_or_else(|| panic!("{} does not name {name} as a normal dependency on every target", package["name"]))
    };
    let direct = dependency("voltip-desktop", "sherpa-onnx-sys");
    // The selection voltip-asr-local makes through `sherpa-onnx` (the default `static` archives
    // link espeak-ng, GPL-3.0), at the version whose manifest declares `links = "sherpa-onnx"`
    // (1.13.8, checked 2026-09-26; a bump has to check it again).
    assert_eq!(direct["uses_default_features"], false);
    assert_eq!(direct["features"], serde_json::json!(["shared"]));
    assert_eq!(direct["req"], "=1.13.8");
    assert_eq!(dependency("voltip-asr-local", "sherpa-onnx")["req"], direct["req"], "the bindings and the -sys crate move together");
    // voltip-asr-local's build.rs copies the Windows runtime next to its test binaries
    // (tests/windows_runtime.rs), so it needs the same ordering.
    assert_eq!(dependency("voltip-asr-local", "sherpa-onnx-sys"), direct);
}

/// The skill's Tauri version model (github-project-scaffold tauri-release.md): release-please bumps
/// the root `package.json` (plus the two app package.json files), both `tauri.conf.json` files name
/// that root file, and the Cargo version stays static — so a release commit never touches
/// Cargo.toml / Cargo.lock. `voltip --version` and the relay hello report the same version the
/// installers and the updater carry. Before (2026-09-26, release PR #6): the Cargo version was the
/// app version and nothing bumped it, so `--version` would have kept printing the old release.
#[test]
fn regression_the_app_version_is_package_json_s_and_the_cargo_version_is_static() {
    let json = |path: PathBuf| -> serde_json::Value { serde_json::from_str(&read(&path)).unwrap() };
    let root = tauri_dir().join("../../..");
    let version = json(root.join("package.json"))["version"].as_str().unwrap().to_owned();
    for app in ["desktop", "mobile"] {
        let dir = root.join("apps").join(app);
        // The root file, which the release candidate's prepare job checks (`check-config --package-json package.json`).
        assert_eq!(json(dir.join("src-tauri/tauri.conf.json"))["version"], "../../../package.json", "{app}");
        assert_eq!(json(dir.join("package.json"))["version"], version.as_str(), "{app}/package.json moves with the root");
    }
    for target in [Target::MacOS, Target::Windows, Target::Linux] {
        assert_eq!(merged(target).version.as_deref(), Some(version.as_str()), "{target:?}: Tauri reads it through the path");
    }
    assert_eq!(voltip_desktop_lib::APP_VERSION, version);
    assert_eq!(env!("CARGO_PKG_VERSION"), "0.0.0", "the Cargo version is not the app version");
    // release-please: the node strategy owns package.json; the extra files are the app package.json
    // files only — never a tauri.conf.json (it would be reformatted) or a Cargo.toml (lock drift).
    let config = json(root.join("release-please-config.json"));
    let package = &config["packages"]["."];
    assert_eq!(package["release-type"], "node");
    let extra: Vec<&str> = package["extra-files"].as_array().unwrap().iter().map(|e| e["path"].as_str().unwrap()).collect();
    assert_eq!(extra, ["apps/desktop/package.json", "apps/mobile/package.json"]);
}

/// The pill window is transparent (src/overlay.rs). On macOS `WebviewWindowBuilder::transparent`
/// exists only with Tauri's `macos-private-api` feature, and tauri-build demands the config switch
/// with it: the first macOS build (2026-09-26, GitHub-hosted macos-15) failed with E0599 on
/// `.transparent(!opaque)` without them.
#[test]
fn regression_the_transparent_pill_has_the_macos_private_api() {
    let macos = merged(Target::MacOS);
    assert!(macos.app.macos_private_api, "app.macOSPrivateApi");
    let manifest = read(&tauri_dir().join("Cargo.toml"));
    let deps = &manifest[manifest.find("\n[dependencies]").expect("[dependencies]")..];
    let deps = &deps[..deps[1..].find("\n[").map_or(deps.len(), |i| i + 1)];
    let tauri = deps.lines().find(|l| l.starts_with("tauri = ")).expect("tauri in [dependencies]");
    assert!(tauri.contains("\"macos-private-api\""), "{tauri}");
}

/// tauri-build writes the Windows resource file (exe icon, version info) from the first `.ico` in
/// `bundle.icon`, else `icons/icon.ico`, and fails the build without one. A native Windows
/// `cargo test --workspace` stopped at the mobile shell (2026-09-26), whose bundle lists a PNG.
#[test]
fn regression_every_tauri_shell_carries_the_ico_the_windows_resource_file_needs() {
    for dir in [tauri_dir(), tauri_dir().join("../../mobile/src-tauri")] {
        let (value, _) = tauri_utils::config::parse::read_from(Target::Windows, &dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        let config: Config = serde_json::from_value(value).unwrap();
        let ico = config.bundle.icon.iter().find(|i| i.ends_with(".ico")).cloned().unwrap_or_else(|| "icons/icon.ico".to_owned());
        let path = dir.join(ico);
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        // ICONDIR: reserved 0, type 1 (icon), then the image count.
        assert!(bytes.len() > 6 && bytes[..4] == [0, 0, 1, 0], "{} is not an .ico", path.display());
        assert!(u16::from_le_bytes([bytes[4], bytes[5]]) >= 1, "{} has no image", path.display());
    }
}

/// MSVC decodes a BOM-less source file in the system code page unless told otherwise. On a
/// Chinese-locale Windows (code page 936) the UTF-8 in transcribe.cpp's comments then swallows line
/// ends and model.cpp fails with C2065 (2026-09-26). `.cargo/config.toml` hands `/utf-8` to the
/// C and C++ builds of the MSVC target, unforced so cargo-xwin's clang-cl flags win when
/// cross-compiling.
#[test]
fn regression_native_msvc_builds_read_c_and_cpp_sources_as_utf8() {
    let config = read(&tauri_dir().join("../../../.cargo/config.toml"));
    for var in ["CFLAGS_x86_64_pc_windows_msvc", "CXXFLAGS_x86_64_pc_windows_msvc"] {
        assert!(config.lines().any(|l| l.trim() == format!(r#"{var} = "/utf-8""#)), "{var} missing from .cargo/config.toml");
    }
}

/// The deb and the rpm name every library the Linux binary takes from the system. 0.0.4's deb did
/// not name the BLAS transcribe.cpp links on Linux (its host decoder's `cblas_sgemv`, found on the
/// build machine): apt installed the package and `voltip-desktop` then failed with
/// `libblas.so.3: cannot open shared object file` (install-scripts.yml, 2026-09-29). The release
/// leg now refuses a binary that links a soname outside `VOLTIP_LINUX_SONAMES`
/// (scripts/lib/artefact-checks.sh); this pins the package side.
#[test]
fn regression_the_linux_packages_depend_on_the_blas_the_binary_links() {
    let linux = merged(Target::Linux);
    let deb = linux.bundle.linux.deb.depends.unwrap_or_default();
    assert!(deb.iter().any(|d| d == "libblas3 | libblas.so.3"), "{deb:?}");
    let rpm = linux.bundle.linux.rpm.depends.unwrap_or_default();
    assert!(rpm.iter().any(|d| d == "libblas.so.3()(64bit)"), "{rpm:?}");
    let checks = read(&tauri_dir().join("../../../scripts/lib/artefact-checks.sh"));
    assert!(checks.contains(" libblas.so.3"), "VOLTIP_LINUX_SONAMES lists libblas.so.3");
}
