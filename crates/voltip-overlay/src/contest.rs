//! When the Windows keeper raises the pill over a window that covers it (docs/dictation.md §32).
//! Kept apart from the Win32 calls (`windows.rs`) so that its rules are tested on every platform.
//!
//! A window that comes to the front always gets the pill raised over it. A window found over the
//! pill after another window appeared or the stacking order changed gets it once: on top again,
//! it is in one of the shell's bands above the pill's (the Start menu, notifications) or a
//! program that puts its window back on top each time, and raising again would only start a
//! contest. At most [`MAX_RAISES`] such raises happen between two windows coming to the front.

/// How many times the pill is raised over covering windows between two windows coming to the
/// front: a bound for programs that put their windows back on top each time.
pub const MAX_RAISES: u32 = 32;

/// The keeper's record since a window last came to the front.
#[derive(Debug, Default)]
pub struct Contest {
    /// The window the pill was last raised over.
    raised_over: Option<isize>,
    /// Raises over covering windows.
    raises: u32,
}

impl Contest {
    /// A window came to the front: the pill is raised over it, and the record starts again.
    pub fn front(&mut self) {
        *self = Self::default();
    }

    /// After a window appeared or the stacking order changed, `cover` covers the pill: whether to
    /// raise the pill over it.
    pub fn raise_over(&mut self, cover: isize) -> bool {
        if self.raised_over == Some(cover) || self.raises >= MAX_RAISES {
            return false;
        }
        self.raised_over = Some(cover);
        self.raises += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_covering_window_gets_the_pill_raised_over_it_once() {
        let mut contest = Contest::default();
        assert!(contest.raise_over(1), "a window pinned over the pill");
        assert!(!contest.raise_over(1), "the same window on top again: a higher band, or it puts itself back");
        assert!(contest.raise_over(2), "another window over the pill");
        assert!(contest.raise_over(1), "the first window, covering after the other one");
    }

    #[test]
    fn a_window_coming_to_the_front_starts_the_record_again() {
        let mut contest = Contest::default();
        assert!(contest.raise_over(1));
        contest.front();
        assert!(contest.raise_over(1), "activated since, the window may be raised over again");
    }

    /// Two programs that each put their window back on top whenever it is covered would keep
    /// raising the pill and each other: the keeper stops after `MAX_RAISES`.
    #[test]
    fn raises_over_windows_that_take_turns_on_top_are_bounded() {
        let mut contest = Contest::default();
        let raised = (0..1000).filter(|i| contest.raise_over(1 + i % 2)).count();
        assert_eq!(raised, usize::try_from(MAX_RAISES).unwrap_or(usize::MAX));
        contest.front();
        assert!(contest.raise_over(1), "a window coming to the front lifts the bound");
    }
}
