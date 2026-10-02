//! Parts of a large body on one secure path (docs/dictation.md §20.8): the sender's window and
//! the receiver's reassembly. One [`BulkPath`] lives inside a path's secure session and goes with
//! it, so a body is put back together only from the parts of one Noise session, in order.
//!
//! The window is what keeps the relay's per-connection queue of 64 frames from overflowing (a
//! dropped frame breaks the Noise session): a path never has more than [`BULK_WINDOW`] parts the
//! other side has not confirmed, and the parts of an ended session are ahead of the next
//! session's handshake, so they never add up with its parts either.

use std::collections::VecDeque;

use voltip_protocol::app::{AppMessage, BULK_PART_BYTES};

use super::MAX_BULK_BYTES;

/// Parts a path may have sent that the other side has not confirmed.
pub const BULK_WINDOW: u64 = 4;
/// The receiver confirms every this many parts, and the last part of a body.
pub const BULK_ACK_EVERY: u64 = 2;
/// A part goes out only while the link's own queue has more free places than this, which keeps
/// room for pings and a phone take's audio.
pub const LINK_RESERVE: usize = 16;

/// Why a body is sent; a newer reply to a phone replaces the older one ([`BulkPath::cancel`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BulkKind {
    /// The computer's settings or history for a phone.
    Reply,
    /// A phone's own records.
    Upload,
}

struct Sending {
    kind: BulkKind,
    tag: u64,
    bytes: Vec<u8>,
    offset: usize,
    seq: u32,
}

/// One path's parts, both directions.
#[derive(Default)]
pub struct BulkPath {
    queue: VecDeque<Sending>,
    current: Option<Sending>,
    sent: u64,
    acked: u64,
    received: u64,
    unconfirmed: u64,
    assembly: Option<Vec<u8>>,
    next_seq: u32,
}

impl std::fmt::Debug for BulkPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BulkPath")
            .field("queued", &self.queue.len())
            .field("sending", &self.current.is_some())
            .field("sent", &self.sent)
            .field("acked", &self.acked)
            .field("received", &self.received)
            .finish()
    }
}

/// What a received part led to.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Received {
    /// A whole body, when this was its last part.
    pub body: Option<Vec<u8>>,
    /// Confirm this many parts on the same path.
    pub ack: Option<u64>,
}

impl BulkPath {
    /// Queue `body` (already encoded); `tag` names it for [`Self::pending`].
    pub fn push(&mut self, kind: BulkKind, tag: u64, body: Vec<u8>) {
        self.queue.push_back(Sending { kind, tag, bytes: body, offset: 0, seq: 0 });
    }

    /// Drop every queued or half-sent body of `kind`. One stopped halfway leaves the receiver an
    /// unfinished body, which the next `seq == 0` replaces.
    pub fn cancel(&mut self, kind: BulkKind) {
        self.queue.retain(|s| s.kind != kind);
        if self.current.as_ref().is_some_and(|s| s.kind == kind) {
            self.current = None;
        }
    }

    /// `tag` is still queued or being sent.
    pub fn pending(&self, tag: u64) -> bool {
        self.current.as_ref().is_some_and(|s| s.tag == tag) || self.queue.iter().any(|s| s.tag == tag)
    }

    /// Anything to send.
    pub fn has_work(&self) -> bool {
        self.current.is_some() || !self.queue.is_empty()
    }

    /// Parts sent and not confirmed.
    pub fn in_flight(&self) -> u64 {
        self.sent.saturating_sub(self.acked)
    }

    /// The next part, when the window and the link (`free` places in its queue) allow one.
    pub fn next_part(&mut self, free: usize) -> Option<AppMessage> {
        if self.in_flight() >= BULK_WINDOW || free <= LINK_RESERVE {
            return None;
        }
        if self.current.is_none() {
            self.current = self.queue.pop_front();
        }
        let current = self.current.as_mut()?;
        let end = (current.offset + BULK_PART_BYTES).min(current.bytes.len());
        let last = end == current.bytes.len();
        let part = AppMessage::bulk(current.seq, last, current.bytes[current.offset..end].to_vec());
        current.offset = end;
        current.seq += 1;
        if last {
            self.current = None;
        }
        self.sent += 1;
        Some(part)
    }

    /// The other side confirmed `received` parts of this session.
    pub fn on_ack(&mut self, received: u64) {
        self.acked = self.acked.max(received.min(self.sent));
    }

    /// A part arrived. Parts are put back together in order; a gap, a body larger than
    /// [`MAX_BULK_BYTES`] or a new `seq == 0` drops what was being put together. Every part counts
    /// toward the confirmations, so the sender's window never waits on a dropped one.
    pub fn on_part(&mut self, seq: u32, last: bool, bytes: &[u8]) -> Received {
        self.received += 1;
        self.unconfirmed += 1;
        let mut out = Received::default();
        if seq == 0 {
            self.assembly = Some(Vec::new());
            self.next_seq = 0;
        }
        match self.assembly.as_mut() {
            Some(body) if seq == self.next_seq && body.len() + bytes.len() <= MAX_BULK_BYTES => {
                body.extend_from_slice(bytes);
                self.next_seq += 1;
                if last {
                    out.body = self.assembly.take();
                }
            }
            Some(_) => {
                tracing::debug!(seq, expected = self.next_seq, "bulk body dropped: a part is missing or it is too large");
                self.assembly = None;
            }
            None => {}
        }
        if last || self.unconfirmed >= BULK_ACK_EVERY {
            self.unconfirmed = 0;
            out.ack = Some(self.received);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(msg: AppMessage) -> (u32, bool, Vec<u8>) {
        let AppMessage::Bulk { seq, last, bytes, .. } = msg else { panic!("a part") };
        (seq, last, bytes.into_vec())
    }

    /// Send `body` from `a` to `b` through the window, confirming as `b` asks.
    fn transfer(a: &mut BulkPath, b: &mut BulkPath) -> Vec<Vec<u8>> {
        let mut bodies = Vec::new();
        while let Some(msg) = a.next_part(64) {
            let (seq, last, bytes) = part(msg);
            let got = b.on_part(seq, last, &bytes);
            if let Some(n) = got.ack {
                a.on_ack(n);
            }
            bodies.extend(got.body);
        }
        bodies
    }

    #[test]
    fn bodies_are_cut_and_put_back_together_byte_for_byte() {
        for size in [1, BULK_PART_BYTES, BULK_PART_BYTES + 1, 300 * 1024, MAX_BULK_BYTES] {
            let body: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
            let (mut a, mut b) = (BulkPath::default(), BulkPath::default());
            a.push(BulkKind::Reply, 1, body.clone());
            assert!(a.pending(1));
            assert_eq!(transfer(&mut a, &mut b), [body], "{size} bytes");
            assert!(!a.pending(1) && !a.has_work());
            assert_eq!(a.in_flight(), 0);
        }
    }

    #[test]
    fn the_window_holds_four_parts_and_acks_come_every_two() {
        let (mut a, mut b) = (BulkPath::default(), BulkPath::default());
        a.push(BulkKind::Upload, 1, vec![7; BULK_PART_BYTES * 10]);
        let parts: Vec<_> = std::iter::from_fn(|| a.next_part(64)).collect();
        assert_eq!(parts.len(), 4, "the window");
        assert_eq!(a.in_flight(), 4);
        let mut acks = Vec::new();
        for msg in parts {
            let (seq, last, bytes) = part(msg);
            acks.extend(b.on_part(seq, last, &bytes).ack);
        }
        assert_eq!(acks, [2, 4]);
        a.on_ack(2);
        assert_eq!(std::iter::from_fn(|| a.next_part(64)).count(), 2, "two places freed");
        a.on_ack(99);
        assert_eq!(a.in_flight(), 0, "an ack never counts past what was sent");
        // A last part is confirmed at once.
        let (mut c, mut d) = (BulkPath::default(), BulkPath::default());
        c.push(BulkKind::Reply, 2, vec![1; 10]);
        let (seq, last, bytes) = part(c.next_part(64).unwrap());
        assert_eq!(d.on_part(seq, last, &bytes).ack, Some(1));
    }

    #[test]
    fn a_busy_link_holds_the_parts_back() {
        let mut a = BulkPath::default();
        a.push(BulkKind::Reply, 1, vec![0; 10]);
        assert!(a.next_part(LINK_RESERVE).is_none(), "the link keeps room for other messages");
        assert!(a.next_part(LINK_RESERVE + 1).is_some());
    }

    #[test]
    fn a_gap_or_an_oversized_body_is_dropped_and_the_next_body_starts_clean() {
        let mut b = BulkPath::default();
        assert_eq!(b.on_part(0, false, &[1]).body, None);
        let skipped = b.on_part(2, true, &[3]);
        assert_eq!(skipped.body, None, "part 1 is missing");
        assert_eq!(skipped.ack, Some(2), "dropped parts still count");
        assert_eq!(b.on_part(3, true, &[4]).body, None, "nothing until the next seq 0");
        assert_eq!(b.on_part(0, true, &[5]).body, Some(vec![5]));
        // Larger than a body may be.
        let mut c = BulkPath::default();
        let big = vec![0; BULK_PART_BYTES];
        let mut seq = 0;
        let mut out = None;
        while seq as usize * BULK_PART_BYTES <= MAX_BULK_BYTES {
            out = c.on_part(seq, false, &big).body;
            seq += 1;
        }
        assert!(out.is_none());
        assert_eq!(c.on_part(seq, true, &big).body, None, "the body was dropped");
    }

    #[test]
    fn a_newer_reply_replaces_the_older_one_halfway() {
        let (mut a, mut b) = (BulkPath::default(), BulkPath::default());
        a.push(BulkKind::Reply, 1, vec![1; BULK_PART_BYTES * 3]);
        a.push(BulkKind::Upload, 2, vec![2; 5]);
        let (seq, last, bytes) = part(a.next_part(64).unwrap());
        assert!(b.on_part(seq, last, &bytes).body.is_none());
        a.cancel(BulkKind::Reply);
        assert!(!a.pending(1) && a.pending(2), "only replies go");
        a.push(BulkKind::Reply, 3, vec![3; 5]);
        a.on_ack(1);
        assert_eq!(transfer(&mut a, &mut b), [vec![2; 5], vec![3; 5]], "the old body is never finished");
    }

    #[test]
    fn other_messages_between_parts_change_nothing_and_a_new_session_starts_at_zero() {
        let (mut a, mut b) = (BulkPath::default(), BulkPath::default());
        a.push(BulkKind::Reply, 1, vec![9; BULK_PART_BYTES * 2]);
        let first = part(a.next_part(64).unwrap());
        // (a ping, a take's audio … go by here: they are not parts)
        let second = part(a.next_part(64).unwrap());
        assert!(b.on_part(first.0, first.1, &first.2).body.is_none());
        assert_eq!(b.on_part(second.0, second.1, &second.2).body, Some(vec![9; BULK_PART_BYTES * 2]));
        let fresh = BulkPath::default();
        assert_eq!((fresh.sent, fresh.acked, fresh.received), (0, 0, 0));
        assert!(format!("{a:?}").contains("sent: 2"));
    }
}
