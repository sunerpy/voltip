//! sherpa-onnx is linked dynamically; its build script copies the runtime (the two shared objects,
//! the two dylibs or the three DLLs) into `target/<profile>/` and adds an rpath to its prebuilt
//! directory on ELF / Mach-O. The test binaries of this crate also get an `$ORIGIN` /
//! `@loader_path` rpath.
//!
//! Windows has no rpath: the loader looks in the exe's directory, then System32, and only then on
//! PATH (where `cargo test` puts the runtime). Since Windows 11 24H2 / Server 2025 System32 carries
//! its own `onnxruntime.dll` (1.17, Windows ML), so the test binaries in `target/<profile>/deps/`
//! loaded that one and died on the first recognizer ("The requested API version [28] is not
//! available", then an access violation; 2026-09-26, Windows Server 2025). The three DLLs are
//! therefore copied next to them. Cargo.toml names sherpa-onnx-sys directly so that its build
//! script, which downloads and copies them, always runs before this one.
//!
//! macOS builds transcribe.cpp with Metal (docs/dictation.md §10.6). ggml's Metal code guards newer
//! APIs with `@available(...)`, which clang compiles to calls of `__isPlatformVersionAtLeast` from
//! its compiler runtime; rustc links with `-nodefaultlibs`, so that runtime is named here, or the
//! final link fails with `Undefined symbols: ___isPlatformVersionAtLeast` (macos.yml, 2026-09-27).
//! Raising the deployment target would compile the checks away but drop macOS 11-13.
//!
//! A Vulkan build (the `vulkan` feature, §10.6) links the loader, `vulkan-1.lib` or `libvulkan.so`,
//! which transcribe.cpp's link manifest names without a directory. The packaging scripts say where
//! it is (`VOLTIP_VULKAN_LIB_DIR`, scripts/lib/vulkan-sdk.sh); without it the loader has to be on the
//! linker's own search path (a system `libvulkan-dev`).

use std::path::{Path, PathBuf};

const WINDOWS_RUNTIME_DLLS: [&str; 3] = ["sherpa-onnx-c-api.dll", "onnxruntime.dll", "onnxruntime_providers_shared.dll"];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=VOLTIP_VULKAN_LIB_DIR");
    if std::env::var_os("CARGO_FEATURE_VULKAN").is_some()
        && let Some(dir) = std::env::var_os("VOLTIP_VULKAN_LIB_DIR")
    {
        println!("cargo:rustc-link-search=native={}", PathBuf::from(dir).display());
    }
    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("linux") | Ok("android") => println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN"),
        Ok("macos") => {
            println!("cargo:rustc-link-arg=-Wl,-rpath,@loader_path");
            link_clang_runtime("clang_rt.osx");
        }
        Ok("ios") => println!("cargo:rustc-link-arg=-Wl,-rpath,@loader_path"),
        Ok("windows") => copy_runtime_next_to_the_test_binaries(),
        _ => {}
    }
}

/// Link clang's compiler runtime archive `lib<name>.a` from the resource directory of the C compiler
/// the C/C++ dependencies were built with (`CC`, else `clang`).
fn link_clang_runtime(name: &str) {
    let clang = std::env::var("CC").unwrap_or_else(|_| "clang".to_owned());
    let resource = match std::process::Command::new(&clang).arg("-print-resource-dir").output() {
        Ok(out) if out.status.success() => PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()),
        Ok(out) => {
            println!("cargo:warning={clang} -print-resource-dir failed ({}); lib{name}.a is not linked", out.status);
            return;
        }
        Err(e) => {
            println!("cargo:warning=could not run {clang} -print-resource-dir ({e}); lib{name}.a is not linked");
            return;
        }
    };
    let dir = resource.join("lib").join("darwin");
    if dir.join(format!("lib{name}.a")).is_file() {
        println!("cargo:rustc-link-search=native={}", dir.display());
        // A plain `-l`: the linker picks the archive up at the final link (not bundled in the rlib).
        println!("cargo:rustc-link-lib={name}");
    } else {
        println!("cargo:warning=lib{name}.a not found in {}; the Metal backend may not link", dir.display());
    }
}

fn copy_runtime_next_to_the_test_binaries() {
    // OUT_DIR = <target>/<triple>/<profile>/build/voltip-asr-local-<hash>/out → profile dir is 3 up.
    let Some(profile) = std::env::var_os("OUT_DIR").map(PathBuf::from).and_then(|out| out.ancestors().nth(3).map(Path::to_path_buf)) else {
        return;
    };
    let deps = profile.join("deps");
    if let Err(e) = std::fs::create_dir_all(&deps) {
        println!("cargo:warning=could not create {}: {e}", deps.display());
        return;
    }
    for name in WINDOWS_RUNTIME_DLLS {
        let src = profile.join(name);
        match std::fs::copy(&src, deps.join(name)) {
            Ok(_) => println!("cargo:rerun-if-changed={}", src.display()),
            Err(e) => println!("cargo:warning=sherpa-onnx runtime {name} not copied next to the test binaries from {}: {e}", src.display()),
        }
    }
}
