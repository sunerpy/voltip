//! The insert check's reader on Windows (docs/dictation.md §28): UI Automation in the reader's own
//! multithreaded COM apartment. The focused control is `GetFocusedElement`, compared with
//! `CompareElements`; a password field is never read. Its text comes from `TextPattern2` (the
//! caret range) or `TextPattern` (the selection stands for the caret), with the document's length,
//! the selection and the text before the caret; a document over [`MAX_DOCUMENT`] is not read.
//! Without either pattern, `ValuePattern`'s whole value stands in, with no caret (which can only
//! ever confirm a paste, never report one missing). Every call is bounded by UI Automation's
//! connection and transaction timeouts on top of the worker thread's deadline.
//!
//! The first query makes Chromium-based applications turn on their accessibility tree for as long
//! as they run, which can slow down large pages; 检查文字是否写入 turns the reading off.

// The crate denies `unsafe`; COM calls are FFI. SAFETY (every block below): the interfaces are
// valid COM pointers obtained from UI Automation on this thread's apartment, the calls follow their
// documented contracts, and no pointer outlives the object that owns it.
#![allow(unsafe_code)]

use voltip_platform::insert_check::rules::{FocusText, MAX_SELECTED_CHARS, UnconfirmedWhy};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize};
use windows::Win32::UI::Accessibility::{
    CUIAutomation8, IUIAutomation, IUIAutomation2, IUIAutomationElement, IUIAutomationTextPattern, IUIAutomationTextPattern2, IUIAutomationTextRange,
    IUIAutomationValuePattern, TextPatternRangeEndpoint, TextPatternRangeEndpoint_End, TextPatternRangeEndpoint_Start, UIA_TextPattern2Id, UIA_TextPatternId,
    UIA_ValuePatternId,
};
use windows::core::{BOOL, Interface as _};

use crate::{FocusPlatform, utf16_before, utf16_text};

/// UI Automation's connection timeout: how long it waits for a provider to answer at all.
pub const CONNECTION_TIMEOUT_MS: u32 = 200;
/// UI Automation's transaction timeout: how long one call may take.
pub const TRANSACTION_TIMEOUT_MS: u32 = 300;
/// The longest document read, in UTF-16 units (1 Mi); a longer one is not read.
pub const MAX_DOCUMENT: usize = 1 << 20;

/// COM initialised on the reader's thread; uninitialised when the reader goes.
struct Apartment;

impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: balances the successful `CoInitializeEx` of `open`, on the same thread.
        unsafe { CoUninitialize() };
    }
}

/// UI Automation on the reader's thread. Field order matters: the automation object is released
/// before the apartment ends.
pub struct Uia {
    automation: IUIAutomation,
    _apartment: Apartment,
}

/// Initialise COM on this thread and create UI Automation, with its timeouts.
pub fn open() -> Result<Uia, String> {
    // SAFETY: called once on the reader's new thread, before any other COM call there.
    let initialised = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if initialised.is_err() {
        return Err(format!("CoInitializeEx: {initialised:?}"));
    }
    let apartment = Apartment;
    // SAFETY: COM is initialised on this thread; the class id and interface are UI Automation's.
    let automation: IUIAutomation = unsafe { CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER) }.map_err(|e| format!("CUIAutomation8: {e}"))?;
    if let Ok(two) = automation.cast::<IUIAutomation2>() {
        // SAFETY: a valid IUIAutomation2; plain setters.
        unsafe {
            if let Err(e) = two.SetConnectionTimeout(CONNECTION_TIMEOUT_MS) {
                tracing::debug!(error = %e, "UI Automation connection timeout not set");
            }
            if let Err(e) = two.SetTransactionTimeout(TRANSACTION_TIMEOUT_MS) {
                tracing::debug!(error = %e, "UI Automation transaction timeout not set");
            }
        }
    }
    Ok(Uia { automation, _apartment: apartment })
}

/// `text` as UTF-16 units, or `None` when it is over `limit`.
fn units(text: &windows::core::BSTR, limit: usize) -> Option<Vec<u16>> {
    let wide: &[u16] = text;
    (wide.len() <= limit).then(|| wide.to_vec())
}

/// The longest text UI Automation is asked for at once (one unit over the limit, so a longer one
/// shows).
fn ask_limit(limit: usize) -> i32 {
    i32::try_from(limit + 1).unwrap_or(i32::MAX)
}

/// How many UTF-16 units of the document come before `range`'s `endpoint`.
fn offset_of(document: &IUIAutomationTextRange, range: &IUIAutomationTextRange, endpoint: TextPatternRangeEndpoint) -> Option<usize> {
    // SAFETY: valid ranges of the same pattern.
    unsafe {
        let head = document.Clone().ok()?;
        head.MoveEndpointByRange(TextPatternRangeEndpoint_End, range, endpoint).ok()?;
        let text = head.GetText(ask_limit(MAX_DOCUMENT)).ok()?;
        (text.len() <= MAX_DOCUMENT).then_some(text.len())
    }
}

/// The text facts of a `TextPattern`, with `caret` from `TextPattern2` when there is one (else the
/// end of the first selection range).
fn read_pattern(pattern: &IUIAutomationTextPattern, caret: Option<IUIAutomationTextRange>) -> Option<FocusText> {
    // SAFETY: a valid pattern of the focused element.
    let document = unsafe { pattern.DocumentRange() }.ok()?;
    // SAFETY: as above.
    let whole = units(&unsafe { document.GetText(ask_limit(MAX_DOCUMENT)) }.ok()?, MAX_DOCUMENT)?;
    // SAFETY: as above; an element without a selection answers an empty array.
    let selection = unsafe { pattern.GetSelection() }
        .ok()
        .and_then(|ranges| (unsafe { ranges.Length() }.ok()? > 0).then(|| unsafe { ranges.GetElement(0) }.ok()).flatten());
    let (bounds, selected) = match &selection {
        Some(range) => {
            let start = offset_of(&document, range, TextPatternRangeEndpoint_Start)?;
            let end = offset_of(&document, range, TextPatternRangeEndpoint_End)?;
            // SAFETY: a valid range of the pattern.
            let selected = (end > start && end - start <= MAX_SELECTED_CHARS)
                .then(|| unsafe { range.GetText(ask_limit(MAX_SELECTED_CHARS)) }.ok().and_then(|t| units(&t, MAX_SELECTED_CHARS)))
                .flatten()
                .map(|t| utf16_text(&t));
            (Some((start, end)), selected)
        }
        None => (None, None),
    };
    let caret = match caret {
        Some(range) => Some(offset_of(&document, &range, TextPatternRangeEndpoint_Start)?),
        None => bounds.map(|(_, end)| end),
    };
    let before = caret.map_or_else(|| utf16_before(&whole, whole.len()), |caret| utf16_before(&whole, caret));
    Some(FocusText::read(Some(whole.len()), caret, bounds, selected.as_deref(), Some(&before)))
}

/// The whole value of a control with `ValuePattern` only: no caret.
fn read_value(element: &IUIAutomationElement) -> Option<FocusText> {
    // SAFETY: a valid element; an unsupported pattern is an error.
    let pattern = unsafe { element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) }.ok()?;
    // SAFETY: a valid pattern.
    let value = units(&unsafe { pattern.CurrentValue() }.ok()?, MAX_DOCUMENT)?;
    Some(FocusText::read(Some(value.len()), None, None, None, Some(&utf16_before(&value, value.len()))))
}

impl FocusPlatform for Uia {
    type Control = IUIAutomationElement;

    fn focused(&mut self) -> Result<IUIAutomationElement, UnconfirmedWhy> {
        // SAFETY: a valid IUIAutomation on this thread.
        let element = unsafe { self.automation.GetFocusedElement() }.map_err(|_| UnconfirmedWhy::Unreadable)?;
        // SAFETY: a valid element.
        if unsafe { element.CurrentIsPassword() }.is_ok_and(BOOL::as_bool) {
            return Err(UnconfirmedWhy::Skipped);
        }
        Ok(element)
    }

    fn same(&mut self, a: &IUIAutomationElement, b: &IUIAutomationElement) -> bool {
        // SAFETY: valid elements of this automation object.
        unsafe { self.automation.CompareElements(a, b) }.is_ok_and(BOOL::as_bool)
    }

    fn text(&mut self, element: &IUIAutomationElement) -> Option<FocusText> {
        // SAFETY: a valid element; unsupported patterns are errors.
        if let Ok(two) = unsafe { element.GetCurrentPatternAs::<IUIAutomationTextPattern2>(UIA_TextPattern2Id) } {
            let mut active = BOOL::default();
            // SAFETY: a valid pattern; `active` outlives the call.
            let caret = unsafe { two.GetCaretRange(&raw mut active) }.ok();
            if let Some(text) = read_pattern(&two, caret) {
                return Some(text);
            }
        }
        // SAFETY: as above.
        if let Ok(one) = unsafe { element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) }
            && let Some(text) = read_pattern(&one, None)
        {
            return Some(text);
        }
        read_value(element)
    }
}
