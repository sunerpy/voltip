//! Activation: how key edges become dictation intents (docs/dictation.md §13).
//!
//! Pure and clock-free: every edge carries its own time stamp and the owner (the core runtime)
//! calls [`ActivationMachine::poll`] when the deadline the machine asked for
//! ([`ActivationMachine::deadline_ms`]) has passed. The machine never touches the engine; it says
//! what should happen ([`Intent`]) and the runtime maps that onto the existing start / stop /
//! cancel paths, then reports the resulting phase back with [`ActivationMachine::on_phase`].

#![warn(missing_docs)]

use serde::{Deserialize, Serialize};

use super::DictationPhase;

/// How the hotkey drives a dictation (`Settings.activation`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Activation {
    /// Press → start, release → stop (default).
    #[default]
    Hold,
    /// Every press alternates start / stop; releases are ignored.
    Toggle,
    /// Press → start. A release after `hold_threshold_ms` stops; a shorter press locks the run
    /// (`Listening.locked`) until the next press stops it.
    HoldOrToggle,
}

/// Default `hold_threshold_ms`: shorter presses lock in `hold_or_toggle`.
pub const DEFAULT_HOLD_THRESHOLD_MS: u32 = 300;
/// Default debounce window for hotkey edges (contact bounce).
pub const DEFAULT_DEBOUNCE_MS: u32 = 30;
/// Default grace after a release within which a press is X11 auto-repeat, not a new press.
pub const DEFAULT_RELEASE_GRACE_MS: u32 = 50;

/// Timing knobs of the machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActivationConfig {
    /// Mode.
    pub mode: Activation,
    /// `hold_or_toggle`: a release this long after the press stops, a shorter one locks.
    pub hold_threshold_ms: u32,
    /// Hotkey presses closer than this to the previous accepted press are dropped (CLI edges are
    /// exempt so `voltip --toggle` twice in a row still alternates).
    pub debounce_ms: u32,
    /// A release's effect (stop / lock) waits this long; a press inside the window is auto-repeat
    /// or bounce and continues the hold instead.
    pub release_grace_ms: u32,
}

impl Default for ActivationConfig {
    fn default() -> Self {
        Self {
            mode: Activation::Hold,
            hold_threshold_ms: DEFAULT_HOLD_THRESHOLD_MS,
            debounce_ms: DEFAULT_DEBOUNCE_MS,
            release_grace_ms: DEFAULT_RELEASE_GRACE_MS,
        }
    }
}

/// Who produced an edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeSource {
    /// The OS global hotkey (debounced, auto-repeat filtered).
    Hotkey,
    /// `voltip --toggle` forwarded by the running instance (exempt from debounce and grace; a
    /// press always alternates start / stop).
    Cli,
    /// A webview or test edge (treated like a hotkey).
    Ui,
}

/// One key transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    /// `true` = key down.
    pub pressed: bool,
    /// When it happened, in the owner's millisecond clock (only differences matter).
    pub at_ms: u64,
    /// Origin.
    pub source: EdgeSource,
}

/// The dictation phase as the machine needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhaseHint {
    /// Nothing running, or a terminal state that a start may interrupt.
    Idle,
    /// The microphone is open.
    Listening,
    /// The recording is being turned into text (or the extra recording window is running): a
    /// press now is remembered and started when the phase returns to `Idle`.
    Processing,
}

impl From<&DictationPhase> for PhaseHint {
    fn from(phase: &DictationPhase) -> Self {
        match phase {
            DictationPhase::Listening { .. } => Self::Listening,
            DictationPhase::Processing { .. } => Self::Processing,
            DictationPhase::Idle | DictationPhase::Done { .. } | DictationPhase::Failed { .. } | DictationPhase::Cancelled { .. } => Self::Idle,
        }
    }
}

/// What the runtime should do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    /// Open the microphone (`DictationStart`).
    Start,
    /// Close it and run the pipeline (`DictationStop`).
    Stop,
    /// Discard the recording or the pending result (`DictationCancel`): a CLI release edge, the
    /// edge-path spelling of `voltip --cancel`.
    Cancel,
    /// `hold_or_toggle`: the short press locked the run; keep listening and show the lock.
    Lock,
    /// The edge was consumed without effect (bounce, auto-repeat, a release in `toggle`, a press
    /// remembered as pending).
    Ignore,
}

/// A release whose effect waits out the grace window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PendingRelease {
    at_ms: u64,
    deadline_ms: u64,
}

/// A press during `Processing`, started once the phase is `Idle` again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PendingStart {
    at_ms: u64,
    /// From the CLI: starts whatever the key state (there is no key).
    cli: bool,
}

/// The activation state machine.
#[derive(Clone, Debug)]
pub struct ActivationMachine {
    config: ActivationConfig,
    /// A `Start` went out for the current run and no `Stop` / `Cancel` followed (kept in step with
    /// the engine through [`ActivationMachine::on_phase`]).
    active: bool,
    /// `hold_or_toggle`: the run is locked; the next press stops it.
    locked: bool,
    /// The press that started the current run (hold length).
    pressed_at: Option<u64>,
    /// Physical key state as far as the edges say.
    key_down: bool,
    /// Last press accepted (debounce).
    last_press_ms: Option<u64>,
    /// Last release seen (auto-repeat grace).
    last_release_ms: Option<u64>,
    /// A release waiting for the grace window to pass.
    pending_release: Option<PendingRelease>,
    /// A press during `Processing`, to start when the phase returns to `Idle`.
    pending_start: Option<PendingStart>,
}

impl Default for ActivationMachine {
    fn default() -> Self {
        Self::new(ActivationConfig::default())
    }
}

impl ActivationMachine {
    /// A machine in `Idle` with `config`.
    pub fn new(config: ActivationConfig) -> Self {
        Self {
            config,
            active: false,
            locked: false,
            pressed_at: None,
            key_down: false,
            last_press_ms: None,
            last_release_ms: None,
            pending_release: None,
            pending_start: None,
        }
    }

    /// Current configuration.
    pub fn config(&self) -> &ActivationConfig {
        &self.config
    }

    /// Replace the configuration. A run in progress keeps running; a pending release or a pending
    /// start is dropped (its meaning depends on the mode that saw the press).
    pub fn set_config(&mut self, config: ActivationConfig) {
        self.config = config;
        self.pending_release = None;
        self.pending_start = None;
        self.locked = false;
    }

    /// The machine believes a run it started is listening.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// `hold_or_toggle` locked the run.
    pub fn is_locked(&self) -> bool {
        self.locked
    }

    /// A press is waiting for the phase to return to `Idle`.
    pub fn has_pending_start(&self) -> bool {
        self.pending_start.is_some()
    }

    /// When [`ActivationMachine::poll`] must be called next, if a release is waiting.
    pub fn deadline_ms(&self) -> Option<u64> {
        self.pending_release.map(|p| p.deadline_ms)
    }

    /// Fold an edge; `phase` is the engine's phase right now.
    pub fn feed(&mut self, edge: Edge, phase: PhaseHint) -> Vec<Intent> {
        let Edge { pressed, at_ms, source } = edge;
        self.sync(phase);
        if source == EdgeSource::Cli {
            return self.cli_edge(pressed, at_ms, phase);
        }
        if pressed { self.press(at_ms, phase) } else { self.release(at_ms, phase) }
    }

    /// Another key or button joined the held key (the lone-key trigger, docs/dictation.md §13.1):
    /// the press began a shortcut such as Right Ctrl + C, not a take. The run this very press
    /// started is cancelled and a start it left pending is dropped; a run that began otherwise (an
    /// earlier press in `toggle`, the UI button, the CLI) goes on. The key counts as up from here:
    /// the shell does not report its physical release.
    pub fn chorded(&mut self, phase: PhaseHint) -> Vec<Intent> {
        self.sync(phase);
        let own = if self.key_down { self.last_press_ms } else { None };
        self.key_down = false;
        let Some(own) = own else { return vec![Intent::Ignore] };
        if phase == PhaseHint::Processing {
            if self.pending_start.is_some_and(|p| !p.cli && p.at_ms == own) {
                self.pending_start = None;
            }
            return vec![Intent::Ignore];
        }
        if self.active && self.pressed_at == Some(own) {
            self.end();
            return vec![Intent::Cancel];
        }
        vec![Intent::Ignore]
    }

    /// Resolve a release whose grace window has passed (`now_ms >= deadline_ms`). Nothing happens
    /// before the deadline or when no release is pending.
    pub fn poll(&mut self, now_ms: u64, phase: PhaseHint) -> Vec<Intent> {
        let Some(pending) = self.pending_release else { return Vec::new() };
        if now_ms < pending.deadline_ms {
            return Vec::new();
        }
        self.pending_release = None;
        self.sync(phase);
        if !self.active {
            return Vec::new();
        }
        match self.config.mode {
            Activation::Hold => {
                self.end();
                vec![Intent::Stop]
            }
            Activation::HoldOrToggle => {
                let held = pending.at_ms.saturating_sub(self.pressed_at.unwrap_or(pending.at_ms));
                if held >= u64::from(self.config.hold_threshold_ms) {
                    self.end();
                    vec![Intent::Stop]
                } else {
                    self.locked = true;
                    vec![Intent::Lock]
                }
            }
            Activation::Toggle => Vec::new(),
        }
    }

    /// The engine's phase changed. Keeps the machine in step (a run stopped by the UI button, the
    /// auto-stop or a failure is no longer `active`) and fires a pending start once the phase is
    /// `Idle` again: `Start`, plus `Lock` in `hold_or_toggle` when the key was already released
    /// (a tap). In `hold` a pending press whose key is already up is dropped: nobody is holding.
    pub fn on_phase(&mut self, phase: PhaseHint) -> Vec<Intent> {
        self.sync(phase);
        if phase != PhaseHint::Idle {
            return Vec::new();
        }
        let Some(PendingStart { at_ms, cli }) = self.pending_start.take() else { return Vec::new() };
        match self.config.mode {
            Activation::Hold if !cli && !self.key_down => Vec::new(),
            Activation::HoldOrToggle if !cli && !self.key_down => {
                self.begin(at_ms);
                self.locked = true;
                vec![Intent::Start, Intent::Lock]
            }
            Activation::Hold | Activation::Toggle | Activation::HoldOrToggle => {
                self.begin(at_ms);
                vec![Intent::Start]
            }
        }
    }

    /// The runtime could not start the run the last `Start` asked for (device busy, model missing):
    /// back to idle, as if the press never happened.
    pub fn on_start_failed(&mut self) {
        self.active = false;
        self.locked = false;
        self.pressed_at = None;
        self.pending_release = None;
    }

    fn sync(&mut self, phase: PhaseHint) {
        match phase {
            PhaseHint::Listening => self.active = true,
            PhaseHint::Idle | PhaseHint::Processing => {
                self.active = false;
                self.locked = false;
                self.pressed_at = None;
                self.pending_release = None;
            }
        }
    }

    fn toggle_pending(&mut self, at_ms: u64, cli: bool) {
        self.pending_start = if self.pending_start.is_some() { None } else { Some(PendingStart { at_ms, cli }) };
    }

    fn begin(&mut self, at_ms: u64) {
        self.active = true;
        self.locked = false;
        self.pressed_at = Some(at_ms);
        self.pending_release = None;
    }

    fn end(&mut self) {
        self.active = false;
        self.locked = false;
        self.pressed_at = None;
        self.pending_release = None;
    }

    fn press(&mut self, at_ms: u64, phase: PhaseHint) -> Vec<Intent> {
        // X11 auto-repeat (release + press with the same time stamp) and contact bounce on the
        // way down: the press continues the hold that the release would have ended.
        if let Some(release) = self.last_release_ms
            && at_ms.saturating_sub(release) <= u64::from(self.config.release_grace_ms)
        {
            self.pending_release = None;
            self.key_down = true;
            return vec![Intent::Ignore];
        }
        if let Some(last) = self.last_press_ms
            && at_ms.saturating_sub(last) < u64::from(self.config.debounce_ms)
        {
            return vec![Intent::Ignore];
        }
        self.last_press_ms = Some(at_ms);
        self.key_down = true;
        if phase == PhaseHint::Processing {
            // Remembered for when the phase returns to Idle; the same key again cancels it.
            self.toggle_pending(at_ms, false);
            return vec![Intent::Ignore];
        }
        match self.config.mode {
            Activation::Hold if self.active => vec![Intent::Ignore],
            Activation::Toggle | Activation::HoldOrToggle if self.active => {
                self.end();
                vec![Intent::Stop]
            }
            Activation::Hold | Activation::Toggle | Activation::HoldOrToggle => {
                self.begin(at_ms);
                vec![Intent::Start]
            }
        }
    }

    fn release(&mut self, at_ms: u64, phase: PhaseHint) -> Vec<Intent> {
        self.last_release_ms = Some(at_ms);
        self.key_down = false;
        if phase == PhaseHint::Processing {
            if self.config.mode == Activation::Hold && self.pending_start.is_some_and(|p| !p.cli) {
                // The key went up before the run could start: nothing to hold.
                self.pending_start = None;
            }
            return vec![Intent::Ignore];
        }
        if !self.active || self.locked {
            return vec![Intent::Ignore];
        }
        match self.config.mode {
            Activation::Toggle => vec![Intent::Ignore],
            Activation::Hold | Activation::HoldOrToggle => {
                self.pending_release = Some(PendingRelease { at_ms, deadline_ms: at_ms + u64::from(self.config.release_grace_ms) });
                vec![Intent::Ignore]
            }
        }
    }

    /// CLI edges bypass debounce and grace and never touch the hotkey's key state: a press
    /// alternates start / stop whatever the mode (pending while processing, like a key); a release
    /// is `voltip --cancel`.
    fn cli_edge(&mut self, pressed: bool, at_ms: u64, phase: PhaseHint) -> Vec<Intent> {
        if !pressed {
            let something_to_cancel = self.active || self.pending_start.is_some() || phase != PhaseHint::Idle;
            self.pending_start = None;
            self.end();
            return if something_to_cancel { vec![Intent::Cancel] } else { vec![Intent::Ignore] };
        }
        if phase == PhaseHint::Processing {
            self.toggle_pending(at_ms, true);
            return vec![Intent::Ignore];
        }
        if self.active {
            self.end();
            vec![Intent::Stop]
        } else {
            self.begin(at_ms);
            vec![Intent::Start]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use Activation::{Hold, HoldOrToggle, Toggle};
    use Intent::{Cancel, Ignore, Lock, Start, Stop};
    use PhaseHint::{Idle, Listening, Processing};

    /// One scripted step: an edge (or a poll at a time) with the phase the engine is in, and what
    /// the machine must answer.
    enum Step {
        Edge(bool, u64, EdgeSource, PhaseHint, Vec<Intent>),
        Poll(u64, PhaseHint, Vec<Intent>),
        Phase(PhaseHint, Vec<Intent>),
        Chord(PhaseHint, Vec<Intent>),
    }

    fn press(at_ms: u64, phase: PhaseHint, expect: Vec<Intent>) -> Step {
        Step::Edge(true, at_ms, EdgeSource::Hotkey, phase, expect)
    }

    fn release(at_ms: u64, phase: PhaseHint, expect: Vec<Intent>) -> Step {
        Step::Edge(false, at_ms, EdgeSource::Hotkey, phase, expect)
    }

    fn poll(at_ms: u64, phase: PhaseHint, expect: Vec<Intent>) -> Step {
        Step::Poll(at_ms, phase, expect)
    }

    fn phase(phase: PhaseHint, expect: Vec<Intent>) -> Step {
        Step::Phase(phase, expect)
    }

    fn chord(phase: PhaseHint, expect: Vec<Intent>) -> Step {
        Step::Chord(phase, expect)
    }

    fn cli(pressed: bool, at_ms: u64, phase: PhaseHint, expect: Vec<Intent>) -> Step {
        Step::Edge(pressed, at_ms, EdgeSource::Cli, phase, expect)
    }

    fn run(mode: Activation, script: Vec<Step>) -> ActivationMachine {
        let mut m = ActivationMachine::new(ActivationConfig { mode, ..ActivationConfig::default() });
        for (i, step) in script.into_iter().enumerate() {
            let (got, want, what) = match step {
                Step::Edge(pressed, at_ms, source, phase, want) => {
                    (m.feed(Edge { pressed, at_ms, source }, phase), want, format!("edge pressed={pressed} at={at_ms} {source:?} {phase:?}"))
                }
                Step::Poll(at_ms, phase, want) => (m.poll(at_ms, phase), want, format!("poll at={at_ms} {phase:?}")),
                Step::Phase(phase, want) => (m.on_phase(phase), want, format!("phase {phase:?}")),
                Step::Chord(phase, want) => (m.chorded(phase), want, format!("chord {phase:?}")),
            };
            assert_eq!(got, want, "{mode:?} step {i}: {what}\n{m:?}");
        }
        m
    }

    #[test]
    fn hold_maps_press_to_start_and_release_to_stop() {
        let m = run(
            Hold,
            vec![
                press(1_000, Idle, vec![Start]),
                phase(Listening, vec![]),
                // A second press without a release (lost focus) changes nothing.
                press(1_500, Listening, vec![Ignore]),
                release(3_000, Listening, vec![Ignore]),
                // The release waits out the grace window, then stops.
                poll(3_049, Listening, vec![]),
                poll(3_050, Listening, vec![Stop]),
                phase(Processing, vec![]),
                phase(Idle, vec![]),
                // A UI-started run is stopped by a hold press / release too.
                phase(Listening, vec![]),
                press(9_000, Listening, vec![Ignore]),
                release(9_400, Listening, vec![Ignore]),
                poll(9_450, Listening, vec![Stop]),
            ],
        );
        assert!(!m.is_active() && !m.is_locked() && m.deadline_ms().is_none());
        assert_eq!(ActivationConfig::default(), ActivationConfig { mode: Hold, hold_threshold_ms: 300, debounce_ms: 30, release_grace_ms: 50 });
        assert_eq!(ActivationMachine::default().config().mode, Hold);
        for (mode, wire) in [(Hold, r#""hold""#), (Toggle, r#""toggle""#), (HoldOrToggle, r#""hold_or_toggle""#)] {
            assert_eq!(serde_json::to_string(&mode).unwrap(), wire);
            assert_eq!(serde_json::from_str::<Activation>(wire).unwrap(), mode);
        }
        assert_eq!(serde_json::to_string(&EdgeSource::Cli).unwrap(), r#""cli""#);
        let edge: Edge = serde_json::from_str(r#"{"pressed":true,"at_ms":7,"source":"ui"}"#).unwrap();
        assert_eq!(edge, Edge { pressed: true, at_ms: 7, source: EdgeSource::Ui });
        assert_eq!(PhaseHint::from(&DictationPhase::Idle), Idle);
        assert_eq!(PhaseHint::from(&DictationPhase::CANCELLED), Idle);
        assert_eq!(PhaseHint::from(&DictationPhase::Listening { started_at: 1, ready: true, live: None, locked: false }), Listening);
        assert_eq!(PhaseHint::from(&DictationPhase::Processing { stage: super::super::ProcessingStage::Refining, started_at: 1, preview: None }), Processing);
    }

    #[test]
    fn toggle_ignores_release_and_alternates() {
        let mut m = run(
            Toggle,
            vec![
                press(1_000, Idle, vec![Start]),
                phase(Listening, vec![]),
                release(1_200, Listening, vec![Ignore]),
                poll(1_300, Listening, vec![]),
                press(5_000, Listening, vec![Stop]),
                release(5_100, Processing, vec![Ignore]),
                phase(Idle, vec![]),
                press(7_000, Idle, vec![Start]),
                phase(Listening, vec![]),
                // The auto-stop / a UI stop ended the run: the next press starts again.
                phase(Processing, vec![]),
                phase(Idle, vec![]),
                press(20_000, Idle, vec![Start]),
            ],
        );
        assert!(m.is_active());
        // Changing the mode keeps the run and drops the transient state.
        m.set_config(ActivationConfig { mode: Hold, ..ActivationConfig::default() });
        assert!(m.is_active() && m.config().mode == Hold && m.deadline_ms().is_none());
    }

    #[test]
    fn hold_or_toggle_locks_on_a_short_press_and_stops_on_the_next() {
        let m = run(
            HoldOrToggle,
            vec![
                // Tap: 120 ms < 300 ms → lock after the grace window.
                press(1_000, Idle, vec![Start]),
                phase(Listening, vec![]),
                release(1_120, Listening, vec![Ignore]),
                poll(1_170, Listening, vec![Lock]),
                // Locked: releases do nothing, the next press stops.
                release(1_500, Listening, vec![Ignore]),
                press(4_000, Listening, vec![Stop]),
                release(4_080, Processing, vec![Ignore]),
                phase(Idle, vec![]),
                // Long press: release ≥ 300 ms after the press stops.
                press(6_000, Idle, vec![Start]),
                phase(Listening, vec![]),
                release(6_300, Listening, vec![Ignore]),
                poll(6_350, Listening, vec![Stop]),
                phase(Processing, vec![]),
                phase(Idle, vec![]),
            ],
        );
        assert!(!m.is_locked() && !m.is_active());
        let locked = run(
            HoldOrToggle,
            vec![press(0, Idle, vec![Start]), phase(Listening, vec![]), release(100, Listening, vec![Ignore]), poll(150, Listening, vec![Lock])],
        );
        assert!(locked.is_locked() && locked.is_active());
    }

    /// X11 has no detectable auto-repeat through the hotkey plugin: a held key arrives as
    /// release + press pairs (same time stamp) every repeat period. They must not end the hold,
    /// and the final, real release still stops it.
    #[test]
    fn regression_x11_auto_repeat_pairs_do_not_interrupt_a_hold() {
        let m = run(
            Hold,
            vec![
                press(0, Idle, vec![Start]),
                phase(Listening, vec![]),
                release(500, Listening, vec![Ignore]),
                press(500, Listening, vec![Ignore]),
                poll(550, Listening, vec![]),
                release(530, Listening, vec![Ignore]),
                press(531, Listening, vec![Ignore]),
                release(560, Listening, vec![Ignore]),
                press(590, Listening, vec![Ignore]),
                poll(640, Listening, vec![]),
                // The real release: no press follows within 50 ms.
                release(2_000, Listening, vec![Ignore]),
                poll(2_049, Listening, vec![]),
                poll(2_050, Listening, vec![Stop]),
            ],
        );
        assert!(!m.is_active());
        // In toggle mode the repeat pairs must not alternate the run either.
        run(
            Toggle,
            vec![
                press(0, Idle, vec![Start]),
                phase(Listening, vec![]),
                release(500, Listening, vec![Ignore]),
                press(500, Listening, vec![Ignore]),
                release(530, Listening, vec![Ignore]),
                press(530, Listening, vec![Ignore]),
                release(560, Listening, vec![Ignore]),
                // A press well after the last release is the user again.
                press(2_000, Listening, vec![Stop]),
            ],
        );
        // hold_or_toggle: repeats during a long hold do not lock it; the release stops.
        run(
            HoldOrToggle,
            vec![
                press(0, Idle, vec![Start]),
                phase(Listening, vec![]),
                release(500, Listening, vec![Ignore]),
                press(500, Listening, vec![Ignore]),
                release(1_000, Listening, vec![Ignore]),
                poll(1_050, Listening, vec![Stop]),
            ],
        );
    }

    #[test]
    fn debounce_drops_bounces_but_never_cli_edges() {
        // Repeated presses without a release: dropped within 30 ms of the accepted one.
        run(
            Toggle,
            vec![
                press(1_000, Idle, vec![Start]),
                phase(Listening, vec![]),
                press(1_010, Listening, vec![Ignore]),
                press(1_029, Listening, vec![Ignore]),
                // 30 ms after the accepted press is a new press.
                press(1_030, Listening, vec![Stop]),
                phase(Processing, vec![]),
                phase(Idle, vec![]),
                press(1_059, Idle, vec![Ignore]),
                press(1_060, Idle, vec![Start]),
            ],
        );
        // Contact bounce on the way down: press, release, press within a few ms.
        run(
            Toggle,
            vec![
                press(1_000, Idle, vec![Start]),
                phase(Listening, vec![]),
                release(1_003, Listening, vec![Ignore]),
                press(1_006, Listening, vec![Ignore]),
                press(1_029, Listening, vec![Ignore]),
                press(1_100, Listening, vec![Stop]),
            ],
        );
        // Bounce on the way up in hold mode: the hold continues, the last release stops.
        run(
            Hold,
            vec![
                press(0, Idle, vec![Start]),
                phase(Listening, vec![]),
                release(1_000, Listening, vec![Ignore]),
                press(1_003, Listening, vec![Ignore]),
                release(1_006, Listening, vec![Ignore]),
                poll(1_056, Listening, vec![Stop]),
            ],
        );
        // CLI edges: two toggles 1 ms apart alternate; a release is a cancel; a press while
        // processing is pending like a key, but the CLI never touches the hotkey's debounce clock.
        let cli = EdgeSource::Cli;
        let mut m = run(
            Hold,
            vec![
                Step::Edge(true, 1_000, cli, Idle, vec![Start]),
                phase(Listening, vec![]),
                Step::Edge(true, 1_001, cli, Listening, vec![Stop]),
                phase(Processing, vec![]),
                Step::Edge(false, 1_002, cli, Processing, vec![Cancel]),
                phase(Idle, vec![]),
                Step::Edge(false, 1_003, cli, Idle, vec![Ignore]),
                Step::Edge(true, 1_004, cli, Idle, vec![Start]),
                phase(Listening, vec![]),
                press(1_005, Listening, vec![Ignore]),
                release(1_400, Listening, vec![Ignore]),
                poll(1_450, Listening, vec![Stop]),
                phase(Processing, vec![]),
                Step::Edge(true, 1_500, cli, Processing, vec![Ignore]),
            ],
        );
        assert!(m.has_pending_start());
        assert_eq!(m.on_phase(Idle), vec![Start], "a CLI press while processing starts once idle");
        assert!(m.is_active());
    }

    #[test]
    fn press_while_processing_is_pending_and_a_second_press_cancels_it() {
        // Toggle: the press is remembered and fires on Idle.
        let mut m = run(
            Toggle,
            vec![
                press(0, Idle, vec![Start]),
                phase(Listening, vec![]),
                press(1_000, Listening, vec![Stop]),
                phase(Processing, vec![]),
                press(1_200, Processing, vec![Ignore]),
                release(1_300, Processing, vec![Ignore]),
            ],
        );
        assert!(m.has_pending_start());
        assert_eq!(m.on_phase(Processing), vec![]);
        assert_eq!(m.on_phase(Idle), vec![Start]);
        assert!(m.is_active() && !m.has_pending_start());
        // The same key again while still processing cancels the pending start.
        let m = run(
            Toggle,
            vec![
                press(0, Idle, vec![Start]),
                phase(Listening, vec![]),
                press(1_000, Listening, vec![Stop]),
                phase(Processing, vec![]),
                press(1_200, Processing, vec![Ignore]),
                press(1_400, Processing, vec![Ignore]),
                phase(Idle, vec![]),
            ],
        );
        assert!(!m.is_active() && !m.has_pending_start());
        // hold_or_toggle: a tap during processing starts locked once idle; a held key starts a hold.
        let m = run(
            HoldOrToggle,
            vec![phase(Processing, vec![]), press(100, Processing, vec![Ignore]), release(200, Processing, vec![Ignore]), phase(Idle, vec![Start, Lock])],
        );
        assert!(m.is_locked());
        let m = run(HoldOrToggle, vec![phase(Processing, vec![]), press(100, Processing, vec![Ignore]), phase(Idle, vec![Start]), phase(Listening, vec![])]);
        assert!(m.is_active() && !m.is_locked());
        // hold: a press whose key went up again before Idle is dropped, a key still down starts.
        run(Hold, vec![phase(Processing, vec![]), press(100, Processing, vec![Ignore]), release(300, Processing, vec![Ignore]), phase(Idle, vec![])]);
        run(Hold, vec![phase(Processing, vec![]), press(100, Processing, vec![Ignore]), phase(Idle, vec![Start])]);
    }

    /// docs/dictation.md §13.1: a lone-key trigger that turns into a chord (Right Ctrl + C)
    /// cancels only the run its own press started.
    #[test]
    fn regression_a_chorded_trigger_cancels_the_take_its_press_started_and_nothing_else() {
        for mode in [Hold, Toggle, HoldOrToggle] {
            let m = run(mode, vec![press(1_000, Idle, vec![Start]), phase(Listening, vec![]), chord(Listening, vec![Cancel]), phase(Idle, vec![])]);
            assert!(!m.is_active() && m.deadline_ms().is_none(), "{mode:?}");
        }
        // toggle: the press that stopped a run is not taken back; the result goes on to processing.
        run(
            Toggle,
            vec![
                press(1_000, Idle, vec![Start]),
                phase(Listening, vec![]),
                release(1_100, Listening, vec![Ignore]),
                press(5_000, Listening, vec![Stop]),
                chord(Processing, vec![Ignore]),
            ],
        );
        // A run the CLI started survives a chord of the (held, ignored) hotkey press.
        run(Hold, vec![cli(true, 500, Idle, vec![Start]), phase(Listening, vec![]), press(1_000, Listening, vec![Ignore]), chord(Listening, vec![Ignore])]);
        // A press remembered during processing is dropped, so nothing starts once idle.
        let m = run(Toggle, vec![phase(Processing, vec![]), press(100, Processing, vec![Ignore]), chord(Processing, vec![Ignore]), phase(Idle, vec![])]);
        assert!(!m.has_pending_start() && !m.is_active());
        // A chord with no key down (the shell reports each chord once) changes nothing.
        run(Hold, vec![chord(Idle, vec![Ignore]), press(1_000, Idle, vec![Start]), phase(Listening, vec![])]);
        // After a chord the next lone press starts again.
        run(
            Hold,
            vec![
                press(1_000, Idle, vec![Start]),
                phase(Listening, vec![]),
                chord(Listening, vec![Cancel]),
                phase(Idle, vec![]),
                press(3_000, Idle, vec![Start]),
                phase(Listening, vec![]),
                release(4_000, Listening, vec![Ignore]),
                poll(4_050, Listening, vec![Stop]),
            ],
        );
    }

    #[test]
    fn start_failure_rolls_back_to_idle() {
        let mut m = ActivationMachine::new(ActivationConfig { mode: HoldOrToggle, ..ActivationConfig::default() });
        assert_eq!(m.feed(Edge { pressed: true, at_ms: 1_000, source: EdgeSource::Hotkey }, Idle), vec![Start]);
        assert!(m.is_active());
        m.on_start_failed();
        assert!(!m.is_active() && !m.is_locked() && m.deadline_ms().is_none());
        // The release that follows the failed press is a no-op, and the next press starts again.
        assert_eq!(m.feed(Edge { pressed: false, at_ms: 1_100, source: EdgeSource::Hotkey }, Idle), vec![Ignore]);
        assert_eq!(m.poll(1_200, Idle), vec![]);
        assert_eq!(m.feed(Edge { pressed: true, at_ms: 2_000, source: EdgeSource::Hotkey }, Idle), vec![Start]);
        // A failure reported through the phase (Failed → Idle) rolls back the same way.
        assert_eq!(m.on_phase(Idle), vec![]);
        assert!(!m.is_active());
        assert_eq!(m.feed(Edge { pressed: false, at_ms: 2_500, source: EdgeSource::Hotkey }, Idle), vec![Ignore]);
        assert!(m.deadline_ms().is_none(), "no stop is pending for a run that never started");
        // A release pending when the run fails is dropped too.
        assert_eq!(m.feed(Edge { pressed: true, at_ms: 3_000, source: EdgeSource::Hotkey }, Idle), vec![Start]);
        assert_eq!(m.feed(Edge { pressed: false, at_ms: 3_400, source: EdgeSource::Hotkey }, Listening), vec![Ignore]);
        assert_eq!(m.deadline_ms(), Some(3_450));
        m.on_start_failed();
        assert_eq!(m.poll(3_450, Idle), vec![]);
        assert_eq!(m.deadline_ms(), None);
    }
}
