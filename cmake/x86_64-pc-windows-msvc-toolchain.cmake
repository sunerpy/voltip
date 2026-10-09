# CMake toolchain for the Windows x64 build of the Rust workspace's C/C++ dependencies
# (transcribe.cpp / ggml through the `cmake` crate). Selected for build scripts by
# `.cargo/config.toml` (`CMAKE_TOOLCHAIN_FILE_x86_64_pc_windows_msvc`, forced over cargo-xwin's own).
#
# Why: the exe links the MSVC C runtime statically (`-C target-feature=+crt-static`, see
# .cargo/config.toml). The `cmake` crate derives `-MT` from that, but CMake >= 3.15 projects take
# the runtime from CMAKE_MSVC_RUNTIME_LIBRARY (policy CMP0091) instead, and its default is the DLL
# runtime — the objects then import MSVCP140.dll / VCRUNTIME140*.dll and the static link fails
# ("__declspec(dllimport) ..." undefined). Pinning MultiThreaded here makes both sides agree.
set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreaded" CACHE STRING "static MSVC runtime, like the Rust side" FORCE)
set(CMAKE_POLICY_DEFAULT_CMP0091 NEW CACHE STRING "" FORCE)

# Cross-compiling from Linux / macOS: chain to cargo-xwin's generated toolchain (clang-cl + the
# xwin CRT / SDK). Its location is the cargo-xwin cache: XWIN_CACHE_DIR, else the platform cache
# directory + `cargo-xwin`. On a native Windows host with MSVC nothing else is needed.
if(NOT CMAKE_HOST_WIN32)
  if(DEFINED ENV{XWIN_CACHE_DIR})
    set(_voltip_xwin_cache "$ENV{XWIN_CACHE_DIR}")
  elseif(CMAKE_HOST_APPLE)
    set(_voltip_xwin_cache "$ENV{HOME}/Library/Caches/cargo-xwin")
  elseif(DEFINED ENV{XDG_CACHE_HOME})
    set(_voltip_xwin_cache "$ENV{XDG_CACHE_HOME}/cargo-xwin")
  else()
    set(_voltip_xwin_cache "$ENV{HOME}/.cache/cargo-xwin")
  endif()
  set(_voltip_xwin_toolchain "${_voltip_xwin_cache}/cmake/clang-cl/x86_64-pc-windows-msvc-toolchain.cmake")
  if(NOT EXISTS "${_voltip_xwin_toolchain}")
    message(FATAL_ERROR "cargo-xwin toolchain not found at ${_voltip_xwin_toolchain}; build the Windows target with `cargo xwin` (set XWIN_CACHE_DIR when the cache lives elsewhere)")
  endif()
  include("${_voltip_xwin_toolchain}")
endif()
