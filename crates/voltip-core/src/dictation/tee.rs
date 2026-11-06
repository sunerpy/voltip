//! One live tap read by two sessions (docs/dictation.md §11.10): a realtime model whose words are
//! not shown recognises the take, and the local model shows its own words, both from the capture's
//! one 16 kHz tap. Each session reads at its own pace from its own copy; whichever reads takes what
//! the tap has and hands the other its share, so a session that ends early, or falls behind,
//! leaves the other unaffected.

use std::collections::VecDeque;
use std::sync::Arc;

use parking_lot::Mutex;

use super::ports::LivePcm;

/// How many samples a copy holds for a session that falls behind: two seconds, as the capture's
/// own tap. A session further behind than that reports an overrun and gives up its preview.
const COPY_SAMPLES: usize = 2 * 16_000;

/// What one read takes from the tap at most before handing it out.
const PULL_SAMPLES: usize = 1_600;

/// Split `tap` into two taps that each deliver every sample `tap` delivers, in order.
pub fn tee(tap: Box<dyn LivePcm>) -> (Box<dyn LivePcm>, Box<dyn LivePcm>) {
    let shared = Arc::new(Mutex::new(Shared { tap, copies: [Queue::default(), Queue::default()] }));
    (Box::new(Half { shared: shared.clone(), index: 0 }), Box::new(Half { shared, index: 1 }))
}

struct Shared {
    tap: Box<dyn LivePcm>,
    copies: [Queue; 2],
}

/// One session's share of the tap.
#[derive(Default)]
struct Queue {
    samples: VecDeque<f32>,
    /// Its session fell behind by more than [`COPY_SAMPLES`]: samples were dropped (sticky).
    dropped: bool,
    /// Its session let go of the tap: nothing is kept for it any more.
    gone: bool,
}

impl Shared {
    /// Everything the tap has now, into each copy still read.
    fn pull(&mut self) {
        let mut buf = [0.0_f32; PULL_SAMPLES];
        loop {
            let n = self.tap.read(&mut buf);
            if n == 0 {
                return;
            }
            for copy in self.copies.iter_mut().filter(|c| !c.gone) {
                if copy.dropped || copy.samples.len() + n > COPY_SAMPLES {
                    copy.dropped = true;
                    copy.samples.clear();
                } else {
                    copy.samples.extend(&buf[..n]);
                }
            }
        }
    }
}

struct Half {
    shared: Arc<Mutex<Shared>>,
    index: usize,
}

impl LivePcm for Half {
    fn read(&mut self, out: &mut [f32]) -> usize {
        let mut shared = self.shared.lock();
        shared.pull();
        let copy = &mut shared.copies[self.index];
        let n = out.len().min(copy.samples.len());
        for (slot, sample) in out.iter_mut().zip(copy.samples.drain(..n)) {
            *slot = sample;
        }
        n
    }

    fn overrun(&self) -> bool {
        let shared = self.shared.lock();
        shared.tap.overrun() || shared.copies[self.index].dropped
    }

    fn is_closed(&self) -> bool {
        let mut shared = self.shared.lock();
        // Closed before the pull: what the pull takes is all there will be.
        let closed = shared.tap.is_closed();
        shared.pull();
        closed && shared.copies[self.index].samples.is_empty()
    }
}

impl Drop for Half {
    fn drop(&mut self) {
        let mut shared = self.shared.lock();
        let copy = &mut shared.copies[self.index];
        copy.gone = true;
        copy.samples = VecDeque::new();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    /// A tap that delivers what the test pushes, and closes when told.
    struct Script {
        pending: Arc<Mutex<VecDeque<f32>>>,
        closed: Arc<AtomicBool>,
        overrun: Arc<AtomicBool>,
    }

    impl LivePcm for Script {
        fn read(&mut self, out: &mut [f32]) -> usize {
            let mut pending = self.pending.lock();
            let n = out.len().min(pending.len());
            for (slot, sample) in out.iter_mut().zip(pending.drain(..n)) {
                *slot = sample;
            }
            n
        }

        fn overrun(&self) -> bool {
            self.overrun.load(Ordering::SeqCst)
        }

        fn is_closed(&self) -> bool {
            self.closed.load(Ordering::SeqCst)
        }
    }

    struct Rig {
        pending: Arc<Mutex<VecDeque<f32>>>,
        closed: Arc<AtomicBool>,
        overrun: Arc<AtomicBool>,
    }

    impl Rig {
        fn new() -> (Self, Box<dyn LivePcm>, Box<dyn LivePcm>) {
            let rig = Self { pending: Arc::default(), closed: Arc::default(), overrun: Arc::default() };
            let tap = Script { pending: rig.pending.clone(), closed: rig.closed.clone(), overrun: rig.overrun.clone() };
            let (a, b) = tee(Box::new(tap));
            (rig, a, b)
        }

        fn push(&self, from: usize, n: usize) {
            self.pending.lock().extend((from..from + n).map(|i| i as f32));
        }
    }

    fn read_all(tap: &mut Box<dyn LivePcm>) -> Vec<f32> {
        let mut out = Vec::new();
        let mut buf = [0.0_f32; 700];
        loop {
            let n = tap.read(&mut buf);
            if n == 0 {
                return out;
            }
            out.extend_from_slice(&buf[..n]);
        }
    }

    fn ramp(from: usize, n: usize) -> Vec<f32> {
        (from..from + n).map(|i| i as f32).collect()
    }

    #[test]
    fn both_halves_get_every_sample_in_order_whichever_reads_first() {
        let (rig, mut a, mut b) = Rig::new();
        rig.push(0, 5_000);
        assert_eq!(read_all(&mut a), ramp(0, 5_000));
        rig.push(5_000, 3_000);
        assert_eq!(read_all(&mut b), ramp(0, 8_000), "b's copy waited for it");
        assert_eq!(read_all(&mut a), ramp(5_000, 3_000));
        assert!(!a.overrun() && !b.overrun());
        assert!(!a.is_closed() && !b.is_closed());
    }

    #[test]
    fn a_half_reports_closed_only_after_it_read_everything_that_came_before_the_close() {
        let (rig, mut a, mut b) = Rig::new();
        rig.push(0, 1_000);
        rig.closed.store(true, Ordering::SeqCst);
        assert!(!a.is_closed(), "samples are still waiting for a");
        assert_eq!(read_all(&mut a), ramp(0, 1_000));
        assert!(a.is_closed());
        assert!(!b.is_closed());
        assert_eq!(read_all(&mut b), ramp(0, 1_000));
        assert!(b.is_closed());
    }

    #[test]
    fn a_half_that_lets_go_leaves_the_other_reading() {
        let (rig, mut a, b) = Rig::new();
        rig.push(0, 2_000);
        drop(b);
        let mut got = read_all(&mut a);
        // Far more than one copy holds, read as it arrives: the gone half's share is not kept.
        for chunk in 0..(COPY_SAMPLES * 2 / 1_600) {
            rig.push(2_000 + chunk * 1_600, 1_600);
            got.extend(read_all(&mut a));
        }
        assert_eq!(got, ramp(0, 2_000 + COPY_SAMPLES * 2));
        assert!(!a.overrun());
        assert_eq!(rig.pending.lock().len(), 0, "the tap is drained");
    }

    #[test]
    fn only_the_half_that_falls_behind_overruns() {
        let (rig, mut fast, mut slow) = Rig::new();
        let mut buf = [0.0_f32; 1_600];
        let mut read = 0;
        for chunk in 0..(COPY_SAMPLES / 1_600 + 2) {
            rig.push(chunk * 1_600, 1_600);
            read += fast.read(&mut buf);
        }
        assert_eq!(read, (COPY_SAMPLES / 1_600 + 2) * 1_600);
        assert!(!fast.overrun());
        assert!(slow.overrun(), "two seconds behind: its copy dropped samples");
        assert_eq!(slow.read(&mut buf), 0, "a dropped copy delivers nothing more");
        rig.overrun.store(true, Ordering::SeqCst);
        assert!(fast.overrun(), "the capture's own overrun reaches both");
    }
}
