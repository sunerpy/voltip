//! Windows only. The sherpa-onnx runtime has to sit next to the test binaries (build.rs): the
//! loader looks in the exe's directory before System32, and since Windows 11 24H2 / Server 2025
//! System32 carries its own `onnxruntime.dll` (1.17), which the first recognizer then called into
//! and died with an access violation (2026-09-26, Windows Server 2025).
#![cfg(windows)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[test]
fn regression_the_sherpa_onnx_runtime_sits_next_to_the_test_binaries() {
    let exe = std::env::current_exe().unwrap();
    let dir = exe.parent().unwrap();
    for dll in ["sherpa-onnx-c-api.dll", "onnxruntime.dll", "onnxruntime_providers_shared.dll"] {
        assert!(dir.join(dll).is_file(), "{dll} missing next to {}", exe.display());
    }
}
