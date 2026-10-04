//! Leaving the process, as the desktop shell does (`apps/desktop/src-tauri/src/exit.rs`, docs/
//! dictation.md §10.6). On Linux the C runtime's exit handlers and the shared libraries' destructors
//! are skipped: once the Vulkan backend has enumerated an NVIDIA GPU, the driver's own teardown
//! frees the same memory twice and turns a normal exit into `double free or corruption` and
//! SIGABRT (L40S, driver 580.126, 2026-09-27). Nothing of the server needs an exit handler: it
//! writes nothing but its uploads, which are removed before this, and the standard streams are
//! flushed here first.
#![cfg_attr(target_os = "linux", allow(unsafe_code))]

use std::io::Write as _;

/// Flush stdout and stderr, then end the process with `code`.
pub fn exit_process(code: i32) -> ! {
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    #[cfg(target_os = "linux")]
    // SAFETY: `_exit` takes a plain int, has no preconditions and never returns.
    unsafe {
        libc::_exit(code)
    }
    #[cfg(not(target_os = "linux"))]
    std::process::exit(code)
}
