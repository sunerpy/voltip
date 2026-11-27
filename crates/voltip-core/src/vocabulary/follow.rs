//! 自动跟踪输入框里的修改 (docs/dictation.md §35.10), the state of following one take's field after
//! its paste: readings of the kept control in, what to store and when to stop out. No IO and no
//! clock: the runtime passes the time since the follow began, reads the field, writes the history
//! and releases the control (`crate::runtime::edit_follow`).
//!
//! The first reading places the take in its paragraph and keeps what the field shows of it (the
//! baseline: the application's own rewrites, such as Word's capital at the start of a sentence, are
//! in it and are not the user's). Every later reading carries the take's place over from the one
//! before ([`follow_next`]) and rebuilds the take with the user's changes only
//! ([`follow_corrected`]). A rebuilt take is stored once it has stood [`FOLLOW_STABLE`] (a
//! half-typed word or an input method's pinyin keeps changing), when it differs from the take and
//! holds a change of the recogniser's wording ([`storable`]), and changes at most
//! [`FOLLOW_MAX_CHANGED`] of the take's words. Nothing here logs a text.

use std::time::Duration;

use super::learn::{CUT_MARGIN, FollowNext, FollowSpot, changed_share, follow_corrected, follow_first, follow_next, storable};
use crate::dictation::insert_check::normalize;
use crate::dictation::{CONTEXT_CHARS, FocusContext, UnconfirmedWhy};
use crate::history::HistoryText;

/// How long a rebuilt take stands before it is stored.
pub const FOLLOW_STABLE: Duration = Duration::from_secs(1);
/// How long the last rebuilt take must have stood when the take leaves the field (the message was
/// sent, the text deleted) or the field is left, to be stored then.
pub const FOLLOW_GONE_STABLE: Duration = Duration::from_millis(500);
/// No change for this long ends the follow.
pub const FOLLOW_IDLE: Duration = Duration::from_secs(120);
/// The follow ends this long after the paste at the latest.
pub const FOLLOW_MAX: Duration = Duration::from_secs(600);
/// The wait between two readings.
pub const FOLLOW_EVERY: Duration = Duration::from_millis(500);
/// The wait between two readings right after a change ([`FOLLOW_FAST_FOR`]).
pub const FOLLOW_FAST: Duration = Duration::from_millis(250);
/// How long after a change the readings come every [`FOLLOW_FAST`].
pub const FOLLOW_FAST_FOR: Duration = Duration::from_secs(2);
/// A rebuilt take that leaves less than this share of the take's words is a rewrite: not stored.
pub const FOLLOW_MAX_CHANGED: f64 = 0.5;

/// The follow's times (`CoreConfig.follow_times`; tests shorten them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FollowTimes {
    /// [`FOLLOW_STABLE`].
    pub stable: Duration,
    /// [`FOLLOW_GONE_STABLE`].
    pub gone_stable: Duration,
    /// [`FOLLOW_IDLE`].
    pub idle: Duration,
    /// [`FOLLOW_MAX`].
    pub max: Duration,
    /// [`FOLLOW_EVERY`].
    pub every: Duration,
    /// [`FOLLOW_FAST`].
    pub fast: Duration,
    /// [`FOLLOW_FAST_FOR`].
    pub fast_for: Duration,
}

impl Default for FollowTimes {
    fn default() -> Self {
        Self {
            stable: FOLLOW_STABLE,
            gone_stable: FOLLOW_GONE_STABLE,
            idle: FOLLOW_IDLE,
            max: FOLLOW_MAX,
            every: FOLLOW_EVERY,
            fast: FOLLOW_FAST,
            fast_for: FOLLOW_FAST_FOR,
        }
    }
}

/// What to write into the take's history entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FollowWrite {
    /// The take as the user corrected it (`CorrectedText { via: Follow }`).
    Store(String),
    /// The user put the take back as it was: the follow's correction goes.
    Clear,
}

/// Why a follow ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FollowEnd {
    /// The first reading did not hold the take.
    NotFound,
    /// Another control has the focus.
    FocusMoved,
    /// The field could not be read (closed, locked, a password field, too slow).
    Unreadable,
    /// The take left the field: sent, deleted or undone.
    Gone,
    /// No change for [`FOLLOW_IDLE`].
    Idle,
    /// [`FOLLOW_MAX`] since the paste.
    Expired,
}

impl FollowEnd {
    /// The name the log uses.
    pub fn name(self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::FocusMoved => "focus_moved",
            Self::Unreadable => "unreadable",
            Self::Gone => "gone",
            Self::Idle => "idle",
            Self::Expired => "expired",
        }
    }
}

/// What one reading leads to: a write first, then the end, both in one handling.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FollowStep {
    /// What to write, if anything.
    pub write: Option<FollowWrite>,
    /// The follow ends.
    pub end: Option<FollowEnd>,
}

impl FollowStep {
    fn end(write: Option<FollowWrite>, end: FollowEnd) -> Self {
        Self { write, end: Some(end) }
    }
}

/// The rebuilt take, from when it first showed to when it was last read.
#[derive(Clone, Debug)]
struct Candidate {
    text: String,
    since: Duration,
    seen: Duration,
}

/// One take's follow.
#[derive(Debug)]
pub struct EditFollow {
    take: HistoryText,
    /// The take as inserted, normalised (A).
    inserted: String,
    times: FollowTimes,
    spot: Option<FollowSpot>,
    /// What the field showed of the take right after the paste (B0).
    baseline: String,
    /// The field's text of the take where its place is followed from: the baseline, then what was
    /// read when this follow last wrote (a stored correction's words are the take's from then on).
    reference: String,
    candidate: Option<Candidate>,
    /// What this follow stored last; `None` when it stored nothing, or cleared it.
    stored: Option<String>,
    /// Where the take's paragraph started in the control at the last reading that held it, when
    /// the reader said ([`FocusContext::paragraph_at`]).
    paragraph_at: Option<usize>,
    /// Readings in a row that did not hold the take.
    missing: u32,
    /// When the field's take last changed; `None` before the first change.
    changed: Option<Duration>,
    /// Since when nothing changed: the first reading, or the last change.
    quiet: Duration,
    reads: u32,
}

impl EditFollow {
    /// A follow of `take` (its text as inserted; its `corrected` is not looked at), or `None` for a
    /// take the paragraph read cannot hold whole with some text beside it: one with a line break,
    /// or longer than [`CONTEXT_CHARS`] less [`CUT_MARGIN`] (2026-11-28: a longer one was taken for
    /// cut by the window at every reading).
    pub fn new(take: HistoryText, times: FollowTimes) -> Option<Self> {
        let inserted = normalize(&take.text);
        if inserted.contains('\n') || inserted.chars().count() > CONTEXT_CHARS - CUT_MARGIN || inserted.trim().is_empty() {
            return None;
        }
        Some(Self {
            take,
            inserted,
            times,
            spot: None,
            baseline: String::new(),
            reference: String::new(),
            candidate: None,
            stored: None,
            paragraph_at: None,
            missing: 0,
            changed: None,
            quiet: Duration::ZERO,
            reads: 0,
        })
    }

    /// The take's history entry.
    pub fn id(&self) -> uuid::Uuid {
        self.take.id
    }

    /// Readings so far.
    pub fn reads(&self) -> u32 {
        self.reads
    }

    /// The wait before the next reading, at `now` since the follow began.
    pub fn next_in(&self, now: Duration) -> Duration {
        if self.changed.is_some_and(|at| now < at + self.times.fast_for) { self.times.fast } else { self.times.every }
    }

    /// One reading of the kept control at `now` since the paste.
    pub fn on_reading(&mut self, now: Duration, reading: Result<FocusContext, UnconfirmedWhy>) -> FollowStep {
        self.reads += 1;
        let context = match reading {
            Ok(context) => context,
            Err(UnconfirmedWhy::ControlChanged) => return FollowStep::end(self.settle(self.times.gone_stable), FollowEnd::FocusMoved),
            Err(_) => return FollowStep::end(self.settle(self.times.gone_stable), FollowEnd::Unreadable),
        };
        let Some(previous) = self.spot.clone() else {
            return match follow_first(&context.before, &context.selected, &context.after, &self.take) {
                Some(spot) => {
                    self.baseline = spot.text();
                    self.reference = self.baseline.clone();
                    self.spot = Some(spot);
                    self.paragraph_at = context.paragraph_at;
                    // The take as inserted is what stands first: no change yet.
                    self.candidate = Some(Candidate { text: self.inserted.clone(), since: now, seen: now });
                    self.quiet = now;
                    FollowStep::default()
                }
                None => FollowStep::end(None, FollowEnd::NotFound),
            };
        };
        // A paragraph that starts elsewhere in the control and reads otherwise is another one (the
        // caret went to another line that reads alike), whatever it holds.
        let paragraph = format!("{}{}{}", context.before, context.selected, context.after);
        // A field read empty has lost the take: sent, or all deleted. What is typed into it next
        // is another message (2026-11-28: 「好的」 sent and 「好啊」 typed was stored as its
        // correction).
        if paragraph.is_empty() {
            return FollowStep::end(self.settle(self.times.gone_stable), FollowEnd::Gone);
        }
        let moved = matches!((self.paragraph_at, context.paragraph_at), (Some(was), Some(now)) if was != now) && paragraph != previous.paragraph;
        let next = if moved { FollowNext::Gone } else { follow_next(&previous, &context.before, &context.selected, &context.after, &self.reference) };
        match next {
            // The reading's window cuts the take, or is too long to compare: it says nothing.
            FollowNext::Cut | FollowNext::TooLong => {}
            FollowNext::Gone => {
                self.missing += 1;
                if self.missing >= 2 {
                    return FollowStep::end(self.settle(self.times.gone_stable), FollowEnd::Gone);
                }
            }
            // A word at the take's start or end is being typed: its place is carried on, and the
            // field shows no candidate now. The next one must stand its second anew, and nothing
            // older is written if the follow ends meanwhile (2026-11-28: the take cut short, read
            // again after such readings, was stored at once).
            FollowNext::Typing(spot) => {
                self.paragraph_at = context.paragraph_at;
                self.missing = 0;
                // Typing goes on while the field changes; standing still, it is no change (2026-11-28:
                // a field left as it was never ended idle).
                if spot.paragraph != previous.paragraph {
                    (self.changed, self.quiet) = (Some(now), now);
                }
                self.spot = Some(spot);
                self.candidate = None;
            }
            FollowNext::Read(spot) => {
                self.paragraph_at = context.paragraph_at;
                self.missing = 0;
                // A space typed right after the take belongs to what follows it: the text read ends
                // as the baseline does.
                let read = spot.text();
                let read = format!("{}{}", read.trim_end(), &self.baseline[self.baseline.trim_end().len()..]);
                let rebuilt = follow_corrected(&self.inserted, &self.baseline, &read).unwrap_or_else(|| self.inserted.clone());
                // The white space at the take's ends is the take's as inserted (2026-11-28: the space
                // starting the text after it was stored with a corrected last word).
                let inserted = self.inserted.as_str();
                let rebuilt =
                    format!("{}{}{}", &inserted[..inserted.len() - inserted.trim_start().len()], rebuilt.trim(), &inserted[inserted.trim_end().len()..]);
                self.spot = Some(spot);
                match &mut self.candidate {
                    Some(c) if c.text == rebuilt => c.seen = now,
                    _ => {
                        self.candidate = Some(Candidate { text: rebuilt, since: now, seen: now });
                        (self.changed, self.quiet) = (Some(now), now);
                    }
                }
            }
        }
        let write = self.settle(self.times.stable);
        // What was written is the take from now on: its place is followed from it.
        if write.is_some()
            && let Some(spot) = &mut self.spot
        {
            spot.fix();
            self.reference = spot.text();
        }
        if now >= self.times.max {
            return FollowStep::end(write, FollowEnd::Expired);
        }
        if now.saturating_sub(self.quiet) >= self.times.idle {
            return FollowStep::end(write, FollowEnd::Idle);
        }
        FollowStep { write, end: None }
    }

    /// The write the rebuilt take calls for, once it has stood `long`.
    fn settle(&mut self, long: Duration) -> Option<FollowWrite> {
        let c = self.candidate.as_ref()?;
        if c.seen.saturating_sub(c.since) < long {
            return None;
        }
        let text = c.text.clone();
        // A rewrite says nothing about the take's words: what was stored stays.
        if changed_share(&self.inserted, &text).is_none_or(|share| share > FOLLOW_MAX_CHANGED) {
            return None;
        }
        // Put back as it was, or with no change left that the suggestions could learn (taken back
        // together with another edit): the follow's correction goes.
        if text == self.inserted || !storable(&self.take, &text) {
            return self.stored.take().map(|_| FollowWrite::Clear);
        }
        if self.stored.as_deref() == Some(text.as_str()) {
            return None;
        }
        self.stored = Some(text.clone());
        Some(FollowWrite::Store(text))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::vocabulary::VocabularyHits;

    fn take(text: &str) -> HistoryText {
        HistoryText {
            id: Uuid::new_v4(),
            at_ms: 10,
            raw_text: text.into(),
            text: text.into(),
            refined: false,
            vocabulary: VocabularyHits::default(),
            corrected: None,
            corrected_via: None,
        }
    }

    fn field(before: &str) -> Result<FocusContext, UnconfirmedWhy> {
        FocusContext::from_parts(before, "", "").ok_or(UnconfirmedWhy::Unreadable)
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    const TAKE: &str = "今天测试一下千问四Max的推理能力";
    const FIXED: &str = "今天测试一下Qwen 4 Max的推理能力";

    /// The paste, then the user's correction, read every 250 ms until the follow ends, as the
    /// runtime reads; the steps that write or end.
    fn run(follow: &mut EditFollow, readings: &[(u64, Result<FocusContext, UnconfirmedWhy>)]) -> Vec<(u64, FollowStep)> {
        let mut steps = Vec::new();
        for (at, reading) in readings {
            let step = follow.on_reading(ms(*at), reading.clone());
            let end = step.end.is_some();
            if step != FollowStep::default() {
                steps.push((*at, step));
            }
            if end {
                break;
            }
        }
        steps
    }

    #[test]
    fn a_correction_that_stands_a_second_is_stored_once() {
        let mut follow = EditFollow::new(take(TAKE), FollowTimes::default()).expect("followed");
        let steps = run(
            &mut follow,
            &[
                (0, field(TAKE)),
                (500, field(TAKE)),
                (1000, field(FIXED)),
                (1250, field(FIXED)),
                (1750, field(FIXED)),
                (2000, field(FIXED)),
                (2500, field(FIXED)),
            ],
        );
        assert_eq!(steps, [(2000, FollowStep { write: Some(FollowWrite::Store(FIXED.into())), end: None })]);
    }

    #[test]
    fn half_typed_words_and_pinyin_that_keep_changing_are_not_stored() {
        let mut follow = EditFollow::new(take(TAKE), FollowTimes::default()).expect("followed");
        let typing = ["今天测试一下Q Max的推理能力", "今天测试一下Qw Max的推理能力", "今天测试一下qwen Max的推理能力", "今天测试一下Qwen Max的推理能力"];
        let mut readings: Vec<(u64, Result<FocusContext, UnconfirmedWhy>)> = vec![(0, field(TAKE))];
        for (i, text) in typing.iter().enumerate() {
            readings.push((250 * (i as u64 + 1), field(text)));
        }
        assert!(run(&mut follow, &readings).is_empty(), "nothing stood a second");
    }

    #[test]
    fn a_take_sent_after_its_correction_stood_half_a_second_is_stored_as_it_ends() {
        let mut follow = EditFollow::new(take(TAKE), FollowTimes::default()).expect("followed");
        let steps = run(&mut follow, &[(0, field(TAKE)), (250, field(FIXED)), (500, field(FIXED)), (750, field(FIXED)), (1000, field("")), (1250, field(""))]);
        // The field read empty ends the follow at once (2026-11-28: before, a second reading without
        // the take was waited for).
        assert_eq!(steps, [(1000, FollowStep { write: Some(FollowWrite::Store(FIXED.into())), end: Some(FollowEnd::Gone) })]);
    }

    #[test]
    fn a_take_sent_right_after_its_correction_ends_without_a_write() {
        let mut follow = EditFollow::new(take(TAKE), FollowTimes::default()).expect("followed");
        let steps = run(&mut follow, &[(0, field(TAKE)), (250, field(FIXED)), (500, field("")), (750, field(""))]);
        assert_eq!(steps, [(500, FollowStep { write: None, end: Some(FollowEnd::Gone) })]);
    }

    #[test]
    fn a_correction_taken_back_clears_what_the_follow_stored() {
        let mut follow = EditFollow::new(take(TAKE), FollowTimes::default()).expect("followed");
        let mut readings = vec![(0, field(TAKE))];
        readings.extend((1..=5).map(|i| (250 * i, field(FIXED))));
        readings.extend((6..=11).map(|i| (250 * i, field(TAKE))));
        let steps = run(&mut follow, &readings);
        // Each stood a second: the correction from 250 ms, the take put back from 1500 ms.
        assert_eq!(
            steps,
            [
                (1250, FollowStep { write: Some(FollowWrite::Store(FIXED.into())), end: None }),
                (2500, FollowStep { write: Some(FollowWrite::Clear), end: None })
            ]
        );
    }

    /// Word's capital at the start was there right after the paste: it is the application's, and
    /// only the user's change is stored.
    #[test]
    fn a_rewrite_of_the_application_in_the_first_reading_is_not_stored() {
        let mut follow = EditFollow::new(take("quinn for max is fast"), FollowTimes::default()).expect("followed");
        let mut readings = vec![(0, field("Quinn for max is fast"))];
        readings.extend((1..=5).map(|i| (250 * i, field("Quinn for max is quick"))));
        let steps = run(&mut follow, &readings);
        assert_eq!(steps, [(1250, FollowStep { write: Some(FollowWrite::Store("quinn for max is quick".into())), end: None })]);
    }

    #[test]
    fn text_typed_after_the_take_is_not_a_change_of_it() {
        let mut follow = EditFollow::new(take(TAKE), FollowTimes::default()).expect("followed");
        let mut readings = vec![(0, field(TAKE))];
        readings.extend((1..=6).map(|i| (250 * i, field(&format!("{TAKE}，你觉得呢")))));
        assert!(run(&mut follow, &readings).is_empty());
    }

    #[test]
    fn a_rewrite_of_more_than_half_of_the_take_is_not_stored() {
        let mut follow = EditFollow::new(take("quinn for max is fast"), FollowTimes::default()).expect("followed");
        let mut readings = vec![(0, field("quinn for max is fast"))];
        // Still the take's place (two of its words kept), but three of its five words gone.
        readings.extend((1..=6).map(|i| (250 * i, field("quinn for Max runs very well"))));
        assert!(run(&mut follow, &readings).is_empty());
    }

    #[test]
    fn the_focus_leaving_the_field_ends_the_follow_and_keeps_a_correction_that_stood() {
        let mut follow = EditFollow::new(take(TAKE), FollowTimes::default()).expect("followed");
        let steps = run(&mut follow, &[(0, field(TAKE)), (250, field(FIXED)), (750, field(FIXED)), (1000, Err(UnconfirmedWhy::ControlChanged))]);
        assert_eq!(steps, [(1000, FollowStep { write: Some(FollowWrite::Store(FIXED.into())), end: Some(FollowEnd::FocusMoved) })]);
        let mut follow = EditFollow::new(take(TAKE), FollowTimes::default()).expect("followed");
        let steps = run(&mut follow, &[(0, field(TAKE)), (250, Err(UnconfirmedWhy::Timeout))]);
        assert_eq!(steps, [(250, FollowStep { write: None, end: Some(FollowEnd::Unreadable) })]);
    }

    #[test]
    fn a_first_reading_without_the_take_ends_at_once() {
        let mut follow = EditFollow::new(take(TAKE), FollowTimes::default()).expect("followed");
        assert_eq!(follow.on_reading(ms(0), field("完全不同的一句话")), FollowStep { write: None, end: Some(FollowEnd::NotFound) });
    }

    #[test]
    fn the_follow_ends_after_two_quiet_minutes_or_ten_minutes_in_all() {
        let mut follow = EditFollow::new(take(TAKE), FollowTimes::default()).expect("followed");
        assert_eq!(follow.on_reading(ms(0), field(TAKE)), FollowStep::default());
        assert_eq!(follow.on_reading(ms(500), field(TAKE)), FollowStep::default());
        assert_eq!(follow.on_reading(ms(120_000), field(TAKE)).end, Some(FollowEnd::Idle));
        let mut follow = EditFollow::new(take(TAKE), FollowTimes::default()).expect("followed");
        assert_eq!(follow.on_reading(ms(0), field(TAKE)), FollowStep::default());
        let mut at = 0;
        let mut text = FIXED;
        while at < 600_000 {
            at += 100_000;
            text = if text == FIXED { TAKE } else { FIXED };
            let step = follow.on_reading(ms(at), field(text));
            if at >= 600_000 {
                assert_eq!(step.end, Some(FollowEnd::Expired));
            } else {
                assert_eq!(step.end, None, "at {at}");
            }
        }
    }

    #[test]
    fn readings_come_faster_right_after_a_change() {
        let mut follow = EditFollow::new(take(TAKE), FollowTimes::default()).expect("followed");
        follow.on_reading(ms(0), field(TAKE));
        follow.on_reading(ms(500), field(FIXED));
        assert_eq!(follow.next_in(ms(500)), FOLLOW_FAST);
        assert_eq!(follow.next_in(ms(2600)), FOLLOW_EVERY);
    }

    /// A field's paragraph of `text`, starting at `at` in the control.
    fn line(text: &str, at: usize) -> Result<FocusContext, UnconfirmedWhy> {
        Ok(FocusContext { paragraph_at: Some(at), ..FocusContext::from_parts(text, "", "").expect("a context") })
    }

    /// Regression (review of the branch, 2026-11-27): with the take on the first line of a list and
    /// the next line reading alike, the caret moved down one line was taken for an edit of the take
    /// and stored a second later (「周一 → 周二」). A paragraph that starts elsewhere and reads
    /// otherwise is another paragraph: the take is not in it.
    #[test]
    fn regression_the_caret_on_another_line_that_reads_alike_is_not_an_edit() {
        let first = "周一下午三点开会";
        let mut follow = EditFollow::new(take(first), FollowTimes::default()).expect("followed");
        let mut readings = vec![(0, line(first, 0))];
        readings.extend((1..=8).map(|i| (250 * i, line("周二下午三点开会", 9))));
        let steps = run(&mut follow, &readings);
        assert_eq!(steps, [(500, FollowStep { write: None, end: Some(FollowEnd::Gone) })]);
    }

    /// Text typed or deleted before the take's paragraph moves its start, not its text: still the
    /// take, and an edit of it afterwards is stored.
    #[test]
    fn a_paragraph_moved_by_text_before_it_is_still_the_takes() {
        let first = "周一下午三点开会";
        let mut follow = EditFollow::new(take(first), FollowTimes::default()).expect("followed");
        let mut readings = vec![(0, line(first, 0)), (250, line(first, 6))];
        readings.extend((2..=7).map(|i| (250 * i, line("周一下午四点开会", 6))));
        let steps = run(&mut follow, &readings);
        assert_eq!(steps, [(1500, FollowStep { write: Some(FollowWrite::Store("周一下午四点开会".into())), end: None })]);
    }

    /// Regression (review of the branch, 2026-11-27): the caret far enough after the take that the
    /// 200 characters before it cut the take's start, the visible rest was stored as the corrected
    /// take, in place of the whole one. A take the reading cuts at its window says nothing.
    #[test]
    fn regression_a_take_cut_by_the_reading_window_is_not_stored_cut() {
        let mut follow = EditFollow::new(take(TAKE), FollowTimes::default()).expect("followed");
        let mut readings = vec![(0, field(TAKE))];
        readings.extend((1..=5).map(|i| (250 * i, field(FIXED))));
        // The caret 200 characters on: the reading starts inside the corrected take.
        let tail: String = FIXED.chars().skip(3).collect();
        let typed = "好".repeat(CONTEXT_CHARS - tail.chars().count() - 1);
        readings.extend((6..=12).map(|i| (250 * i, field(&format!("{tail}，{typed}")))));
        let steps = run(&mut follow, &readings);
        assert_eq!(steps, [(1250, FollowStep { write: Some(FollowWrite::Store(FIXED.into())), end: None })], "stored whole, once");
    }

    /// Regression (review of the branch, 2026-11-27): the correction taken back together with
    /// another edit (a word deleted too) left the stored correction in place, though the field no
    /// longer held it. With no learnable change left, the follow's correction goes.
    #[test]
    fn regression_a_correction_taken_back_with_another_edit_clears_what_was_stored() {
        let mut follow = EditFollow::new(take(TAKE), FollowTimes::default()).expect("followed");
        let mut readings = vec![(0, field(TAKE))];
        readings.extend((1..=5).map(|i| (250 * i, field(FIXED))));
        readings.extend((6..=11).map(|i| (250 * i, field("今天测试千问四Max的推理能力"))));
        let steps = run(&mut follow, &readings);
        assert_eq!(
            steps,
            [
                (1250, FollowStep { write: Some(FollowWrite::Store(FIXED.into())), end: None }),
                (2500, FollowStep { write: Some(FollowWrite::Clear), end: None })
            ]
        );
    }

    /// Regression (review of the branch, 2026-11-27): a short take whose Chinese words became Latin
    /// ones fell under the similarity a take must have to be chosen among fifty (「用维他命录音」 →
    /// 「用 Voltip 录音」, 3 / 9): the follow lost it and stored nothing. The take followed is
    /// known; it is found again with less.
    #[test]
    fn regression_a_short_take_changed_into_latin_words_is_still_followed() {
        let mut follow = EditFollow::new(take("用维他命录音"), FollowTimes::default()).expect("followed");
        let mut readings = vec![(0, field("用维他命录音"))];
        readings.extend((1..=5).map(|i| (250 * i, field("用 Voltip 录音"))));
        let steps = run(&mut follow, &readings);
        assert_eq!(steps, [(1250, FollowStep { write: Some(FollowWrite::Store("用 Voltip 录音".into())), end: None })]);
    }

    /// Readings every 250 ms from 0: each text of `steps` read as many times as it says.
    fn every_quarter(steps: &[(String, usize)]) -> Vec<(u64, Result<FocusContext, UnconfirmedWhy>)> {
        steps.iter().flat_map(|(text, n)| std::iter::repeat_n(text.clone(), *n)).enumerate().map(|(i, text)| (250 * i as u64, field(&text))).collect()
    }

    /// `word` typed key by key between `prefix` and `suffix`, one reading a key, then read `hold`
    /// more times.
    fn typed(prefix: &str, word: &str, suffix: &str, hold: usize) -> Vec<(String, usize)> {
        let chars: Vec<char> = word.chars().collect();
        let mut out: Vec<(String, usize)> = (1..chars.len()).map(|k| (format!("{prefix}{}{suffix}", chars[..k].iter().collect::<String>()), 1)).collect();
        out.push((format!("{prefix}{word}{suffix}"), hold + 1));
        out
    }

    fn once(text: &str) -> (String, usize) {
        (text.to_owned(), 1)
    }

    /// What the steps stored, in order.
    fn stores(steps: &[(u64, FollowStep)]) -> Vec<String> {
        steps
            .iter()
            .filter_map(|(_, step)| match &step.write {
                Some(FollowWrite::Store(text)) => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    /// Regression (review of #271, 2026-11-27): the take's first word retyped key by key. A reading
    /// half typed (「Q的推理能力很强。」 for 千问四) did not pass the match's rule for a change at an
    /// end of the take, the take's span shrank to the rest and never grew back, and nothing was
    /// stored (the reviewer's port: no write, the spot stayed 「的推理能力很强。」).
    #[test]
    fn regression_the_takes_first_word_retyped_key_by_key_is_stored() {
        let take_text = "千问四的推理能力很强。";
        let mut steps = vec![once(take_text)];
        steps.extend(typed("", "Qwen 4", "的推理能力很强。", 6));
        let mut follow = EditFollow::new(take(take_text), FollowTimes::default()).expect("followed");
        assert_eq!(stores(&run(&mut follow, &every_quarter(&steps))), ["Qwen 4的推理能力很强。"]);
        // Deleted with Backspace first, then typed.
        let mut steps = vec![once(take_text), once("问四的推理能力很强。"), once("四的推理能力很强。"), once("的推理能力很强。")];
        steps.extend(typed("", "Qwen 4", "的推理能力很强。", 6));
        let mut follow = EditFollow::new(take(take_text), FollowTimes::default()).expect("followed");
        assert_eq!(stores(&run(&mut follow, &every_quarter(&steps))), ["Qwen 4的推理能力很强。"]);
    }

    /// Regression (review of #271, 2026-11-27): the take's last word deleted with Backspace and
    /// typed again stored nothing; once the word was gone, what the user typed after the take's
    /// span was taken for text typed after the take.
    #[test]
    fn regression_the_takes_last_word_deleted_and_typed_again_is_stored() {
        let mut steps = vec![once("今天测试一下千问四"), once("今天测试一下千问"), once("今天测试一下千"), once("今天测试一下")];
        steps.extend(typed("今天测试一下", "Qwen 4", "", 6));
        let mut follow = EditFollow::new(take("今天测试一下千问四"), FollowTimes::default()).expect("followed");
        assert_eq!(stores(&run(&mut follow, &every_quarter(&steps))), ["今天测试一下Qwen 4"]);
        let steps = [once("我们明天下午开会"), once("我们明天下午开"), once("我们明天下午"), once("我们明天下午聚"), ("我们明天下午聚餐".to_owned(), 6)];
        let mut follow = EditFollow::new(take("我们明天下午开会"), FollowTimes::default()).expect("followed");
        assert_eq!(stores(&run(&mut follow, &every_quarter(&steps))), ["我们明天下午聚餐"]);
    }

    /// Regression (review of #271, 2026-11-27): 三 → 四 stored, then 开会 deleted and 见面 typed: the
    /// take cut at 会议室 was stored over the right correction while the field read 「…见面」.
    #[test]
    fn regression_a_last_word_typed_again_does_not_store_the_take_cut_short() {
        let steps = [
            once("今天下午三点在会议室开会"),
            ("今天下午四点在会议室开会".to_owned(), 5),
            once("今天下午四点在会议室开"),
            once("今天下午四点在会议室"),
            once("今天下午四点在会议室见"),
            ("今天下午四点在会议室见面".to_owned(), 6),
        ];
        let mut follow = EditFollow::new(take("今天下午三点在会议室开会"), FollowTimes::default()).expect("followed");
        assert_eq!(stores(&run(&mut follow, &every_quarter(&steps))), ["今天下午四点在会议室开会", "今天下午四点在会议室见面"]);
    }

    /// Regression (review of #271, 2026-11-27): Chinese words replaced by one Latin word fell out of
    /// the take's span (five words against one, past the match's gap rule) and were read as
    /// deleted: with 千问 → Qwen in the same take, 「我们今天用Qwen部署到」 was stored.
    #[test]
    fn regression_words_replaced_by_one_latin_word_are_not_read_as_deleted() {
        for (take_text, fixed) in [
            ("我们今天用千问部署到库伯内特斯", "我们今天用Qwen部署到Kubernetes"),
            ("帮我把代码部署到库伯内特斯集群上", "帮我把代码部署到Kubernetes集群上"),
            ("现在打开维他命", "现在打开Voltip"),
        ] {
            let mut follow = EditFollow::new(take(take_text), FollowTimes::default()).expect("followed");
            assert_eq!(stores(&run(&mut follow, &every_quarter(&[once(take_text), (fixed.to_owned(), 6)]))), [fixed], "{take_text}");
        }
    }

    /// Regression (review of #271, 2026-11-27): the reading's window starting inside a Latin word of
    /// the take (「etes集群里…」) or inside the corrected word (「wen 4 Max…」) was not seen as a cut:
    /// the visible rest was stored as the take, or the correction cleared.
    #[test]
    fn regression_a_window_starting_inside_a_word_of_the_take_is_a_cut() {
        for (take_text, fixed) in [
            ("Kubernetes集群里部署千问四模型", "Kubernetes集群里部署Qwen 4模型"),
            ("今天测试一下千问四Max的推理能力然后再看看它写代码的能力怎么样", "今天测试一下Qwen 4 Max的推理能力然后再看看它写代码的能力怎么样"),
        ] {
            for skip in 1..14 {
                let tail: String = fixed.chars().skip(skip).collect();
                let filler = "好".repeat(CONTEXT_CHARS - tail.chars().count() - 1);
                let steps = [once(take_text), (fixed.to_owned(), 5), (format!("{tail}，{filler}"), 7)];
                let mut follow = EditFollow::new(take(take_text), FollowTimes::default()).expect("followed");
                let steps = run(&mut follow, &every_quarter(&steps));
                assert_eq!(steps, [(1250, FollowStep { write: Some(FollowWrite::Store(fixed.into())), end: None })], "{take_text}, window from {skip}");
            }
        }
    }

    /// The far side: the caret before the take, the window after it ending inside the take's last
    /// word.
    #[test]
    fn regression_a_window_ending_inside_a_word_of_the_take_is_a_cut() {
        let (take_text, fixed) = ("把千问四模型部署到Kubernetes", "把Qwen 4模型部署到Kubernetes");
        for cut in 1..10 {
            let head: String = fixed.chars().take(fixed.chars().count() - cut).collect();
            let filler = "好".repeat(CONTEXT_CHARS - head.chars().count() - 1);
            let mut readings = vec![(0, field(take_text))];
            readings.extend((1..=5).map(|i| (250 * i, field(fixed))));
            readings.extend((6..=12).map(|i| (250 * i, FocusContext::from_parts("", "", &format!("{filler}，{head}")).ok_or(UnconfirmedWhy::Unreadable))));
            let mut follow = EditFollow::new(take(take_text), FollowTimes::default()).expect("followed");
            assert_eq!(run(&mut follow, &readings), [(1250, FollowStep { write: Some(FollowWrite::Store(fixed.into())), end: None })], "cut {cut}");
        }
    }

    /// Regression (found while checking the review, 2026-11-28): text typed after the corrected take
    /// that holds the take's replaced characters in order (问 and 四 of 千问四 in 「，我问了四个问题」)
    /// was matched to them and stored as part of the take: 「今天测试一下Qwen 4，我问了四」.
    #[test]
    fn regression_text_typed_after_the_take_is_not_matched_to_its_replaced_words() {
        let mut steps = vec![once("今天测试一下千问四"), ("今天测试一下Qwen 4".to_owned(), 6)];
        let typed: Vec<char> = "，我问了四个问题".chars().collect();
        steps.extend((1..=typed.len()).map(|k| once(&format!("今天测试一下Qwen 4{}", typed[..k].iter().collect::<String>()))));
        steps.push(("今天测试一下Qwen 4，我问了四个问题".to_owned(), 6));
        let mut follow = EditFollow::new(take("今天测试一下千问四"), FollowTimes::default()).expect("followed");
        assert_eq!(stores(&run(&mut follow, &every_quarter(&steps))), ["今天测试一下Qwen 4"]);
        // Corrected and typed on in one reading.
        let mut follow = EditFollow::new(take("今天测试一下千问四"), FollowTimes::default()).expect("followed");
        let steps = [once("今天测试一下千问四"), ("今天测试一下Qwen 4，我问了四个问题".to_owned(), 6)];
        assert_eq!(stores(&run(&mut follow, &every_quarter(&steps))), ["今天测试一下Qwen 4"]);
    }

    /// A reading with the caret between `before` and `after`.
    fn caret(before: &str, after: &str) -> Result<FocusContext, UnconfirmedWhy> {
        FocusContext::from_parts(before, "", after).ok_or(UnconfirmedWhy::Unreadable)
    }

    /// Readings every 250 ms from 0, each as many times as it says.
    fn quarters(steps: Vec<(Result<FocusContext, UnconfirmedWhy>, usize)>) -> Vec<(u64, Result<FocusContext, UnconfirmedWhy>)> {
        steps.into_iter().flat_map(|(reading, n)| std::iter::repeat_n(reading, n)).enumerate().map(|(i, reading)| (250 * i as u64, reading)).collect()
    }

    /// Regression (second review of #271, 2026-11-28): text typed right before a corrected take
    /// was made part of it. The take's first character matched the same character typed before
    /// it (「我觉得」 before 「我们…」), and letters typed before a Latin word were glued to it in a
    /// reading in between (「SQwen」); a full stop typed after the last word joined it
    /// (「model.」).
    #[test]
    fn regression_text_typed_before_or_after_a_corrected_take_is_not_part_of_it() {
        let mut follow = EditFollow::new(take("我门明天开会"), FollowTimes::default()).expect("followed");
        let steps = quarters(vec![
            (field("我门明天开会"), 1),
            (field("我们明天开会"), 6),
            (caret("我", "我们明天开会"), 1),
            (caret("我觉", "我们明天开会"), 1),
            (caret("我觉得", "我们明天开会"), 7),
        ]);
        assert_eq!(stores(&run(&mut follow, &steps)), ["我们明天开会"]);
        let mut follow = EditFollow::new(take("quinn is fast"), FollowTimes::default()).expect("followed");
        let steps = quarters(vec![
            (field("quinn is fast"), 1),
            (field("Qwen is fast"), 6),
            (caret("S", "Qwen is fast"), 1),
            (caret("So", "Qwen is fast"), 1),
            (caret("So ", "Qwen is fast"), 7),
        ]);
        assert_eq!(stores(&run(&mut follow, &steps)), ["Qwen is fast"]);
        let mut follow = EditFollow::new(take("please send the quinn model"), FollowTimes::default()).expect("followed");
        let steps =
            every_quarter(&[once("please send the quinn model"), ("please send the Qwen model".to_owned(), 6), ("please send the Qwen model.".to_owned(), 7)]);
        assert_eq!(stores(&run(&mut follow, &steps)), ["please send the Qwen model"]);
    }

    /// Regression (second review of #271, 2026-11-28): a replacement of the take's first or last
    /// word with an apostrophe, a thousands comma or book-title marks in it was cut at the mark:
    /// 「t forget the meeting」, 「价格一共是1」, or nothing for 「《三体》」.
    #[test]
    fn regression_marks_inside_a_replaced_first_or_last_word_stay_in_it() {
        for (take_text, fixed) in [
            ("dont forget the meeting", "don't forget the meeting"),
            ("see you tomorrow im", "see you tomorrow I'm"),
            ("价格一共是一千", "价格一共是1,000"),
            ("我最近在看三提", "我最近在看《三体》"),
        ] {
            let mut follow = EditFollow::new(take(take_text), FollowTimes::default()).expect("followed");
            assert_eq!(stores(&run(&mut follow, &every_quarter(&[once(take_text), (fixed.to_owned(), 6)]))), [fixed], "{take_text}");
        }
    }

    /// Regression (second review of #271, 2026-11-28): a stored correction of the take's last word
    /// changed when the user typed on right after it (「通义」 became 「通义模」, 「now」 became
    /// 「now if you」). What was written stays the take's; text typed next to it later does not
    /// join it.
    #[test]
    fn regression_text_typed_after_a_stored_last_word_correction_does_not_change_it() {
        let mut follow = EditFollow::new(take("今天测试一下千问四"), FollowTimes::default()).expect("followed");
        let steps = every_quarter(&[once("今天测试一下千问四"), ("今天测试一下通义".to_owned(), 6), ("今天测试一下通义模型".to_owned(), 6)]);
        assert_eq!(stores(&run(&mut follow, &steps)), ["今天测试一下通义"]);
        let mut follow = EditFollow::new(take("we can meet tomorrow"), FollowTimes::default()).expect("followed");
        let mut steps = vec![once("we can meet tomorrow"), ("we can meet now".to_owned(), 6)];
        steps.extend(typed("we can meet now", " if you want", "", 6));
        assert_eq!(stores(&run(&mut follow, &every_quarter(&steps))), ["we can meet now"]);
    }

    /// Regression (second review of #271, 2026-11-28): the last word deleted with Backspace and
    /// replaced by a much shorter one (「一百二十三」 → 「123」, 「库伯内特斯」 → 「K8s」) was never
    /// decided: every reading after it was taken for a word half typed, and nothing was stored.
    #[test]
    fn regression_a_deleted_last_word_replaced_by_a_short_one_is_stored() {
        for (take_text, kept, typed_word) in [("用户总数是一百二十三", "用户总数是", "123"), ("帮我部署到库伯内特斯", "帮我部署到", "K8s")]
        {
            let chars: Vec<char> = take_text.chars().collect();
            let mut steps = vec![once(take_text)];
            steps.extend((kept.chars().count()..chars.len()).rev().map(|n| once(&chars[..n].iter().collect::<String>())));
            steps.extend(typed(kept, typed_word, "", 6));
            let mut follow = EditFollow::new(take(take_text), FollowTimes::default()).expect("followed");
            assert_eq!(stores(&run(&mut follow, &every_quarter(&steps))), [format!("{kept}{typed_word}")], "{take_text}");
        }
    }

    /// Regression (second review of #271, 2026-11-28): readings of a word half typed did not break
    /// the second a candidate must stand: the take cut short, read again after them, was stored at
    /// once over the right correction.
    #[test]
    fn regression_readings_of_a_word_half_typed_break_the_candidates_second() {
        let p = "明天四点把代码部署到";
        let mut steps = vec![once("明天三点把代码部署到库伯内特斯"), ("明天四点把代码部署到库伯内特斯".to_owned(), 6)];
        steps.extend(["明天四点把代码部署到库伯内特", "明天四点把代码部署到库伯内", "明天四点把代码部署到库伯", "明天四点把代码部署到库"].map(once));
        steps.extend(
            [p.to_owned(), format!("{p}K"), format!("{p}Ku"), format!("{p}Kub"), format!("{p}Ku"), format!("{p}K"), p.to_owned()].iter().map(|t| once(t)),
        );
        let mut follow = EditFollow::new(take("明天三点把代码部署到库伯内特斯"), FollowTimes::default()).expect("followed");
        assert_eq!(stores(&run(&mut follow, &every_quarter(&steps))), ["明天四点把代码部署到库伯内特斯"]);
    }

    /// Regression (second review of #271, 2026-11-28): the field's whole text replaced in one
    /// reading (another chat's draft in the same input box, select all and type) was taken for
    /// the take rewritten, and the follow went on reading the field for two minutes. No word of
    /// the take left: it is gone.
    #[test]
    fn regression_a_field_whose_text_was_replaced_whole_ends_the_follow() {
        let mut follow = EditFollow::new(take("今天下午三点在会议室开会"), FollowTimes::default()).expect("followed");
        let steps = run(
            &mut follow,
            &[(0, line("今天下午三点在会议室开会", 0)), (500, line("周五之前把报告发给我，谢谢", 0)), (1000, line("周五之前把报告发给我，谢谢", 0))],
        );
        assert_eq!(steps, [(1000, FollowStep { write: None, end: Some(FollowEnd::Gone) })]);
    }

    /// Regression (third review of #271, 2026-11-28): the field's text replaced at once by another
    /// that shares some of the take's words (another chat's draft in the same box) was taken for
    /// the take edited and stored as its correction. More than half of the take's words replaced
    /// in one reading, with the whole paragraph in view, is other text.
    #[test]
    fn regression_other_text_sharing_some_words_of_the_take_is_not_its_correction() {
        let (take_text, other) = ("报告有有会议室要了都测试", "说吃饭说酒店项目都会议室快递都会我会议室测试也");
        let mut follow = EditFollow::new(take(take_text), FollowTimes::default()).expect("followed");
        let steps = run(&mut follow, &every_quarter(&[once(take_text), (other.to_owned(), 12)]));
        assert!(stores(&steps).is_empty(), "{steps:?}");
        assert_eq!(steps.last().map(|(_, step)| step.end), Some(Some(FollowEnd::Gone)), "{steps:?}");
    }

    /// Regression (second review of #271, 2026-11-28): the take's final mark deleted, the next
    /// character typed was taken for its replacement (「…很强而」 stored); the last word and the
    /// mark deleted and typed again lost the mark (「…见面」 for 「…见面。」). With the mark deleted
    /// and the sentence typed on, the take ends where its words do.
    #[test]
    fn regression_the_takes_final_mark_is_not_a_word_to_replace() {
        let mut follow = EditFollow::new(take("千问四的推理能力很强。"), FollowTimes::default()).expect("followed");
        let mut steps = vec![once("千问四的推理能力很强。"), ("Qwen 4的推理能力很强。".to_owned(), 6), once("Qwen 4的推理能力很强")];
        steps.extend(typed("Qwen 4的推理能力很强", "而且速度很快", "", 6));
        assert_eq!(stores(&run(&mut follow, &every_quarter(&steps))), ["Qwen 4的推理能力很强。", "Qwen 4的推理能力很强"]);
        let mut follow = EditFollow::new(take("我们明天下午三点开会。"), FollowTimes::default()).expect("followed");
        let steps = [
            once("我们明天下午三点开会。"),
            once("我们明天下午三点开会"),
            once("我们明天下午三点开"),
            once("我们明天下午三点"),
            once("我们明天下午三点见"),
            once("我们明天下午三点见面"),
            ("我们明天下午三点见面。".to_owned(), 7),
        ];
        assert_eq!(stores(&run(&mut follow, &every_quarter(&steps))), ["我们明天下午三点见面。"]);
    }

    /// Regression (second review of #271, 2026-11-28): text pasted before a take that stood alone
    /// in the field filled the reading's window, and every reading after it was taken for a cut.
    /// The window's edge in other text is not inside the take.
    #[test]
    fn regression_a_long_paste_before_the_take_does_not_cut_every_reading() {
        let quote = "引".repeat(223);
        let mut follow = EditFollow::new(take("千问四的推理能力很强"), FollowTimes::default()).expect("followed");
        let steps = quarters(vec![(field("千问四的推理能力很强"), 1), (caret(&quote, "千问四的推理能力很强"), 4), (caret(&quote, "Qwen 4的推理能力很强"), 12)]);
        assert_eq!(stores(&run(&mut follow, &steps)), ["Qwen 4的推理能力很强"]);
    }

    /// Regression (second review of #271, 2026-11-28): the window's start, moving into the take,
    /// matched a character of the text before it by chance (「的」 of 「我说的」), the cut went
    /// unseen and the correction was cleared. A cut needs some of the text before the take to be
    /// in view to be ruled out.
    #[test]
    fn regression_a_window_edge_matched_by_chance_to_text_before_the_take_is_a_cut() {
        let take_text = "千问四的推理能力很强然后再看看它写代码的能力怎么样";
        let fixed = "Qwen 4的推理能力很强然后再看看它写代码的能力怎么样";
        let filler: String = "这是后面接着写的一段很长的文字".chars().cycle().take(177).collect();
        let mut steps = vec![(format!("我说的{take_text}"), 1), (format!("我说的{fixed}"), 6)];
        let chars: Vec<char> = filler.chars().collect();
        steps.extend((1..=chars.len()).map(|k| (format!("我说的{fixed}，{}", chars[..k].iter().collect::<String>()), 1)));
        steps.push((format!("我说的{fixed}，{filler}"), 6));
        let mut follow = EditFollow::new(take(take_text), FollowTimes::default()).expect("followed");
        let steps = run(&mut follow, &every_quarter(&steps));
        assert_eq!(stores(&steps), [fixed]);
        assert!(!steps.iter().any(|(_, step)| step.write == Some(FollowWrite::Clear)), "{steps:?}");
    }

    /// `take_text` with everything after `kept` deleted key by key, then `word` typed key by key.
    fn deleted_then_typed(take_text: &str, kept: &str, word: &str, hold: usize) -> Vec<(String, usize)> {
        let chars: Vec<char> = take_text.chars().collect();
        let mut steps = vec![once(take_text)];
        steps.extend((kept.chars().count()..chars.len()).rev().map(|n| once(&chars[..n].iter().collect::<String>())));
        steps.extend(typed(kept, word, "", hold));
        steps
    }

    /// Regression (third review of #271, 2026-11-28): a last word typed key by key after Backspace
    /// was cut at a mark that belongs to it: 「I'」, 「1,」, 「《三体」 were stored.
    #[test]
    fn regression_a_mark_inside_a_last_word_typed_key_by_key_stays_in_it() {
        for (take_text, kept, word) in [
            ("see you tomorrow im", "see you tomorrow ", "I'm"),
            ("价格一共是一千", "价格一共是", "1,000"),
            ("我最近在看三提", "我最近在看", "《三体》"),
            ("we met the boy", "we met the ", "boys'"),
            // The take's final full stop is in its last word's token: typed again, it is the take's.
            ("we met the boy.", "we met the ", "boys'."),
        ] {
            let mut follow = EditFollow::new(take(take_text), FollowTimes::default()).expect("followed");
            assert_eq!(stores(&run(&mut follow, &every_quarter(&deleted_then_typed(take_text, kept, word, 6)))), [format!("{kept}{word}")], "{take_text}");
        }
        // At the start: the first word deleted with Backspace (the caret after it), the new one typed.
        let (take_text, rest, word) = ("三提是一本好书", "是一本好书", "《三体》");
        let mut steps = vec![(field(take_text), 1), (caret("三", rest), 1), (caret("", rest), 1)];
        let chars: Vec<char> = word.chars().collect();
        steps.extend((1..=chars.len()).map(|k| (caret(&chars[..k].iter().collect::<String>(), rest), if k == chars.len() { 7 } else { 1 })));
        let mut follow = EditFollow::new(take(take_text), FollowTimes::default()).expect("followed");
        assert_eq!(stores(&run(&mut follow, &quarters(steps))), [format!("{word}{rest}")]);
    }

    /// Regression (third review of #271, 2026-11-28): Backspace from the end deletes the take's
    /// final 「。」 with its last word; counted in the missing words, it made a one-character
    /// replacement too short (never stored, the follow stuck) and a Latin one lose its last part
    /// (「今天测试一下Qwen」).
    #[test]
    fn regression_the_last_word_and_final_mark_deleted_and_typed_again_is_stored() {
        let mut follow = EditFollow::new(take("我们明天开会。"), FollowTimes::default()).expect("followed");
        let steps = every_quarter(&[once("我们明天开会。"), once("我们明天开会"), once("我们明天开"), once("我们明天开完"), ("我们明天开完。".to_owned(), 8)]);
        assert_eq!(stores(&run(&mut follow, &steps)), ["我们明天开完。"]);
        let mut follow = EditFollow::new(take("今天测试一下千问四。"), FollowTimes::default()).expect("followed");
        assert_eq!(
            stores(&run(&mut follow, &every_quarter(&deleted_then_typed("今天测试一下千问四。", "今天测试一下", "Qwen 4。", 7)))),
            ["今天测试一下Qwen 4。"]
        );
    }

    /// Regression (third review of #271, 2026-11-28): text inserted next to the take's last word,
    /// then the word deleted and typed again: the inserted text was taken for the word's
    /// replacement (「千问四 → 最新的」).
    #[test]
    fn regression_text_inserted_next_to_a_last_word_is_not_its_replacement() {
        let mut steps = vec![(field("我们用的是千问四"), 1), (caret("我们用的是最新的", "千问四"), 8)];
        for text in ["我们用的是最新的千问四", "我们用的是最新的千问", "我们用的是最新的千", "我们用的是最新的"] {
            steps.push((field(text), 1));
        }
        for k in 1..6 {
            steps.push((field(&format!("我们用的是最新的{}", &"Qwen 4"[..k])), 1));
        }
        steps.push((field("我们用的是最新的Qwen 4"), 8));
        let mut follow = EditFollow::new(take("我们用的是千问四"), FollowTimes::default()).expect("followed");
        assert_eq!(stores(&run(&mut follow, &quarters(steps))), ["我们用的是最新的Qwen 4"]);
    }

    /// Regression (third review of #271, 2026-11-28): the full stop of an abbreviation typed as the
    /// last word was cut with the one typed after the take (「U.S」, 「p.m」).
    #[test]
    fn regression_a_full_stop_of_an_abbreviation_typed_as_the_last_word_is_kept() {
        for (take_text, fixed) in [("we moved to the us", "we moved to the U.S."), ("see you at five pm", "see you at five p.m.")] {
            let mut follow = EditFollow::new(take(take_text), FollowTimes::default()).expect("followed");
            assert_eq!(stores(&run(&mut follow, &every_quarter(&[once(take_text), (fixed.to_owned(), 8)]))), [fixed], "{take_text}");
        }
    }

    /// Regression (third review of #271, 2026-11-28): the space starting the text after the take was
    /// stored as the end of the corrected take (「we use Qwen 」).
    #[test]
    fn regression_a_space_that_starts_the_text_after_the_take_is_not_stored_with_it() {
        let mut follow = EditFollow::new(take("we use quinn"), FollowTimes::default()).expect("followed");
        let steps = quarters(vec![(caret("we use quinn", " for coding"), 1), (caret("we use Qwen", " for coding"), 8)]);
        assert_eq!(stores(&run(&mut follow, &steps)), ["we use Qwen"]);
    }

    /// Regression (third review of #271, 2026-11-28): text typed before a corrected take, read
    /// with the caret already at the end, was taken for part of the take: an insertion that could
    /// have been made in several places goes to the take's edge, not into it.
    #[test]
    fn regression_text_typed_before_a_corrected_take_read_with_the_caret_at_the_end_is_not_part_of_it() {
        let mut follow = EditFollow::new(take("我门明天开会"), FollowTimes::default()).expect("followed");
        let steps = quarters(vec![(field("我门明天开会"), 1), (field("我们明天开会"), 6), (field("我觉得我们明天开会"), 8)]);
        assert_eq!(stores(&run(&mut follow, &steps)), ["我们明天开会"]);
    }

    /// Regression (third review of #271, 2026-11-28): a short take sent, the next message typed in
    /// the emptied field was taken for its correction (「好的」 → 「好啊」 stored). A field read empty
    /// has lost the take.
    #[test]
    fn regression_the_next_message_typed_after_a_short_take_was_sent_is_not_its_correction() {
        let mut follow = EditFollow::new(take("好的"), FollowTimes::default()).expect("followed");
        let readings = [
            (0, field("好的")),
            (500, field("好的")),
            (1000, field("好的")),
            (1500, field("")),
            (2000, field("好啊")),
            (2250, field("好啊")),
            (2500, field("好啊")),
        ];
        assert_eq!(run(&mut follow, &readings), [(1500, FollowStep { write: None, end: Some(FollowEnd::Gone) })]);
    }

    /// Regression (third review of #271, 2026-11-28): the window's start inside a first word typed
    /// again was not a cut (the margin was measured to the take's kept words), and the visible
    /// rest of the word was stored as its replacement (「ubernetes集群…」).
    #[test]
    fn regression_a_window_starting_inside_a_first_word_typed_again_is_a_cut() {
        let (word, rest) = ("Kubernetes", "集群已经部署好了");
        let take_text = format!("{word}{rest}");
        let mut steps = vec![(field(&format!("好的，{take_text}")), 1)];
        for k in (0..word.len()).rev() {
            steps.push((caret(&format!("好的，{}", &word[..k]), rest), 1));
        }
        for k in 1..=word.len() {
            steps.push((caret(&format!("好的，{}", &word[..k]), rest), 1));
        }
        steps.push((caret(&format!("好的，{word}"), rest), 6));
        // The reply typed on after the take until the window's start lies inside the first word.
        let reply: String = "，接下来我们还需要检查一下监控和日志的配置，确认告警规则都已经生效".chars().cycle().take(183).collect();
        let chars: Vec<char> = reply.chars().collect();
        let mut typed = 0;
        while typed < chars.len() {
            typed = (typed + 3).min(chars.len());
            steps.push((field(&format!("好的，{take_text}{}", chars[..typed].iter().collect::<String>())), 1));
        }
        steps.push((field(&format!("好的，{take_text}{reply}")), 6));
        let mut follow = EditFollow::new(take(&take_text), FollowTimes::default()).expect("followed");
        assert_eq!(stores(&run(&mut follow, &quarters(steps))), Vec::<String>::new());
    }

    /// Regression (third review of #271, 2026-11-28): a field standing still while a word at the
    /// take's end could not be told never reached the idle end: every such reading counted as a
    /// change.
    #[test]
    fn regression_a_field_that_stands_still_while_a_word_cannot_be_told_ends_idle() {
        let mut follow = EditFollow::new(take("我觉得可以"), FollowTimes::default()).expect("followed");
        let mut readings = vec![(0, field("我觉得可以")), (250, field("我觉得可")), (500, field("我觉得")), (750, field("我觉得x"))];
        readings.extend((4..=2400).map(|i| (250 * i, field("我觉得x"))));
        let steps = run(&mut follow, &readings);
        assert!(steps.last().is_some_and(|(at, s)| s.end == Some(FollowEnd::Idle) && *at <= 121_000), "{steps:?}");
    }

    /// Regression (third review of #271, 2026-11-28): a short take whose Chinese word became a
    /// longer Latin one fell under the similarity floor and ended (「打开微信」 → 「打开WeChat」).
    #[test]
    fn regression_a_short_take_whose_word_became_a_longer_latin_word_is_still_followed() {
        for (take_text, fixed) in [("打开微信", "打开WeChat"), ("we on one thousand", "we on 1,000"), ("it one thousand it", "it 1,000 it")] {
            let mut follow = EditFollow::new(take(take_text), FollowTimes::default()).expect("followed");
            assert_eq!(stores(&run(&mut follow, &every_quarter(&[once(take_text), (fixed.to_owned(), 8)]))), [fixed], "{take_text}");
        }
    }

    /// Regression (third review of #271, 2026-11-28): both ends replaced in one reading, the kept
    /// middle was dropped as a match by chance and the follow ended.
    #[test]
    fn regression_both_ends_of_a_take_replaced_in_one_reading_are_stored() {
        let t = "维他命部署成功了吗千问";
        let mut follow = EditFollow::new(take(t), FollowTimes::default()).expect("followed");
        assert_eq!(stores(&run(&mut follow, &every_quarter(&[once(t), ("Voltip部署成功了吗Qwen Max".to_owned(), 8)]))), ["Voltip部署成功了吗Qwen Max"]);
    }

    /// Regression (third review of #271, 2026-11-28): a take too long to leave the cut margin in
    /// the window was followed, and every reading was a cut. It is not followed.
    #[test]
    fn regression_a_take_too_long_for_the_cut_margin_is_not_followed() {
        assert!(EditFollow::new(take(&"字".repeat(CONTEXT_CHARS - CUT_MARGIN + 1)), FollowTimes::default()).is_none());
        let body: String = "我们今天讨论了很多关于模型部署的问题然后".chars().cycle().take(CONTEXT_CHARS - CUT_MARGIN - 4).collect();
        let (take_text, fixed) = (format!("{body}下午三点"), format!("{body}下午四点"));
        let mut follow = EditFollow::new(take(&take_text), FollowTimes::default()).expect("followed");
        assert_eq!(stores(&run(&mut follow, &every_quarter(&[once(&take_text), (fixed.clone(), 8)]))), [fixed]);
    }

    #[test]
    fn takes_the_paragraph_cannot_hold_are_not_followed() {
        assert!(EditFollow::new(take("第一行\n第二行"), FollowTimes::default()).is_none());
        assert!(EditFollow::new(take(&"字".repeat(CONTEXT_CHARS - CUT_MARGIN + 1)), FollowTimes::default()).is_none());
        assert!(EditFollow::new(take("   "), FollowTimes::default()).is_none());
        assert!(EditFollow::new(take(&"字".repeat(CONTEXT_CHARS - CUT_MARGIN)), FollowTimes::default()).is_some());
    }
}
