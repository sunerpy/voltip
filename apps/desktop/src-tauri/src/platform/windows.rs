//! Windows facts (docs/dictation.md §15.3, §18.2): the foreground window's token integrity level
//! versus our own, the input desktop's name, the microphone rows of the CapabilityAccessManager
//! consent store, and the foreground application (image name + window title) scenes match on.
//! Every decision is made by `voltip_platform`; this file only asks Win32.
//!
//! One of the shell's two Win32 modules that contain `unsafe` (with `solo_key/windows.rs`):
//! `windows-sys` is raw FFI. Each
//! block has a SAFETY comment; every handle is wrapped so it is closed exactly once; no pointer
//! outlives the buffer it points into.

#![allow(unsafe_code)]

use std::ffi::c_void;
use std::ptr;

use voltip_core::ForegroundApp;
use voltip_platform::foreground::from_exe_path;
use voltip_platform::permissions::{Permission, PermissionReport};
use voltip_platform::windows::{
    ConsentValue, ForegroundFacts, InjectPreflight, IntegrityLevel, MicrophoneConsent, MicrophonePolicy, StackedWindow, WEBVIEW_PROCESS, consent_store_app_key,
    is_paste_target, microphone_consent,
};
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE, HWND, RECT, S_OK};
use windows_sys::Win32::Globalization::GetUserDefaultUILanguage;
use windows_sys::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows_sys::Win32::Security::{GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TokenIntegrityLevel};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, REG_DWORD, REG_EXPAND_SZ, REG_SZ, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
};
use windows_sys::Win32::System::StationsAndDesktops::{CloseDesktop, DESKTOP_READOBJECTS, GetUserObjectInformationW, OpenInputDesktop, UOI_NAME};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, OpenProcess, OpenProcessToken, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows_sys::Win32::UI::HiDpi::{GetDpiForSystem, GetSystemMetricsForDpi};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GW_HWNDNEXT, GWL_EXSTYLE, GetClassNameW, GetForegroundWindow, GetWindow, GetWindowLongW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible, SM_CXSMICON, SetForegroundWindow, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};

/// `PRIMARYLANGID` of a Chinese `LANGID` (`LANG_CHINESE`).
const LANG_CHINESE: u16 = 0x04;

/// Whether the display language is Chinese (`GetUserDefaultUILanguage`), which is what WebView2
/// reports as `navigator.language` and so what `settings.locale = "system"` resolves with.
pub fn ui_language_is_chinese() -> bool {
    // SAFETY: no arguments; returns the calling user's UI language id.
    let langid = unsafe { GetUserDefaultUILanguage() };
    langid & 0x3ff == LANG_CHINESE
}

/// The small-icon size (`SM_CXSMICON`) at the system DPI: what the notification area draws a tray
/// icon at. `None` when the call fails (it returns 0).
pub fn small_icon_size() -> Option<u32> {
    // SAFETY: plain metric queries without pointers.
    let size = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem()) };
    u32::try_from(size).ok().filter(|&size| size > 0)
}

/// `HKCU` subkey of the microphone consent store.
const CONSENT_MICROPHONE: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";
/// `HKLM` subkey of the app-privacy group policy.
const POLICY_APP_PRIVACY: &str = r"SOFTWARE\Policies\Microsoft\Windows\AppPrivacy";

/// Settings ▸ Privacy & security ▸ Microphone, where the "desktop apps" switch lives.
pub const MICROPHONE_SETTINGS_URI: &str = "ms-settings:privacy-microphone";

/// Windows never shows a consent prompt to a desktop app: the microphone request opens the Settings
/// page instead. Accessibility is macOS-only and has nothing to ask.
pub fn permissions_request(permission: Permission) -> Result<(), String> {
    match permission {
        Permission::Microphone => tauri_plugin_opener::open_url(MICROPHONE_SETTINGS_URI, None::<&str>).map_err(|e| e.to_string()),
        Permission::Accessibility => Ok(()),
    }
}

/// The microphone consent state; Accessibility stays `not_applicable`.
pub fn permissions_status() -> PermissionReport {
    PermissionReport::for_host().with(Permission::Microphone, microphone_consent(&read_microphone_consent()))
}

/// Compare the foreground window's integrity level with ours and look at the input desktop.
pub fn inject_preflight() -> InjectPreflight {
    InjectPreflight::from_facts(foreground_facts())
}

/// The application in front when a take starts (docs/dictation.md §18.2): the foreground window's
/// process image name (the same chain as the preflight) and the window's title. Voltip's own window
/// and "no window has the focus" are no answer; a process that refuses to be opened is an error.
pub fn foreground_app() -> Result<Option<ForegroundApp>, String> {
    // SAFETY: no arguments; a null result means no window has the focus.
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_null() {
        return Ok(None);
    }
    let mut pid = 0u32;
    // SAFETY: `hwnd` came from `GetForegroundWindow` a moment ago (a window that has since closed
    // makes the call return 0, which is handled); `pid` is a valid out-pointer.
    let thread = unsafe { GetWindowThreadProcessId(hwnd, &raw mut pid) };
    if thread == 0 || pid == 0 {
        return Ok(None);
    }
    // SAFETY: no arguments.
    if pid == unsafe { GetCurrentProcessId() } {
        return Ok(None);
    }
    // SAFETY: plain call with a valid access mask; a null handle is checked before use.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(format!("OpenProcess refused for pid {pid}"));
    }
    let process = OwnedHandle(process);
    let Some(identity) = image_name(process.0).as_deref().and_then(from_exe_path) else { return Ok(None) };
    Ok(Some(ForegroundApp { app_id: identity.app_id, name: identity.name, title: window_title(hwnd), window: Some(hwnd as usize as u64) }))
}

/// The history paste's way back (`crate::paste`): Voltip's window in front and the window the user
/// came from. Handles travel as integers: `HWND` is a raw pointer, which is not `Send`.
#[derive(Clone, Copy, Debug)]
pub struct PasteReturn {
    voltip: isize,
    target: isize,
}

/// How far down the Z order the walk goes before it gives up.
const MAX_STACKED_WINDOWS: usize = 512;

/// Voltip's window in front and the first ordinary application window below it
/// (`voltip_platform::windows::is_paste_target`): activating a window puts it on top of the
/// others, so the one right under Voltip is the one that was active before it. `None` when none of
/// Voltip's windows is in front, or nothing below qualifies.
pub fn paste_return() -> Option<PasteReturn> {
    // SAFETY: no arguments.
    let own = unsafe { GetCurrentProcessId() };
    // SAFETY: no arguments; a null result means no window has the focus.
    let front = unsafe { GetForegroundWindow() };
    if front.is_null() || process_of(front) != Some(own) {
        return None;
    }
    // SAFETY: `front` is a window handle from `GetForegroundWindow`; a stale one returns null.
    let mut hwnd = unsafe { GetWindow(front, GW_HWNDNEXT) };
    for _ in 0..MAX_STACKED_WINDOWS {
        if hwnd.is_null() {
            return None;
        }
        let class = class_name(hwnd);
        if is_paste_target(&StackedWindow { class: &class, ..stacked_facts(hwnd, own) }) {
            tracing::debug!(class, image = ?owner_image(hwnd), "paste: the window below Voltip");
            return Some(PasteReturn { voltip: front as isize, target: hwnd as isize });
        }
        // SAFETY: as above, `hwnd` came from `GetWindow` a moment ago.
        hwnd = unsafe { GetWindow(hwnd, GW_HWNDNEXT) };
    }
    None
}

/// Once Voltip's window is minimised, put the window the user came from in front, unless another
/// application already is: Windows usually activates it on `SW_MINIMIZE`, but when it leaves
/// nothing in front (or Voltip's own window) the call is allowed, no window or Voltip holding the
/// foreground. `false` while Voltip's window is not minimised yet (the minimise runs on the event
/// loop), so the caller asks again.
pub fn return_to(back: PasteReturn) -> bool {
    // SAFETY: plain calls on window handles; a window that has closed since makes them fail
    // harmlessly (0 / null).
    unsafe {
        if IsIconic(back.voltip as HWND) == 0 {
            return false;
        }
        let front = GetForegroundWindow();
        if front.is_null() || process_of(front) == Some(GetCurrentProcessId()) {
            let brought = SetForegroundWindow(back.target as HWND) != 0;
            tracing::info!(brought, "paste: brought the window below Voltip to the front");
        }
    }
    true
}

/// The process that owns `hwnd`.
fn process_of(hwnd: HWND) -> Option<u32> {
    let mut pid = 0u32;
    // SAFETY: `hwnd` is a window handle; `pid` is a valid out-pointer. A stale handle returns 0.
    let thread = unsafe { GetWindowThreadProcessId(hwnd, &raw mut pid) };
    (thread != 0 && pid != 0).then_some(pid)
}

/// What `is_paste_target` needs about `hwnd`, but its class.
fn stacked_facts(hwnd: HWND, own: u32) -> StackedWindow<'static> {
    let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    let mut cloak = 0u32;
    // SAFETY: `hwnd` is a window handle (a stale one makes every call fail: 0, which reads as
    // hidden / empty); `rect` and `cloak` are valid out-pointers of the sizes passed.
    unsafe {
        let visible = IsWindowVisible(hwnd) != 0;
        let minimized = IsIconic(hwnd) != 0;
        let empty = GetWindowRect(hwnd, &raw mut rect) == 0 || rect.right <= rect.left || rect.bottom <= rect.top;
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE).cast_unsigned();
        let tool = ex_style & (WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE) != 0;
        let size = u32::try_from(size_of::<u32>()).unwrap_or(4);
        let cloaked = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED.cast_unsigned(), (&raw mut cloak).cast(), size) == S_OK && cloak != 0;
        let own_process = process_of(hwnd) == Some(own);
        // Only a window every other check lets through is worth opening its process for.
        let webview = visible
            && !minimized
            && !empty
            && !tool
            && !cloaked
            && !own_process
            && owner_image(hwnd).is_some_and(|name| name.eq_ignore_ascii_case(WEBVIEW_PROCESS));
        StackedWindow { visible, minimized, empty, tool, cloaked, own_process, webview, class: "" }
    }
}

/// The image name of the process that owns `hwnd` (`notepad.exe`); `None` when it cannot be read.
fn owner_image(hwnd: HWND) -> Option<String> {
    let pid = process_of(hwnd)?;
    // SAFETY: plain call with a valid access mask; a null handle (access refused) is checked.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return None;
    }
    let process = OwnedHandle(process);
    image_name(process.0)
}

/// The window class of `hwnd` (`""` when it cannot be read).
fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let cap = i32::try_from(buf.len()).unwrap_or(256);
    // SAFETY: `buf` has `cap` UTF-16 slots, the most the call writes; it returns the number copied.
    let copied = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), cap) };
    let copied = usize::try_from(copied).unwrap_or(0).min(buf.len());
    String::from_utf16_lossy(&buf[..copied])
}

/// The caption of `hwnd`. For a window of another process `GetWindowTextW` reads the cached
/// caption without sending `WM_GETTEXT`, so a hung application cannot block the probe.
fn window_title(hwnd: HWND) -> Option<String> {
    // SAFETY: `hwnd` is a window handle from `GetForegroundWindow`; a stale one makes it return 0.
    let len = unsafe { GetWindowTextLengthW(hwnd) };
    let len = usize::try_from(len).ok().filter(|&n| n > 0)?;
    let mut buf = vec![0u16; len + 1];
    let cap = i32::try_from(buf.len()).ok()?;
    // SAFETY: `buf` has `cap` UTF-16 slots, the most the call writes (the NUL included); it returns
    // the number of characters copied, which is checked against the buffer below.
    let copied = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), cap) };
    let copied = usize::try_from(copied).ok().filter(|&n| n > 0)?;
    Some(String::from_utf16_lossy(&buf[..copied.min(len)]))
}

fn read_microphone_consent() -> MicrophoneConsent {
    let app_key = std::env::current_exe().ok().map(|p| consent_store_app_key(&p.to_string_lossy()));
    let value = |sub: &str| read_string(HKEY_CURRENT_USER, sub, "Value").as_deref().and_then(ConsentValue::parse);
    MicrophoneConsent {
        policy: read_dword(HKEY_LOCAL_MACHINE, POLICY_APP_PRIVACY, "LetAppsAccessMicrophone").and_then(MicrophonePolicy::from_dword),
        global: value(CONSENT_MICROPHONE),
        non_packaged: value(&format!(r"{CONSENT_MICROPHONE}\NonPackaged")),
        app: app_key.and_then(|k| value(&format!(r"{CONSENT_MICROPHONE}\NonPackaged\{k}"))),
    }
}

fn foreground_facts() -> ForegroundFacts {
    // SAFETY: `GetCurrentProcess` takes no arguments and returns a pseudo-handle that must not be
    // closed; it is never wrapped in `OwnedHandle`.
    let own = unsafe { GetCurrentProcess() };
    let self_level = token_level(own);
    // SAFETY: no arguments; a null result means no window has the focus.
    let hwnd = unsafe { GetForegroundWindow() };
    let (target_level, target_process) = if hwnd.is_null() {
        (None, None)
    } else {
        let mut pid = 0u32;
        // SAFETY: `hwnd` came from `GetForegroundWindow` a moment ago (a window that has since
        // closed makes the call return 0, which is handled); `pid` is a valid out-pointer.
        let thread = unsafe { GetWindowThreadProcessId(hwnd, &raw mut pid) };
        if thread == 0 || pid == 0 {
            (None, None)
        } else {
            // SAFETY: plain call with a valid access mask; a null handle (access refused, e.g. a
            // protected process) is checked before use.
            let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
            if process.is_null() {
                (None, None)
            } else {
                let process = OwnedHandle(process);
                (token_level(process.0), image_name(process.0))
            }
        }
    };
    ForegroundFacts { self_level, target_level, secure_desktop: secure_desktop(), target_process }
}

/// A kernel handle closed on drop.
struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: the handle was returned by a successful `OpenProcess` / `OpenProcessToken` and
        // is closed exactly once, here.
        unsafe { CloseHandle(self.0) };
    }
}

/// The mandatory integrity level of `process`'s primary token.
fn token_level(process: HANDLE) -> Option<IntegrityLevel> {
    let mut token: HANDLE = ptr::null_mut();
    // SAFETY: `process` is a live process handle (or the current-process pseudo-handle); `token`
    // is a valid out-pointer.
    let opened = unsafe { OpenProcessToken(process, TOKEN_QUERY, &raw mut token) };
    if opened == 0 || token.is_null() {
        return None;
    }
    let token = OwnedHandle(token);
    let mut needed = 0u32;
    // SAFETY: the size probe: a null buffer of length 0 fails with ERROR_INSUFFICIENT_BUFFER and
    // writes the required byte count into `needed`, a valid out-pointer.
    unsafe { GetTokenInformation(token.0, TokenIntegrityLevel, ptr::null_mut(), 0, &raw mut needed) };
    if needed == 0 {
        return None;
    }
    // u64 slots: TOKEN_MANDATORY_LABEL starts with a pointer, so the buffer must be 8-byte aligned.
    let mut buf = vec![0u64; (needed as usize).div_ceil(8)];
    // SAFETY: `buf` is at least `needed` bytes long and 8-byte aligned, which is what
    // TOKEN_MANDATORY_LABEL needs; `needed` is a valid out-pointer.
    let ok = unsafe { GetTokenInformation(token.0, TokenIntegrityLevel, buf.as_mut_ptr().cast::<c_void>(), needed, &raw mut needed) };
    if ok == 0 {
        return None;
    }
    // SAFETY: on success the buffer holds an initialised TOKEN_MANDATORY_LABEL whose `Sid` points
    // inside the same buffer, which stays alive for the rest of this function.
    let label = unsafe { &*buf.as_ptr().cast::<TOKEN_MANDATORY_LABEL>() };
    let sid = label.Label.Sid;
    if sid.is_null() {
        return None;
    }
    // SAFETY: `sid` is a valid SID (written by the API) inside `buf`; the returned pointer is
    // read once while `buf` is alive.
    let count = unsafe { *GetSidSubAuthorityCount(sid) };
    if count == 0 {
        return None;
    }
    // SAFETY: `count - 1` is the index of the last sub-authority of this SID, so the pointer is
    // in bounds; read once while `buf` is alive.
    let rid = unsafe { *GetSidSubAuthority(sid, u32::from(count) - 1) };
    Some(IntegrityLevel::from_rid(rid))
}

/// File name of `process`'s image (`explorer.exe`).
fn image_name(process: HANDLE) -> Option<String> {
    let mut buf = vec![0u16; 1024];
    let mut len = u32::try_from(buf.len()).ok()?;
    // SAFETY: `buf` has `len` UTF-16 slots; the API writes at most that many and updates `len`
    // to the characters written; both pointers are valid for the call.
    let ok = unsafe { QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &raw mut len) };
    if ok == 0 {
        return None;
    }
    let path = String::from_utf16_lossy(&buf[..(len as usize).min(buf.len())]);
    path.rsplit(['\\', '/']).next().filter(|n| !n.is_empty()).map(str::to_owned)
}

/// `Some(true)` when the input desktop is not the interactive `Default` desktop (`Winlogon` during
/// a UAC prompt or on the lock screen, `Screen-saver`); `None` when it cannot be opened.
fn secure_desktop() -> Option<bool> {
    // SAFETY: plain call; a null handle means the desktop could not be opened.
    let desktop = unsafe { OpenInputDesktop(0, 0, DESKTOP_READOBJECTS) };
    if desktop.is_null() {
        return None;
    }
    let mut buf = vec![0u16; 256];
    let mut needed = 0u32;
    let bytes = u32::try_from(buf.len() * 2).ok()?;
    // SAFETY: `desktop` is the live handle opened above; `buf` is `bytes` bytes long, which is the
    // length passed; `needed` is a valid out-pointer.
    let ok = unsafe { GetUserObjectInformationW(desktop, UOI_NAME, buf.as_mut_ptr().cast::<c_void>(), bytes, &raw mut needed) };
    // SAFETY: closes the handle opened above exactly once.
    unsafe { CloseDesktop(desktop) };
    if ok == 0 {
        return None;
    }
    let end = buf.iter().position(|&u| u == 0).unwrap_or(buf.len());
    let name = String::from_utf16_lossy(&buf[..end]);
    Some(!name.eq_ignore_ascii_case("Default"))
}

/// A registry key closed on drop.
struct RegKey(HKEY);

impl Drop for RegKey {
    fn drop(&mut self) {
        // SAFETY: the key was returned by a successful `RegOpenKeyExW` and is closed exactly once.
        unsafe { RegCloseKey(self.0) };
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn open_key(root: HKEY, path: &str) -> Option<RegKey> {
    let path = wide(path);
    let mut key: HKEY = ptr::null_mut();
    // SAFETY: `path` is NUL-terminated and alive for the call; `key` is a valid out-pointer.
    let rc = unsafe { RegOpenKeyExW(root, path.as_ptr(), 0, KEY_READ, &raw mut key) };
    (rc == ERROR_SUCCESS && !key.is_null()).then_some(RegKey(key))
}

/// `(type, bytes)` of one value.
fn query_value(key: &RegKey, name: &str) -> Option<(u32, Vec<u8>)> {
    let name = wide(name);
    let mut kind = 0u32;
    let mut len = 0u32;
    // SAFETY: a null data pointer asks for the size only; `kind` and `len` are valid out-pointers.
    let rc = unsafe { RegQueryValueExW(key.0, name.as_ptr(), ptr::null(), &raw mut kind, ptr::null_mut(), &raw mut len) };
    if rc != ERROR_SUCCESS || len == 0 {
        return None;
    }
    let mut buf = vec![0u8; len as usize];
    // SAFETY: `buf` holds exactly `len` bytes, the maximum the API writes; `len` is updated to the
    // bytes written and the buffer is truncated to it.
    let rc = unsafe { RegQueryValueExW(key.0, name.as_ptr(), ptr::null(), &raw mut kind, buf.as_mut_ptr(), &raw mut len) };
    if rc != ERROR_SUCCESS {
        return None;
    }
    buf.truncate((len as usize).min(buf.len()));
    Some((kind, buf))
}

fn read_string(root: HKEY, path: &str, name: &str) -> Option<String> {
    let key = open_key(root, path)?;
    let (kind, bytes) = query_value(&key, name)?;
    if kind != REG_SZ && kind != REG_EXPAND_SZ {
        return None;
    }
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    let end = units.iter().position(|&u| u == 0).unwrap_or(units.len());
    Some(String::from_utf16_lossy(&units[..end]))
}

fn read_dword(root: HKEY, path: &str, name: &str) -> Option<u32> {
    let key = open_key(root, path)?;
    let (kind, bytes) = query_value(&key, name)?;
    if kind != REG_DWORD {
        return None;
    }
    let (head, _) = bytes.split_first_chunk::<4>()?;
    Some(u32::from_le_bytes(*head))
}
