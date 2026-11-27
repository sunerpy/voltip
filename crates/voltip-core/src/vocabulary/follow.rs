//! 自动跟踪输入框里的修改 (docs/dictation.md §35.10), the state of following one take's field after
//! its paste: readings of the kept control in, what to store and when to stop out. No IO and no
//! clock: the runtime passes the time since the follow began, reads the field, writes the history
//! and releases the control (`crate::runtime::edit_follow`).
//!
//! The first reading places the take in its paragraph and keeps what the field shows of it (the
//! baseline: the application's own rewrites, such as Word's capital at the start of a sentence, are
//! in it and are not the user's). Every later reading carries the take's place over and rebuilds
//! the take with the user's changes only ([`follow_corrected`]). A rebuilt take is stored once it
//! has stood [`FOLLOW_STABLE`] (a half-typed word or an input method's pinyin keeps changing), when
//! it differs from the take and holds a change of the recogniser's wording ([`storable`]), and
//! changes at most [`FOLLOW_MAX_CHANGED`] of the take's words. Nothing here logs a text.

use std::time::Duration;

use super::learn::{FollowSpot, changed_share, follow_corrected, follow_first, follow_next, storable};
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
    /// take the paragraph read cannot hold whole: one with a line break, or longer than
    /// [`CONTEXT_CHARS`].
    pub fn new(take: HistoryText, times: FollowTimes) -> Option<Self> {
        let inserted = normalize(&take.text);
        if inserted.contains('\n') || inserted.chars().count() > CONTEXT_CHARS || inserted.trim().is_empty() {
            return None;
        }
        Some(Self {
            take,
            inserted,
            times,
            spot: None,
            baseline: String::new(),
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
        let moved = |spot: &FollowSpot| {
            matches!((self.paragraph_at, context.paragraph_at), (Some(was), Some(now)) if was != now) && spot.paragraph != previous.paragraph
        };
        // The reading's window cuts the take at its start or end: it cannot tell what the user left
        // of it, and says nothing.
        let cut = |spot: &FollowSpot| {
            (spot.span.0 == 0 && context.before.chars().count() >= CONTEXT_CHARS)
                || (spot.span.1 >= spot.paragraph.chars().count() && context.after.chars().count() >= CONTEXT_CHARS)
        };
        match follow_next(&previous, &context.before, &context.selected, &context.after, &self.take).filter(|spot| !moved(spot)) {
            Some(spot) if cut(&spot) => {}
            None => {
                self.missing += 1;
                if self.missing >= 2 {
                    return FollowStep::end(self.settle(self.times.gone_stable), FollowEnd::Gone);
                }
            }
            Some(spot) => {
                self.paragraph_at = context.paragraph_at;
                self.missing = 0;
                let rebuilt = follow_corrected(&self.inserted, &self.baseline, &spot.text()).unwrap_or_else(|| self.inserted.clone());
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
        assert_eq!(steps, [(1250, FollowStep { write: Some(FollowWrite::Store(FIXED.into())), end: Some(FollowEnd::Gone) })]);
    }

    #[test]
    fn a_take_sent_right_after_its_correction_ends_without_a_write() {
        let mut follow = EditFollow::new(take(TAKE), FollowTimes::default()).expect("followed");
        let steps = run(&mut follow, &[(0, field(TAKE)), (250, field(FIXED)), (500, field("")), (750, field(""))]);
        assert_eq!(steps, [(750, FollowStep { write: None, end: Some(FollowEnd::Gone) })]);
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

    #[test]
    fn takes_the_paragraph_cannot_hold_are_not_followed() {
        assert!(EditFollow::new(take("第一行\n第二行"), FollowTimes::default()).is_none());
        assert!(EditFollow::new(take(&"字".repeat(CONTEXT_CHARS + 1)), FollowTimes::default()).is_none());
        assert!(EditFollow::new(take("   "), FollowTimes::default()).is_none());
        assert!(EditFollow::new(take(&"字".repeat(CONTEXT_CHARS)), FollowTimes::default()).is_some());
    }
}
