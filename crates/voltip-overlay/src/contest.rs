//! When a keeper raises the pill over a window that covers it (docs/dictation.md §32): the
//! Windows keeper and the X11 one share these rules, kept apart from their system calls so that
//! they are tested on every platform.
//!
//! A window that comes to the front always gets the pill raised over it (the keepers do that
//! themselves). A window found over the pill after another window appeared or the stacking order
//! changed gets it at once too, except right after the pill was raised over that same window: on
//! top again within [`AGAIN_AFTER`], it may be in one of the shell's bands above the pill's (the
//! Start menu, notifications) or a program that puts its window back on top each time, and raising
//! at once would only start a contest. The keeper then looks again once [`AGAIN_AFTER`] has passed
//! and raises the pill if a window still covers it: a window unpinned and pinned on top again
//! quickly has the pill back over it half a second later, and a program that keeps putting its
//! window back gets the pill raised over it at most once every [`AGAIN_AFTER`]. At most
//! [`RAISES_PER_SECOND`] raises over covering windows happen in any second.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// How soon the same window back over the pill is looked at again rather than raised over at once.
pub const AGAIN_AFTER: Duration = Duration::from_millis(500);

/// How many raises over covering windows may happen in one second: a bound for programs that
/// put their windows back on top each time.
pub const RAISES_PER_SECOND: usize = 8;

/// What the keeper does about a window that covers the pill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// Raise the pill over it now.
    Raise,
    /// The same window came back over the pill right after the pill was raised over it: look again
    /// after this long, and ask again if a window still covers the pill then.
    LookAgainIn(Duration),
    /// Leave it there: this second's raises are spent.
    Leave,
}

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

    /// After a window appeared, the stacking order changed or the keeper looked again, `cover`
    /// covers the pill at `now`: what to do about it.
    pub fn raise_over(&mut self, cover: isize, now: Instant) -> Answer {
        if let Some((window, at)) = self.last
            && window == cover
        {
            let since = now.saturating_duration_since(at);
            if since < AGAIN_AFTER {
                return Answer::LookAgainIn(AGAIN_AFTER - since);
            }
        }
        while self.recent.front().is_some_and(|&at| now.saturating_duration_since(at) >= Duration::from_secs(1)) {
            self.recent.pop_front();
        }
        if self.recent.len() >= RAISES_PER_SECOND {
            return Answer::Leave;
        }
        self.recent.push_back(now);
        self.last = Some((cover, now));
        Answer::Raise
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn the_same_window_back_on_top_at_once_is_looked_at_again_when_the_wait_is_over() {
        let mut contest = Contest::default();
        let t0 = Instant::now();
        assert_eq!(contest.raise_over(1, t0), Answer::Raise, "a window pinned over the pill");
        assert_eq!(
            contest.raise_over(1, t0 + ms(20)),
            Answer::LookAgainIn(AGAIN_AFTER - ms(20)),
            "the same window on top again at once (unpinned and pinned again, a higher band, or it puts itself back): look again later"
        );
        assert_eq!(contest.raise_over(1, t0 + AGAIN_AFTER), Answer::Raise, "still over the pill when the keeper looks again");
        assert_eq!(
            contest.raise_over(1, t0 + AGAIN_AFTER + ms(5)),
            Answer::LookAgainIn(AGAIN_AFTER - ms(5)),
            "a window that puts itself back on top is raised over once every AGAIN_AFTER"
        );
        assert_eq!(contest.raise_over(1, t0 + ms(2000)), Answer::Raise, "the same window pinned on top again later");
    }

    #[test]
    fn another_window_over_the_pill_is_raised_over_at_once() {
        let mut contest = Contest::default();
        let t0 = Instant::now();
        assert_eq!(contest.raise_over(1, t0), Answer::Raise);
        assert_eq!(contest.raise_over(2, t0 + ms(10)), Answer::Raise, "another window over the pill");
        assert_eq!(contest.raise_over(1, t0 + ms(20)), Answer::Raise, "the first window, covering after the other one");
    }

    #[test]
    fn a_window_coming_to_the_front_makes_the_next_cover_a_new_event() {
        let mut contest = Contest::default();
        let t0 = Instant::now();
        assert_eq!(contest.raise_over(1, t0), Answer::Raise);
        contest.front();
        assert_eq!(contest.raise_over(1, t0 + ms(10)), Answer::Raise, "activated since, the window may be raised over again at once");
    }

    /// Two programs that each put their window back on top whenever it is covered would keep
    /// raising the pill and each other: the keeper raises at most `RAISES_PER_SECOND` times a
    /// second, and goes on in the next one.
    #[test]
    fn raises_over_windows_that_take_turns_on_top_are_bounded_per_second() {
        let mut contest = Contest::default();
        let t0 = Instant::now();
        let raised =
            (0..1000u32).filter(|&i| contest.raise_over(1 + isize::from(i % 2 == 1), t0 + Duration::from_micros(u64::from(i) * 500)) == Answer::Raise).count();
        assert_eq!(raised, RAISES_PER_SECOND, "within half a second");
        assert_eq!(contest.raise_over(3, t0 + ms(1600)), Answer::Raise, "a second later the keeper raises again");
    }
}
