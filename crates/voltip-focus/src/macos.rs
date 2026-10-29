//! The insert check's reader on macOS (docs/dictation.md §28): the Accessibility API, which the
//! paste already needs (no new permission). The focused control is the system-wide element's
//! `AXFocusedUIElement`, compared with `CFEqual`; a secure text field is never read. Its text comes
//! from `AXNumberOfCharacters`, `AXSelectedTextRange` and `AXSelectedText` (the selection, whose end
//! stands for the caret) and `AXStringForRange` (the text before the caret); without those,
//! `AXValue`'s whole value stands in, with no caret (which can only ever confirm a paste). Every
//! call is bounded by the messaging timeout on top of the worker thread's deadline. The
//! declarations are written here: no crate in the graph binds `AXUIElement`.

// The crate denies `unsafe`; the Accessibility API is FFI. SAFETY (every block below): the
// elements and values are live CF objects this module owns (create rule) or borrows for the call,
// out-pointers point at locals, and every returned object is wrapped under the create rule once.
#![allow(unsafe_code)]

use std::ffi::c_void;

use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};
use core_foundation_sys::base::{CFIndex, CFRange};
use voltip_platform::insert_check::rules::{FocusText, MAX_SELECTED_CHARS, READ_BEFORE_CARET, UnconfirmedWhy};

use crate::FocusPlatform;

/// How long one Accessibility call may wait for the application, in seconds.
pub const MESSAGING_TIMEOUT_SECS: f32 = 0.3;

type AXError = i32;
type AXValueType = u32;
const AX_ERROR_SUCCESS: AXError = 0;
/// `kAXValueCFRangeType`.
const AX_VALUE_CF_RANGE: AXValueType = 4;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCreateSystemWide() -> CFTypeRef;
    fn AXUIElementCopyAttributeValue(element: CFTypeRef, attribute: CFStringRef, value: *mut CFTypeRef) -> AXError;
    fn AXUIElementCopyParameterizedAttributeValue(element: CFTypeRef, attribute: CFStringRef, parameter: CFTypeRef, result: *mut CFTypeRef) -> AXError;
    fn AXUIElementSetMessagingTimeout(element: CFTypeRef, timeout: f32) -> AXError;
    fn AXValueCreate(kind: AXValueType, value: *const c_void) -> CFTypeRef;
    fn AXValueGetValue(value: CFTypeRef, kind: AXValueType, out: *mut c_void) -> u8;
}

/// The Accessibility API on the reader's thread.
pub struct Ax {
    system: CFType,
}

/// The system-wide element, with the messaging timeout.
pub fn open() -> Result<Ax, String> {
    // SAFETY: no arguments; the result follows the create rule.
    let raw = unsafe { AXUIElementCreateSystemWide() };
    if raw.is_null() {
        return Err("AXUIElementCreateSystemWide".into());
    }
    // SAFETY: a new reference we own.
    let system = unsafe { CFType::wrap_under_create_rule(raw) };
    // SAFETY: a live element; on the system-wide element the timeout applies to every element.
    let set = unsafe { AXUIElementSetMessagingTimeout(system.as_CFTypeRef(), MESSAGING_TIMEOUT_SECS) };
    if set != AX_ERROR_SUCCESS {
        tracing::debug!(error = set, "Accessibility messaging timeout not set");
    }
    Ok(Ax { system })
}

/// `element`'s `attribute`, when it has one.
fn copy(element: &CFType, attribute: &'static str) -> Option<CFType> {
    let name = CFString::from_static_string(attribute);
    let mut value: CFTypeRef = std::ptr::null();
    // SAFETY: live element and name; `value` is a local out-pointer.
    let error = unsafe { AXUIElementCopyAttributeValue(element.as_CFTypeRef(), name.as_concrete_TypeRef(), &raw mut value) };
    // SAFETY: on success `value` is a new reference we own.
    (error == AX_ERROR_SUCCESS && !value.is_null()).then(|| unsafe { CFType::wrap_under_create_rule(value) })
}

fn string(value: &CFType) -> Option<String> {
    value.downcast::<CFString>().map(|s| s.to_string())
}

fn index(value: &CFType) -> Option<usize> {
    value.downcast::<CFNumber>().and_then(|n| n.to_i64()).and_then(|n| usize::try_from(n).ok())
}

/// An `AXValue` holding a `CFRange`, as `(location, length)`.
fn range(value: &CFType) -> Option<(usize, usize)> {
    let mut out = CFRange { location: 0, length: 0 };
    // SAFETY: a live value; `out` is a local of the type `kAXValueCFRangeType` names.
    let ok = unsafe { AXValueGetValue(value.as_CFTypeRef(), AX_VALUE_CF_RANGE, (&raw mut out).cast()) };
    (ok != 0).then(|| Some((usize::try_from(out.location).ok()?, usize::try_from(out.length).ok()?))).flatten()
}

/// `element`'s text in `[start, end)` (UTF-16 units), through `AXStringForRange`.
fn string_for_range(element: &CFType, start: usize, end: usize) -> Option<String> {
    let wanted = CFRange { location: CFIndex::try_from(start).ok()?, length: CFIndex::try_from(end.saturating_sub(start)).ok()? };
    // SAFETY: `wanted` is a local CFRange; the result follows the create rule.
    let raw = unsafe { AXValueCreate(AX_VALUE_CF_RANGE, (&raw const wanted).cast()) };
    if raw.is_null() {
        return None;
    }
    // SAFETY: a new reference we own.
    let parameter = unsafe { CFType::wrap_under_create_rule(raw) };
    let name = CFString::from_static_string("AXStringForRange");
    let mut value: CFTypeRef = std::ptr::null();
    // SAFETY: live element, name and parameter; `value` is a local out-pointer.
    let error =
        unsafe { AXUIElementCopyParameterizedAttributeValue(element.as_CFTypeRef(), name.as_concrete_TypeRef(), parameter.as_CFTypeRef(), &raw mut value) };
    if error != AX_ERROR_SUCCESS || value.is_null() {
        return None;
    }
    // SAFETY: on success `value` is a new reference we own.
    string(&unsafe { CFType::wrap_under_create_rule(value) })
}

impl FocusPlatform for Ax {
    type Control = CFType;

    fn focused(&mut self) -> Result<CFType, UnconfirmedWhy> {
        let element = copy(&self.system, "AXFocusedUIElement").ok_or(UnconfirmedWhy::Unreadable)?;
        if copy(&element, "AXSubrole").and_then(|s| string(&s)).is_some_and(|subrole| subrole == "AXSecureTextField") {
            return Err(UnconfirmedWhy::Skipped);
        }
        Ok(element)
    }

    fn same(&mut self, a: &CFType, b: &CFType) -> bool {
        a == b
    }

    fn text(&mut self, element: &CFType) -> Option<FocusText> {
        let chars = copy(element, "AXNumberOfCharacters").and_then(|n| index(&n));
        let selection = copy(element, "AXSelectedTextRange").and_then(|r| range(&r));
        if let (Some(chars), Some((location, length))) = (chars, selection) {
            let (start, end) = (location, location + length);
            let selected = (length > 0 && length <= MAX_SELECTED_CHARS).then(|| copy(element, "AXSelectedText").and_then(|s| string(&s))).flatten();
            let caret = end.min(chars);
            if let Some(before) = string_for_range(element, caret.saturating_sub(READ_BEFORE_CARET), caret) {
                return Some(FocusText::read(Some(chars), Some(caret), Some((start, end)), selected.as_deref(), Some(&before)));
            }
        }
        // Only the whole value: its end stands for the text before the caret.
        let value = copy(element, "AXValue").and_then(|v| string(&v))?;
        let units: Vec<u16> = value.encode_utf16().collect();
        Some(FocusText::read(Some(units.len()), None, None, None, Some(&crate::utf16_before(&units, units.len()))))
    }
}
