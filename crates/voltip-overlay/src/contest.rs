//! When a keeper raises the pill over a window that covers it (docs/dictation.md §32): the
//! Windows keeper and the X11 one share these rules, kept apart from their system calls so that
//! they are tested on every platform.
//!
//! A window that comes to the front always gets the pill raised over it (the keepers do that
//! themselves). A window found over the pill after another window appeared or the stacking order
//! changed gets it too, except right after the pill was raised over that same window: on top
//! again within [`AGAIN_AFTER`], it is in one of the shell's bands above the pill's (the Start
//! menu, notifications) or a program that puts its window back on top each time, and raising
//! again would only start a contest. Later, the same window pinned on top again is a new event and
//! gets the pill raised over it once more. At most [`RAISES_PER_SECOND`] such raises happen in any
//! second.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// How soon the same window back over the pill counts as a contest rather than a new event.
pub const AGAIN_AFTER: Duration = Duration::from_millis(500);

/// How many raises over covering windows may happen in one second: a bound for programs that
/// put their windows back on top each time.
pub const RAISES_PER_SECOND: usize = 8;

/// The keeper's record of its raises over covering windows.
#[derive(Debug, Default)]
pub struct Contest {
    /// The window the pill was last raised over, and when.
    last: Option<(isize, Instant)>,
    /// The raises within the last second.
    recent: VecDeque<Instant>,
}

impl Contest {
    /// A window came to the front (the keeper raises the pill over it): a window that covers the
    /// pill from now on is a new event, even the one raised over last.
    pub fn front(&mut self) {
        self.last = None;
    }

    /// After a window appeared or the stacking order changed, `cover` covers the pill at `now`:
    /// whether to raise the pill over it.
    pub fn raise_over(&mut self, cover: isize, now: Instant) -> bool {
        if let Some((window, at)) = self.last
            && window == cover
            && now.saturating_duration_since(at) < AGAIN_AFTER
        {
            return false;
        }
        while self.recent.front().is_some_and(|&at| now.saturating_duration_since(at) >= Duration::from_secs(1)) {
            self.recent.pop_front();
        }
        if self.recent.len() >= RAISES_PER_SECOND {
            return false;
        }
        self.recent.push_back(now);
        self.last = Some((cover, now));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn the_same_window_back_on_top_at_once_is_a_contest_and_later_a_new_pin() {
        let mut contest = Contest::default();
        let t0 = Instant::now();
        assert!(contest.raise_over(1, t0), "a window pinned over the pill");
        assert!(!contest.raise_over(1, t0 + ms(20)), "the same window on top again at once: a higher band, or it puts itself back");
        assert!(contest.raise_over(1, t0 + AGAIN_AFTER + ms(20)), "the same window pinned on top again later");
    }

    #[test]
    fn another_window_over_the_pill_is_raised_over_at_once() {
        let mut contest = Contest::default();
        let t0 = Instant::now();
        assert!(contest.raise_over(1, t0));
        assert!(contest.raise_over(2, t0 + ms(10)), "another window over the pill");
        assert!(contest.raise_over(1, t0 + ms(20)), "the first window, covering after the other one");
    }

    #[test]
    fn a_window_coming_to_the_front_makes_the_next_cover_a_new_event() {
        let mut contest = Contest::default();
        let t0 = Instant::now();
        assert!(contest.raise_over(1, t0));
        contest.front();
        assert!(contest.raise_over(1, t0 + ms(10)), "activated since, the window may be raised over again at once");
    }

    /// Two programs that each put their window back on top whenever it is covered would keep
    /// raising the pill and each other: the keeper raises at most `RAISES_PER_SECOND` times a
    /// second, and goes on in the next one.
    #[test]
    fn raises_over_windows_that_take_turns_on_top_are_bounded_per_second() {
        let mut contest = Contest::default();
        let t0 = Instant::now();
        let raised = (0..1000u32).filter(|&i| contest.raise_over(1 + isize::from(i % 2 == 1), t0 + Duration::from_micros(u64::from(i) * 500))).count();
        assert_eq!(raised, RAISES_PER_SECOND, "within half a second");
        assert!(contest.raise_over(3, t0 + ms(1600)), "a second later the keeper raises again");
    }
}
