//! Whether a paste reached the focused field (docs/dictation.md §28): the comparison of what a
//! reader of the focused control read right before the paste with what it reads afterwards. The
//! platform readers (`voltip-focus`) only read; every decision is here, in [`decide`], so one table
//! of tests covers the three platforms, and the core and the readers decide alike (the core
//! re-exports this module as `voltip_core::dictation::insert_check`).
//!
//! A reading holds the control's length, caret and selection, the selected text and at most
//! [`BEFORE_CARET_CHARS`] characters before the caret. It stays in memory for the few hundred
//! milliseconds of one paste: never stored, sent or logged, and `Debug` prints no text.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// What the insert check found right after a paste (docs/dictation.md §28): the take's text is at
/// the caret of the focused field, the field stayed exactly as it was, or neither can be told.
/// Only [`InsertCheck::NotWritten`] changes what the user sees (the 没有写进去 card).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsertCheck {
    /// The same control shows this take's text right before its caret.
    Written,
    /// The same control, read every time, kept its text, caret and selection for the whole window,
    /// and the paste was not the same text over the same selection.
    NotWritten,
    /// Anything else: unreadable, another control, a terminal or remote desktop, a change that is
    /// not this take's text.
    Unconfirmed,
}

impl InsertCheck {
    /// Wire name (`written` | `not_written` | `unconfirmed`), also what the log says.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Written => "written",
            Self::NotWritten => "not_written",
            Self::Unconfirmed => "unconfirmed",
        }
    }
}

/// Why a check could not tell (docs/dictation.md §28): for the log only, never on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnconfirmedWhy {
    /// The focused control, or its text, could not be read.
    Unreadable,
    /// Another control had the focus after the paste.
    ControlChanged,
    /// The application is one the check leaves alone (a terminal, a remote desktop, a virtual
    /// machine), or the field is a password field.
    Skipped,
    /// No application could be named for the window the paste went to (a pure Wayland session, a
    /// foreground probe that did not answer): nothing tells a terminal or a remote desktop from an
    /// ordinary window, so the field is not read.
    UnknownApp,
    /// A read did not answer within [`FOCUS_READ_DEADLINE`].
    Timeout,
    /// Nothing could show the paste: the same text over the same selection.
    NoChangeExpected,
    /// The field changed, but not into this take's text at the caret (an autocompletion, an
    /// application that rewrote the punctuation, other input at the same time).
    ChangedOtherwise,
}

/// What a paste's watch decided ([`settle_by_polling`]), and why when it could not tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settled {
    /// The verdict.
    pub check: InsertCheck,
    /// Why it is [`InsertCheck::Unconfirmed`]; `None` otherwise.
    pub why: Option<UnconfirmedWhy>,
}

impl Settled {
    /// A verdict with nothing to explain.
    pub const fn sure(check: InsertCheck) -> Self {
        Self { check, why: None }
    }

    /// [`InsertCheck::Unconfirmed`] for `why`.
    pub const fn unconfirmed(why: UnconfirmedWhy) -> Self {
        Self { check: InsertCheck::Unconfirmed, why: Some(why) }
    }
}

/// How long a check watches the focused control after a paste (docs/dictation.md §28.7): the
/// slowest change measured across the platforms (45 ms, an Edge textarea on Windows) times 1.5,
/// rounded up to 50 ms and held to at least 300 ms.
pub const CONFIRM_WITHIN: Duration = Duration::from_millis(300);
/// The longest one read of the focused control may take; a reader that does not answer by then
/// counts as unreadable (docs/dictation.md §28).
pub const FOCUS_READ_DEADLINE: Duration = Duration::from_millis(300);

/// How many characters at the end of the take's text a reading must show before its caret.
pub const EXPECTED_TAIL_CHARS: usize = 64;
/// How many characters before the caret a reading keeps (after [`normalize`]).
pub const BEFORE_CARET_CHARS: usize = 64;
/// How much a reader reads before the caret, in its own units: twice what is kept, so that line
/// breaks written as two characters (`\r\n`) still leave [`BEFORE_CARET_CHARS`] once normalised.
pub const READ_BEFORE_CARET: usize = 2 * BEFORE_CARET_CHARS;
/// The longest selection a reader reads the text of; a longer one is not compared.
pub const MAX_SELECTED_CHARS: usize = 8192;
/// How often a paste's control is read again while it settles.
pub const POLL_EVERY: Duration = Duration::from_millis(50);

/// The text as the comparison sees it: line breaks as `\n` (`\r\n`, `\r`, U+2028 and U+2029
/// included) and the no-break spaces as plain spaces.
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            '\u{2028}' | '\u{2029}' => out.push('\n'),
            '\u{a0}' | '\u{2007}' | '\u{202f}' => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// The last `n` characters of `text`.
fn last_chars(text: &str, n: usize) -> &str {
    match text.char_indices().rev().nth(n.saturating_sub(1)) {
        Some((at, _)) if n > 0 => &text[at..],
        _ if n == 0 => "",
        _ => text,
    }
}

/// What a paste is expected to leave before the caret: the take's text, [`normalize`]d and
/// trimmed (`whole`), and its last [`EXPECTED_TAIL_CHARS`] characters (`tail`).
#[derive(Clone, PartialEq, Eq)]
pub struct Expected {
    whole: String,
    tail: String,
}

impl Expected {
    /// The expectation for pasting `text`.
    pub fn of(text: &str) -> Self {
        let whole = normalize(text).trim().to_owned();
        let tail = last_chars(&whole, EXPECTED_TAIL_CHARS).to_owned();
        Self { whole, tail }
    }

    /// The last [`EXPECTED_TAIL_CHARS`] characters, as a reader may compare them.
    pub fn tail(&self) -> &str {
        &self.tail
    }

    /// The whole text, normalised and trimmed.
    pub fn whole(&self) -> &str {
        &self.whole
    }
}

impl std::fmt::Debug for Expected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Expected").field("chars", &self.whole.chars().count()).finish()
    }
}

/// One reading of the focused control. Positions are in the reader's own units (UTF-16 on
/// Windows and macOS, characters on AT-SPI): only compared with each other.
#[derive(Clone, PartialEq, Eq)]
pub struct FocusText {
    /// The control's length; `None` when it could not be read.
    chars: Option<usize>,
    /// The caret; `None` for a control that only tells its whole value.
    caret: Option<usize>,
    /// The selection `(start, end)`, empty at the caret; `None` when it could not be read.
    selection: Option<(usize, usize)>,
    /// The selected text, normalised, when the selection is not empty and was read.
    selected: Option<String>,
    /// The text right before the caret (or the end of the value), normalised, without trailing
    /// white space, at most [`BEFORE_CARET_CHARS`].
    before_caret: Option<String>,
}

impl FocusText {
    /// A reading as the platform reader got it: `before_caret` is what it read before the caret
    /// (at most [`READ_BEFORE_CARET`] of its units; the end of the value without a caret),
    /// `selected` the text of a non-empty selection of at most [`MAX_SELECTED_CHARS`].
    pub fn read(chars: Option<usize>, caret: Option<usize>, selection: Option<(usize, usize)>, selected: Option<&str>, before_caret: Option<&str>) -> Self {
        let selection = selection.map(|(a, b)| (a.min(b), a.max(b)));
        let selected = selected.filter(|_| selection.is_some_and(|(a, b)| a < b)).map(normalize);
        let before_caret = before_caret.map(|text| {
            let text = normalize(text);
            last_chars(text.trim_end(), BEFORE_CARET_CHARS).to_owned()
        });
        Self { chars, caret, selection, selected, before_caret }
    }

    /// Everything [`InsertCheck::NotWritten`] needs was read: length, caret, selection, the text
    /// before the caret, and the selected text when there is a selection.
    fn complete(&self) -> bool {
        let selection_read = self.selection.is_some_and(|(a, b)| a == b || self.selected.is_some());
        self.chars.is_some() && self.caret.is_some() && self.before_caret.is_some() && selection_read
    }

    /// The text before the caret ends with the take's.
    fn shows(&self, expected: &Expected) -> bool {
        !expected.tail.is_empty() && self.before_caret.as_deref().is_some_and(|before| before.ends_with(&expected.tail))
    }
}

impl std::fmt::Debug for FocusText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FocusText")
            .field("chars", &self.chars)
            .field("caret", &self.caret)
            .field("selection", &self.selection)
            .field("selected", &self.selected.as_ref().map(|s| s.chars().count()))
            .field("before_caret", &self.before_caret.as_ref().map(|s| s.chars().count()))
            .finish()
    }
}

/// One reading after the paste.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sample {
    /// The same control as before the paste, read.
    Text(FocusText),
    /// Another control has the focus.
    OtherControl,
    /// The control (or its text) could not be read.
    Unreadable,
    /// The reader did not answer within [`FOCUS_READ_DEADLINE`].
    TimedOut,
}

/// The verdict for a paste of `expected` whose control read `before` right before it and
/// `after` since, in order (docs/dictation.md §28):
/// - [`InsertCheck::Written`]: a reading of the same control differs from `before` and shows the
///   take's text right before its caret — the change is this paste;
/// - [`InsertCheck::NotWritten`]: every reading is of the same control, read in full, and equal to
///   `before`, and the paste was not the same text over the same selection;
/// - [`InsertCheck::Unconfirmed`] otherwise. The first decisive reading wins: another control, or
///   the text, end the reading.
pub fn decide(before: &FocusText, after: &[Sample], expected: &Expected) -> Settled {
    let (mut changed, mut unreadable, mut timed_out) = (false, false, false);
    for sample in after {
        match sample {
            Sample::OtherControl => return Settled::unconfirmed(UnconfirmedWhy::ControlChanged),
            Sample::Unreadable => unreadable = true,
            Sample::TimedOut => timed_out = true,
            Sample::Text(now) if now != before => {
                if now.shows(expected) {
                    return Settled::sure(InsertCheck::Written);
                }
                changed = true;
            }
            Sample::Text(_) => {}
        }
    }
    if changed {
        return Settled::unconfirmed(UnconfirmedWhy::ChangedOtherwise);
    }
    if timed_out {
        return Settled::unconfirmed(UnconfirmedWhy::Timeout);
    }
    if unreadable || after.is_empty() || !before.complete() {
        return Settled::unconfirmed(UnconfirmedWhy::Unreadable);
    }
    // The same text over the same selection, or nothing to write, changes nothing either way.
    if expected.whole.is_empty() || before.selected.as_deref() == Some(expected.whole.as_str()) {
        return Settled::unconfirmed(UnconfirmedWhy::NoChangeExpected);
    }
    Settled::sure(InsertCheck::NotWritten)
}

/// Whether the readings so far settle it before the window is over: the text is there, or
/// another control has the focus.
fn settled_early(before: &FocusText, sample: &Sample, expected: &Expected) -> bool {
    match sample {
        Sample::OtherControl => true,
        Sample::Text(now) => now != before && now.shows(expected),
        Sample::Unreadable | Sample::TimedOut => false,
    }
}

/// Read the control with `read` right away and then every `every` until a reading settles it or
/// `within` has passed, and decide. Also returns how long it took. Blocking: the readers' worker
/// threads bound every `read`.
pub fn settle_by_polling(before: &FocusText, expected: &Expected, within: Duration, every: Duration, mut read: impl FnMut() -> Sample) -> (Settled, Duration) {
    let started = Instant::now();
    let mut samples = Vec::new();
    loop {
        let sample = read();
        let early = settled_early(before, &sample, expected);
        samples.push(sample);
        let elapsed = started.elapsed();
        if early || elapsed >= within {
            break;
        }
        std::thread::sleep(every.min(within - elapsed));
    }
    (decide(before, &samples, expected), started.elapsed())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// A field at the caret: `text` before it, nothing selected.
    fn at_caret(text: &str) -> FocusText {
        let n = text.chars().count();
        FocusText::read(Some(n), Some(n), Some((n, n)), None, Some(text))
    }

    /// A field with `selected` selected right before the caret after `text`.
    fn selecting(text: &str, selected: &str) -> FocusText {
        let (n, s) = (text.chars().count(), selected.chars().count());
        FocusText::read(Some(n + s), Some(n + s), Some((n, n + s)), Some(selected), Some(&format!("{text}{selected}")))
    }

    fn text(sample: FocusText) -> Sample {
        Sample::Text(sample)
    }

    #[test]
    fn text_is_normalised_and_the_tail_is_the_last_64_characters() {
        assert_eq!(normalize("a\r\nb\rc\u{2028}d\u{2029}e"), "a\nb\nc\nd\ne");
        assert_eq!(normalize("a\u{a0}b\u{202f}c\u{2007}d"), "a b c d");
        let expected = Expected::of("  第一行\r\n第二\u{a0}行  \n");
        assert_eq!((expected.whole(), expected.tail()), ("第一行\n第二 行", "第一行\n第二 行"));
        let long = format!("{}{}", "前".repeat(10), "字".repeat(EXPECTED_TAIL_CHARS));
        let expected = Expected::of(&long);
        assert_eq!(expected.tail(), "字".repeat(EXPECTED_TAIL_CHARS));
        assert_eq!(expected.whole().chars().count(), 10 + EXPECTED_TAIL_CHARS);
        assert_eq!(Expected::of("  \n").tail(), "");
        assert_eq!(last_chars("abc", 0), "");
        assert_eq!(last_chars("abc", 5), "abc");
        assert_eq!(last_chars("你好世界", 2), "世界");
    }

    #[test]
    fn a_reading_keeps_64_normalised_characters_before_the_caret_and_prints_no_text() {
        let raw = format!("{}\r\n{}", "x".repeat(100), "y".repeat(10));
        let reading = FocusText::read(Some(112), Some(112), Some((112, 112)), None, Some(&raw));
        let kept = reading.before_caret.clone().unwrap();
        assert_eq!(kept.chars().count(), BEFORE_CARET_CHARS);
        assert!(kept.ends_with(&format!("x\n{}", "y".repeat(10))), "{kept}");
        let secret = FocusText::read(Some(6), Some(6), Some((0, 6)), Some("secret"), Some("secret  "));
        assert_eq!(secret.before_caret.as_deref(), Some("secret"), "trailing white space dropped");
        let printed = format!("{secret:?} {:?}", Expected::of("secret"));
        assert!(!printed.contains("secret\""), "{printed}");
        assert!(!printed.contains("\"secret"), "{printed}");
        // A selection that is empty carries no selected text, whatever the reader passed.
        assert_eq!(FocusText::read(Some(3), Some(3), Some((3, 3)), Some("abc"), Some("abc")).selected, None);
        assert_eq!(FocusText::read(None, None, Some((5, 2)), None, None).selection, Some((2, 5)), "ordered");
    }

    /// The table of docs/dictation.md §28: what each series of readings after the paste decides.
    #[test]
    fn the_verdicts_follow_the_rules_of_the_insert_check() {
        let expected = Expected::of("你好世界");
        let before = at_caret("前文");
        let written = at_caret("前文你好世界");
        let autocompleted = at_caret("前文你好世");
        type Row = (&'static str, Vec<Sample>, Settled);
        let rows: Vec<Row> = vec![
            ("the text at the caret", vec![text(written.clone())], Settled::sure(InsertCheck::Written)),
            (
                "late, after two unchanged readings",
                vec![text(before.clone()), text(before.clone()), text(written.clone())],
                Settled::sure(InsertCheck::Written),
            ),
            ("unchanged for the whole window", vec![text(before.clone()); 4], Settled::sure(InsertCheck::NotWritten)),
            ("changed, but not into the text", vec![text(autocompleted.clone())], Settled::unconfirmed(UnconfirmedWhy::ChangedOtherwise)),
            ("changed, then the text", vec![text(autocompleted), text(written.clone())], Settled::sure(InsertCheck::Written)),
            ("another control", vec![Sample::OtherControl], Settled::unconfirmed(UnconfirmedWhy::ControlChanged)),
            ("another control after the text", vec![text(written.clone()), Sample::OtherControl], Settled::sure(InsertCheck::Written)),
            ("the text after another control", vec![Sample::OtherControl, text(written)], Settled::unconfirmed(UnconfirmedWhy::ControlChanged)),
            ("one reading failed", vec![text(before.clone()), Sample::Unreadable, text(before.clone())], Settled::unconfirmed(UnconfirmedWhy::Unreadable)),
            ("one reading timed out", vec![text(before.clone()), Sample::TimedOut], Settled::unconfirmed(UnconfirmedWhy::Timeout)),
            ("no reading at all", Vec::new(), Settled::unconfirmed(UnconfirmedWhy::Unreadable)),
        ];
        for (name, after, verdict) in rows {
            assert_eq!(decide(&before, &after, &expected), verdict, "{name}");
        }
    }

    /// Regression (docs/dictation.md §28): the same text pasted over the same selection changes
    /// nothing a reader can see — it is never 没有写进去.
    #[test]
    fn regression_the_same_text_over_the_same_selection_is_unconfirmed() {
        let expected = Expected::of("你好");
        let before = selecting("前文", "你好");
        assert_eq!(decide(&before, &[text(before.clone()), text(before.clone())], &expected), Settled::unconfirmed(UnconfirmedWhy::NoChangeExpected));
        // Another selection that stayed put is a paste that went nowhere.
        let other = selecting("前文", "别的");
        assert_eq!(decide(&other, &[text(other.clone())], &expected), Settled::sure(InsertCheck::NotWritten));
        // A selection whose text could not be read cannot tell either.
        let unread = FocusText::read(Some(4), Some(4), Some((2, 4)), None, Some("前文你好"));
        assert_eq!(decide(&unread, &[text(unread.clone())], &expected), Settled::unconfirmed(UnconfirmedWhy::Unreadable));
    }

    /// A field whose text before the caret already ended with the take's text, and which stayed
    /// as it was, did not get the paste: unchanged is not written.
    #[test]
    fn unchanged_text_that_already_ended_with_the_take_is_not_written() {
        let expected = Expected::of("好的");
        let before = at_caret("好的");
        assert_eq!(decide(&before, &[text(before.clone())], &expected), Settled::sure(InsertCheck::NotWritten));
        let twice = at_caret("好的好的");
        assert_eq!(decide(&before, &[text(twice)], &expected), Settled::sure(InsertCheck::Written));
    }

    /// A control that only tells its whole value (no caret, no selection) can show the text at its
    /// end, but never prove that nothing changed.
    #[test]
    fn a_value_without_a_caret_can_be_written_but_never_not_written() {
        let expected = Expected::of("你好");
        let before = FocusText::read(Some(2), None, None, None, Some("前文"));
        assert_eq!(decide(&before, &vec![text(before.clone()); 3], &expected), Settled::unconfirmed(UnconfirmedWhy::Unreadable));
        let after = FocusText::read(Some(4), None, None, None, Some("前文你好"));
        assert_eq!(decide(&before, &[text(after)], &expected), Settled::sure(InsertCheck::Written));
    }

    /// Line endings the field writes its own way (`\r\n`, a paragraph separator) and no-break
    /// spaces still match.
    #[test]
    fn line_endings_and_no_break_spaces_still_match() {
        let expected = Expected::of("第一行\n第二 行");
        let before = at_caret("");
        let after = FocusText::read(Some(9), Some(9), Some((9, 9)), None, Some("第一行\r\n第二\u{a0}行"));
        assert_eq!(decide(&before, &[text(after)], &expected), Settled::sure(InsertCheck::Written));
        let paragraphs = FocusText::read(Some(8), Some(8), Some((8, 8)), None, Some("第一行\u{2029}第二 行\u{2029}"));
        assert_eq!(decide(&before, &[text(paragraphs)], &expected), Settled::sure(InsertCheck::Written));
        assert_eq!(decide(&before, &[text(before.clone())], &Expected::of("  ")), Settled::unconfirmed(UnconfirmedWhy::NoChangeExpected));
    }

    #[test]
    fn polling_stops_at_the_text_or_another_control_and_otherwise_at_the_window() {
        let expected = Expected::of("你好");
        let before = at_caret("");
        let mut readings = vec![text(before.clone()), text(at_caret("你好"))].into_iter();
        let mut reads = 0;
        let (verdict, _) = settle_by_polling(&before, &expected, Duration::from_secs(5), Duration::from_millis(1), || {
            reads += 1;
            readings.next().unwrap()
        });
        assert_eq!((verdict, reads), (Settled::sure(InsertCheck::Written), 2), "stopped at the text");

        let mut reads = 0;
        let (verdict, took) = settle_by_polling(&before, &expected, Duration::from_millis(40), Duration::from_millis(5), || {
            reads += 1;
            text(before.clone())
        });
        assert_eq!(verdict, Settled::sure(InsertCheck::NotWritten));
        assert!(took >= Duration::from_millis(40), "watched the whole window: {took:?}");
        assert!(reads >= 2, "read again while it watched: {reads}");

        let mut reads = 0;
        let (verdict, _) = settle_by_polling(&before, &expected, Duration::from_secs(5), Duration::from_millis(1), || {
            reads += 1;
            Sample::OtherControl
        });
        assert_eq!((verdict, reads), (Settled::unconfirmed(UnconfirmedWhy::ControlChanged), 1));
    }
}
