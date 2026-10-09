use std::path::PathBuf;

/// The dynamic sherpa-onnx runtime (voltip-asr-local → sherpa-onnx-sys) is copied by that crate's
/// build script into the target profile directory. Cargo runs that build script before this one
/// because Cargo.toml names sherpa-onnx-sys directly (it has `links`; a transitive dependency alone
/// gives no ordering), so by now the DLLs exist; `tauri_build` validates `bundle.resources` paths
/// while it runs, so the DLLs are staged into a stable in-tree directory first
/// (`resources/windows/`, git-ignored) that `tauri.windows.conf.json` points at.
const WINDOWS_RUNTIME_DLLS: [&str; 3] = ["sherpa-onnx-c-api.dll", "onnxruntime.dll", "onnxruntime_providers_shared.dll"];

/// Same story on Linux (docs/dictation.md §14): the two shared objects the binary links are staged
/// into `resources/linux/` (git-ignored) and `tauri.linux.conf.json` ships them as
/// `/usr/lib/voltip/` through the deb / rpm / AppImage `files` maps. `libsherpa-onnx-cxx-api.so`
/// is also dropped next to the binary but nothing links it, so it is not shipped.
const LINUX_RUNTIME_LIBS: [&str; 2] = ["libsherpa-onnx-c-api.so", "libonnxruntime.so"];

/// macOS (docs/dictation.md §15.5): the two dylibs the binary links, staged into `resources/macos/`
/// (git-ignored) for `bundle.macOS.frameworks` in `tauri.macos.conf.json`. `tauri_build` copies
/// them to `target/Frameworks/` and adds `-rpath @executable_path/../Frameworks` itself; the bundler
/// puts them into `Voltip.app/Contents/Frameworks/` and signs them with the app when an identity is
/// set. Both carry `@rpath/` install names (`libsherpa-onnx-c-api.dylib` loads
/// `@rpath/libonnxruntime.dylib`, minos 11.0). `libsherpa-onnx-cxx-api.dylib` is not linked.
const MACOS_RUNTIME_DYLIBS: [&str; 2] = ["libsherpa-onnx-c-api.dylib", "libonnxruntime.dylib"];

/// Where the packaged Linux binary finds the staged runtime: `/usr/bin/<bin>` → `/usr/lib/voltip/`
/// in the deb / rpm tree, `usr/bin/<bin>` → `usr/lib/voltip/` inside the AppDir. Must match the
/// `files` keys in `tauri.linux.conf.json`.
const LINUX_BUNDLED_LIB_DIR: &str = "$ORIGIN/../lib/voltip";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    emit_app_version();
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    match target_os.as_str() {
        "windows" => stage_runtime_libs("windows", &WINDOWS_RUNTIME_DLLS),
        "linux" => stage_runtime_libs("linux", &LINUX_RUNTIME_LIBS),
        "macos" => stage_runtime_libs("macos", &MACOS_RUNTIME_DYLIBS),
        _ => {}
    }
    tauri_build::build();
    // tauri-build links its Windows resource (icon, version, and the manifest that asks for Common
    // Controls v6) into the application binary only. tauri-plugin-dialog's message dialogs import
    // `TaskDialogIndirect`, which only Common Controls v6 has: an integration test that builds the
    // app (tests/ipc.rs, tests/update.rs) would not even start without the manifest
    // (STATUS_ENTRYPOINT_NOT_FOUND). The test binaries get the same resource.
    if target_os == "windows" {
        let resource = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default()).join("resource.lib");
        if resource.exists() {
            println!("cargo:rustc-link-arg-tests={}", resource.display());
        }
    }
    // Static CRT (.cargo/config.toml `+crt-static`): the C++ objects from transcribe.cpp / ggml
    // and Rust std then need the static UCRT. `cargo xwin build` adds it by itself, but the Tauri
    // CLI invokes the binary as `cargo-xwin build …`, where that step does not run and the link
    // fails on `round` / `floor` from the math library. Naming the archive here works in both
    // invocation forms because the ucrt lib directory is always on the link path.
    if target_os == "windows" && std::env::var("CARGO_CFG_TARGET_FEATURE").is_ok_and(|f| f.split(',').any(|x| x == "crt-static")) {
        println!("cargo:rustc-link-arg=libucrt.lib");
    }
    // ELF / Mach-O loaders do not look next to the executable by default; Windows does.
    // Linux gets two entries: `$ORIGIN` for `cargo run` / the smoke scripts (the .so files sit next
    // to the binary in target/<profile>/) and the packaged layout of `tauri.linux.conf.json`.
    match target_os.as_str() {
        "linux" => println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN:{LINUX_BUNDLED_LIB_DIR}"),
        // `@loader_path` for `cargo run` (the dylibs sit next to the binary); the bundle's
        // `@executable_path/../Frameworks` entry comes from `tauri_build` (see MACOS_RUNTIME_DYLIBS).
        "macos" => println!("cargo:rustc-link-arg=-Wl,-rpath,@loader_path"),
        _ => {}
    }
}

/// The app version as `VOLTIP_APP_VERSION` (`voltip --version`, the relay hello). release-please
/// bumps the root `package.json`; `tauri.conf.json` names it (`"../../../package.json"`), so the
/// installers, the updater and this value all read the same field, and the Cargo version stays
/// static — a release commit never touches `Cargo.toml` or `Cargo.lock` (skill
/// github-project-scaffold, tauri-release.md "Version model"). A literal version is taken as is.
fn emit_app_version() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let conf = dir.join("tauri.conf.json");
    println!("cargo:rerun-if-changed={}", conf.display());
    let version = match json_field(&conf, "version") {
        Ok(v) if v.ends_with(".json") => {
            let package = dir.join(&v);
            println!("cargo:rerun-if-changed={}", package.display());
            json_field(&package, "version")
        }
        other => other,
    };
    match version {
        Ok(version) => println!("cargo:rustc-env=VOLTIP_APP_VERSION={version}"),
        Err(e) => panic!("no app version: {e}"),
    }
}

/// The string at `key` in the JSON object in `path`.
fn json_field(path: &std::path::Path, key: &str) -> Result<String, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    value.get(key).and_then(serde_json::Value::as_str).map(str::to_owned).ok_or_else(|| format!("{}: no string `{key}`", path.display()))
}

/// Copy `names` from the profile directory into `resources/<platform>/`, warning (not failing) for
/// each one that is missing: a plain `cargo check` never produced them.
fn stage_runtime_libs(platform: &str, names: &[&str]) {
    // OUT_DIR = <target>/<triple>/<profile>/build/voltip-desktop-<hash>/out → profile dir is 3 up.
    let Ok(out_dir) = std::env::var("OUT_DIR") else { return };
    let profile_dir = PathBuf::from(out_dir).ancestors().nth(3).map(PathBuf::from);
    let Some(profile_dir) = profile_dir else { return };
    let staging = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources").join(platform);
    if std::fs::create_dir_all(&staging).is_err() {
        println!("cargo:warning=could not create {}", staging.display());
        return;
    }
    for name in names {
        let src = profile_dir.join(name);
        let dst = staging.join(name);
        match std::fs::copy(&src, &dst) {
            Ok(_) => println!("cargo:rerun-if-changed={}", src.display()),
            Err(e) => println!("cargo:warning=sherpa-onnx runtime {} not staged from {}: {e}", name, src.display()),
        }
    }
}
