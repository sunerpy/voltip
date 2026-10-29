//! Missed releases of a hold-to-talk take (docs/dictation.md §29). A take held with a key ends
//! when the key goes up, and the edge that says so can get lost: the system removed the hook in
//! the meantime, a hook came back while the key was down, the window system kept the key-up. The
//! shell remembers every press it forwarded (a chord's, the lone key's) until it forwards the
//! release. While a take of a hold-type activation listens, it reads the key's physical state
//! every [`POLL_EVERY`] (`voltip_hooks::is_down`), and two readings in a row that say "up" send
//! the release the edge never brought, stamped with the first of them. A reading nobody can make
//! (a Wayland session, a key the tables do not name) changes nothing: the take then ends as it
//! always did.

use std::sync::Arc;
use std::time::Duration;

use voltip_core::{Activation, DictationPhase, EdgeSource, TakeKind, now_ms};
use voltip_hooks::HeldKey;
use voltip_inject::SessionKind;
use voltip_tauri_bridge::{Bridge, UiCommand};

use crate::hotkey::{HotkeyRegistry, linux_session};

/// How often a held key is read while its take waits for the release.
pub const POLL_EVERY: Duration = Duration::from_millis(250);

/// Where a press came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// A chord the global-shortcut plugin reported, for its purpose.
    Chord(TakeKind),
    /// The lone-key trigger (always a dictation).
    Solo,
}

/// A press whose release has not been forwarded.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Held {
    source: Source,
    key: HeldKey,
    purpose: TakeKind,
    /// When the key first read as up, of the readings in a row that say so.
    up_since: Option<u64>,
}

/// The take a press is checked against: the activation that decides what a release does, the
/// phase, and whose take it is.
#[derive(Clone, Copy, Debug)]
pub struct Take<'a> {
    /// `Settings.activation`.
    pub activation: Activation,
    /// The dictation phase.
    pub phase: &'a DictationPhase,
    /// What the current (or last) take is for.
    pub kind: TakeKind,
}

/// A release to send for a key that is up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MissedRelease {
    /// Whose press it ends.
    pub source: Source,
    /// The `HotkeyEdge` purpose of the release.
    pub purpose: TakeKind,
    /// The first reading that found the key up.
    pub at_ms: u64,
}

/// The presses waiting for their release.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeldKeys {
    held: Vec<Held>,
}

impl HeldKeys {
    /// A press was forwarded; a press from the same source replaces the earlier one.
    pub fn pressed(&mut self, source: Source, key: HeldKey, purpose: TakeKind) {
        self.released(source);
        self.held.push(Held { source, key, purpose, up_since: None });
    }

    /// The release from `source` was forwarded (or the press became a chord).
    pub fn released(&mut self, source: Source) {
        self.held.retain(|held| held.source != source);
    }

    /// Nothing to check.
    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    /// One round of readings at `now_ms`: the releases to send. Only a take that a release ends
    /// is checked (a `hold` or `hold_or_toggle` run that listens unlocked, or one processing); a
    /// take that is over forgets every press.
    pub fn poll(&mut self, take: Take<'_>, now_ms: u64, mut read: impl FnMut(&HeldKey) -> Option<bool>) -> Vec<MissedRelease> {
        match take.phase {
            DictationPhase::Listening { locked: false, .. } | DictationPhase::Processing { .. } => {}
            // The next press ends a locked run, not a release.
            DictationPhase::Listening { locked: true, .. } => return Vec::new(),
            _ => {
                self.held.clear();
                return Vec::new();
            }
        }
        if take.activation == Activation::Toggle {
            return Vec::new();
        }
        let mut missed = Vec::new();
        self.held.retain_mut(|held| {
            // The other kind's take runs: the core drops this key's edges anyway.
            if held.purpose != take.kind {
                return true;
            }
            match (read(&held.key), held.up_since) {
                (Some(false), Some(first)) => {
                    missed.push(MissedRelease { source: held.source, purpose: held.purpose, at_ms: first });
                    false
                }
                (Some(false), None) => {
                    held.up_since = Some(now_ms);
                    true
                }
                // Down, or nobody knows: two readings in a row it is not.
                (Some(true) | None, _) => {
                    held.up_since = None;
                    true
                }
            }
        });
        missed
    }
}

/// A key-state query: down, up, or `None` when nobody can tell.
type Read = dyn Fn(&HeldKey) -> Option<bool> + Send + Sync;

/// What reads a key's physical state.
#[derive(Clone)]
pub struct KeyReader(Arc<Read>);

impl KeyReader {
    /// A reader that asks `read`.
    pub fn new(read: impl Fn(&HeldKey) -> Option<bool> + Send + Sync + 'static) -> Self {
        Self(Arc::new(read))
    }

    /// The platform's reading (`voltip_hooks::is_down`). On Linux a plain X11 session only:
    /// XWayland knows the keys pressed over X11 windows alone, and a Wayland session has no
    /// reading at all.
    pub fn physical() -> Self {
        if cfg!(target_os = "linux") && !linux_session().is_some_and(|session| session.kind == SessionKind::X11) {
            return Self::new(|_| None);
        }
        Self::new(voltip_hooks::is_down)
    }

    /// Whether `key` is down; `None` when nobody can tell.
    pub fn read(&self, key: &HeldKey) -> Option<bool> {
        (self.0)(key)
    }
}

impl Default for KeyReader {
    fn default() -> Self {
        Self::physical()
    }
}

impl std::fmt::Debug for KeyReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KeyReader")
    }
}

/// Read the held keys while a take may be waiting for their release, and send the releases that
/// never came. Idle while nothing is held; ends with the core's event stream.
pub fn follow(bridge: Bridge, registry: Arc<HotkeyRegistry>) {
    let mut events = bridge.events();
    tauri::async_runtime::spawn(async move {
        let mut tick = tokio::time::interval(POLL_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let idle = registry.held_is_empty();
            tokio::select! {
                () = registry.held_changed(), if idle => {
                    tick.reset();
                    continue;
                }
                _ = tick.tick(), if !idle => {}
                event = events.recv() => {
                    if matches!(event, Err(tokio::sync::broadcast::error::RecvError::Closed)) {
                        return;
                    }
                    continue;
                }
            }
            for release in registry.poll_held(&bridge, now_ms()) {
                tracing::warn!(source = ?release.source, purpose = release.purpose.as_str(), at_ms = release.at_ms, "the key is up but its release never came; sending it");
                let command =
                    UiCommand::HotkeyEdge { pressed: false, at_ms: release.at_ms, source: EdgeSource::Hotkey, purpose: release.purpose, chorded: false };
                if let Err(e) = bridge.dispatch(command) {
                    tracing::warn!(error = %e, "missed release not accepted");
                }
                registry.released_by_reading(&bridge, release.source);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use voltip_core::SoloKey;

    use super::*;

    fn listening(locked: bool) -> DictationPhase {
        DictationPhase::Listening { started_at: 0, ready: true, locked, live: None }
    }

    fn take(activation: Activation, phase: &DictationPhase) -> Take<'_> {
        Take { activation, phase, kind: TakeKind::Dictation }
    }

    fn solo() -> HeldKey {
        HeldKey::Solo(SoloKey::RightCtrl)
    }

    #[test]
    fn a_key_read_up_twice_in_a_row_gets_its_release_at_the_first_reading() {
        let phase = listening(false);
        let mut held = HeldKeys::default();
        held.pressed(Source::Solo, solo(), TakeKind::Dictation);
        assert_eq!(held.poll(take(Activation::Hold, &phase), 750, |_| Some(true)), []);
        assert_eq!(held.poll(take(Activation::Hold, &phase), 1000, |_| Some(false)), [], "one reading is not enough");
        assert_eq!(
            held.poll(take(Activation::Hold, &phase), 1250, |_| Some(false)),
            [MissedRelease { source: Source::Solo, purpose: TakeKind::Dictation, at_ms: 1000 }]
        );
        assert!(held.is_empty(), "sent once");
        // `hold_or_toggle` listening unlocked ends the same way, and so does a chord.
        let chord = HeldKey::Chord("Space".to_owned());
        held.pressed(Source::Chord(TakeKind::Dictation), chord, TakeKind::Dictation);
        assert_eq!(held.poll(take(Activation::HoldOrToggle, &phase), 2000, |_| Some(false)), []);
        assert_eq!(
            held.poll(take(Activation::HoldOrToggle, &phase), 2250, |_| Some(false)),
            [MissedRelease { source: Source::Chord(TakeKind::Dictation), purpose: TakeKind::Dictation, at_ms: 2000 }]
        );
    }

    #[test]
    fn a_reading_that_says_down_or_nothing_starts_the_count_again() {
        let phase = listening(false);
        let mut held = HeldKeys::default();
        held.pressed(Source::Solo, solo(), TakeKind::Dictation);
        for (at, reading) in [(0, Some(false)), (250, Some(true)), (500, Some(false)), (750, None), (1000, Some(false))] {
            assert_eq!(held.poll(take(Activation::Hold, &phase), at, |_| reading), [], "at {at}");
        }
        // Wayland: never a reading, never a release.
        let mut wayland = HeldKeys::default();
        wayland.pressed(Source::Solo, solo(), TakeKind::Dictation);
        for at in 0..100 {
            assert_eq!(wayland.poll(take(Activation::Hold, &phase), at * 250, |_| None), []);
        }
        assert!(!wayland.is_empty());
        // A press again (auto-repeat, the key went down anew) forgets the reading before it.
        held.pressed(Source::Solo, solo(), TakeKind::Dictation);
        assert_eq!(held.poll(take(Activation::Hold, &phase), 1250, |_| Some(false)), []);
    }

    #[test]
    fn a_locked_run_a_toggle_take_and_the_other_kinds_take_are_not_read() {
        let reads = Cell::new(0);
        let read = |_: &HeldKey| {
            reads.set(reads.get() + 1);
            Some(false)
        };
        let mut held = HeldKeys::default();
        held.pressed(Source::Solo, solo(), TakeKind::Dictation);
        for at in [0, 250, 500] {
            // `hold_or_toggle` locked: the next press ends it, not a release.
            assert_eq!(held.poll(take(Activation::HoldOrToggle, &listening(true)), at, read), []);
            // `toggle`: a release does nothing.
            assert_eq!(held.poll(take(Activation::Toggle, &listening(false)), at, read), []);
            // A voice edit runs: the dictation key's release would be dropped anyway.
            let phase = listening(false);
            let edit = Take { activation: Activation::Hold, phase: &phase, kind: TakeKind::Edit };
            assert_eq!(held.poll(edit, at, read), []);
        }
        assert_eq!(reads.get(), 0);
        assert!(!held.is_empty(), "kept for when the take is one a release ends");
    }

    #[test]
    fn a_release_the_shell_forwarded_or_a_take_that_ended_forgets_the_press() {
        let mut held = HeldKeys::default();
        held.pressed(Source::Solo, solo(), TakeKind::Dictation);
        held.pressed(Source::Chord(TakeKind::Edit), HeldKey::Chord("E".to_owned()), TakeKind::Edit);
        held.released(Source::Solo);
        held.released(Source::Chord(TakeKind::Edit));
        assert!(held.is_empty());
        // Processing is still checked: a press that started nothing yet waits for its release.
        held.pressed(Source::Solo, solo(), TakeKind::Dictation);
        let processing = DictationPhase::Processing { stage: voltip_core::ProcessingStage::Transcribing, started_at: 0, stage_started_at: 0, preview: None };
        assert_eq!(held.poll(take(Activation::Hold, &processing), 0, |_| Some(false)), []);
        assert_eq!(held.poll(take(Activation::Hold, &processing), 250, |_| Some(false)).len(), 1);
        // Idle, cancelled, done: the take is over, the press says nothing more.
        for over in [DictationPhase::Idle, DictationPhase::Cancelled { injected_chars: 0 }] {
            held.pressed(Source::Solo, solo(), TakeKind::Dictation);
            assert_eq!(held.poll(take(Activation::Hold, &over), 0, |_| panic!("not read")), []);
            assert!(held.is_empty());
        }
    }
}
