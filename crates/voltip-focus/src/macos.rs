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
use core_foundation_sys::string::{CFStringGetCharacters, CFStringGetLength};
use voltip_platform::insert_check::rules::{CONTEXT_CHARS, FocusContext, FocusText, MAX_SELECTED_CHARS, READ_BEFORE_CARET, UnconfirmedWhy};

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

/// `value` as text, read as UTF-16 units: a range cut inside a surrogate pair (the learning read
/// asks for ranges at any unit, §35) reads as U+FFFD at that end, where core-foundation's
/// `to_string` asserts and would end the reader's thread (the review of 2026-11-25).
fn string(value: &CFType) -> Option<String> {
    let text = value.downcast::<CFString>()?;
    let raw = text.as_concrete_TypeRef();
    // SAFETY: a live CFString.
    let length = unsafe { CFStringGetLength(raw) };
    let mut units = vec![0u16; usize::try_from(length).ok()?];
    // SAFETY: `units` holds exactly `length` UniChars, the range is the whole string.
    unsafe { CFStringGetCharacters(raw, CFRange { location: 0, length }, units.as_mut_ptr()) };
    Some(String::from_utf16_lossy(&units))
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

    fn context(&mut self, element: &CFType, hint: Option<&str>) -> Option<FocusContext> {
        let chars = copy(element, "AXNumberOfCharacters").and_then(|n| index(&n));
        let selection = copy(element, "AXSelectedTextRange").and_then(|r| range(&r));
        if let (Some(chars), Some((location, length))) = (chars, selection) {
            if length > MAX_SELECTED_CHARS {
                return None;
            }
            let (start, end) = (location.min(chars), (location + length).min(chars));
            // Two units a character at most, and a few more: what `FocusContext` keeps of each side.
            let window = 2 * CONTEXT_CHARS + 4;
            let before = string_for_range(element, start.saturating_sub(window), start)?;
            let selected = if end > start { string_for_range(element, start, end)? } else { String::new() };
            let after = string_for_range(element, end, (end + window).min(chars))?;
            return FocusContext::from_parts(&before, &selected, &after);
        }
        // Only the whole value: the copied selection places the context.
        let value = copy(element, "AXValue").and_then(|v| string(&v))?;
        FocusContext::find(&value, hint?)
    }
}

#[cfg(test)]
mod tests {
    use core_foundation_sys::base::kCFAllocatorDefault;
    use core_foundation_sys::string::CFStringCreateWithCharacters;

    use super::*;

    /// The review of 2026-11-25: a string with an unpaired surrogate, as `AXStringForRange` gives
    /// for a range cut inside a pair, reads with U+FFFD there instead of ending the thread.
    #[test]
    fn regression_a_range_cut_inside_a_pair_reads_with_a_replacement_character() {
        let units: [u16; 3] = [0xDC00, 0x0041, 0xD83D];
        // SAFETY: `units` outlives the call; the result follows the create rule.
        let raw = unsafe { CFStringCreateWithCharacters(kCFAllocatorDefault, units.as_ptr(), 3) };
        assert!(!raw.is_null());
        // SAFETY: a new reference we own.
        let value = unsafe { CFType::wrap_under_create_rule(raw.cast()) };
        assert_eq!(string(&value).as_deref(), Some("\u{fffd}A\u{fffd}"));
    }
}
