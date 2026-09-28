//! The phone as the desktop's microphone (docs/dictation.md §20), both ends.
//!
//! **Desktop.** A trusted phone's [`AppMessage::TakeStart`] starts an ordinary dictation take whose
//! audio is a [`RemoteFeed`] instead of the microphone; `TakeAudio` chunks are appended to it,
//! `TakeStop` / `TakeCancel` stop or cancel the take, and every phase the take goes through is
//! reported back as a [`AppMessage::TakeStatus`]. One phone take at a time, and never while the
//! desktop runs its own take. A take whose phone went offline, or that stopped streaming for
//! [`REMOTE_QUIET_LIMIT`], is cancelled.
//!
//! **Phone.** [`CoreCommand::PhoneTakeStart`](super::CoreCommand::PhoneTakeStart) opens the
//! microphone (the dictation ports' audio source) with its 16 kHz live tap; a pump task reads the
//! tap every [`PUMP_INTERVAL`] and hands PCM16 chunks to the core, which seals them for the
//! desktop. Stop closes the capture, the pump drains what is left, and `TakeStop` follows the last
//! chunk. The desktop's statuses become `UiState.phone_take`.
//!
//! Handlers that cannot await (the message dispatch, the dictation effects) queue their messages
//! in `take_outbox`; the run loop seals and sends them after every event.

use std::time::Duration;

use parking_lot::Mutex;
use serde_bytes::ByteBuf;
use tokio::sync::mpsc;
use voltip_crypto::PublicKey;
use voltip_protocol::ProtocolVersion;
use voltip_protocol::app::{AppMessage, MAX_TAKE_AUDIO_BYTES, TAKE_SAMPLE_RATE_HZ, TakeFailure, TakeState};
use voltip_protocol::relay::RelayFrame;

use super::take_codec::{TakeDecoder, TakeEncoder};
use super::{CoreEvent, Runtime, now_ms};
use crate::CoreError;
use crate::dictation::remote::RemoteFeed;
use crate::dictation::{Capture, CaptureOptions, DictationError, DictationPhase, DictationStatus, LevelFrame, LivePcm};
use crate::peer::PeerPhase;
use crate::phone::{PhoneTakeFailure, PhoneTakeState, PhoneTakeView, take_state_for};

/// How often the phone's pump reads its live tap and sends what it found.
pub(super) const PUMP_INTERVAL: Duration = Duration::from_millis(100);
/// A phone take that sent no audio for this long while listening is cancelled on the desktop.
pub(super) const REMOTE_QUIET_LIMIT: Duration = Duration::from_secs(5);
/// Samples read from the tap per call (one [`PUMP_INTERVAL`] at 16 kHz).
const PUMP_READ: usize = 1600;

/// What the phone's background tasks report to the core.
pub(super) enum PhoneEvent {
    /// The microphone opened (or did not) for `take`.
    Opened { take: u32, result: Result<Box<dyn Capture>, DictationError> },
    /// PCM16 LE mono read from the tap.
    Chunk { take: u32, pcm: Vec<u8> },
    /// The capture stopped and the tap is empty: every chunk went out.
    Drained { take: u32 },
    /// Desktop: the injector is done with a phone's text (docs/dictation.md §20.6).
    TextDelivered { text: Box<super::texts::IncomingText>, result: Result<crate::dictation::Injection, DictationError> },
}

/// Desktop: the take a paired phone streams.
pub(super) struct RemoteTake {
    peer: PublicKey,
    take: u32,
    /// The engine session it runs as.
    session: u64,
    feed: RemoteFeed,
    /// The phone's name, stamped on the take's statuses.
    name: String,
    /// The last state sent, so repeated `Listening` statuses (live text, `ready`) go out once.
    reported: Option<TakeState>,
    /// Decodes the phone's `take_opus` chunks, from the first one on.
    opus: Option<TakeDecoder>,
}

/// Phone: the take this phone streams to a desktop.
pub(super) struct PhoneTake {
    view: PhoneTakeView,
    to: PublicKey,
    /// Behind a mutex only so the runtime stays `Sync` (a capture is `Send`, not `Sync`).
    capture: Mutex<Option<Box<dyn Capture>>>,
    pump: Option<tokio::task::JoinHandle<()>>,
    seq: u32,
    /// Stop was asked while the microphone was still opening.
    stop_wanted: bool,
    /// Encodes the chunks as Opus once the desktop said it decodes them (docs/dictation.md §20.1);
    /// until then, and for a desktop that never says so, the chunks go out as PCM.
    opus: Option<TakeEncoder>,
}

impl PhoneTake {
    fn take(&self) -> u32 {
        self.view.take
    }

    fn running(&self) -> bool {
        !self.view.state.is_final()
    }

    /// Close the microphone and the pump; the audio is discarded (it went out in chunks already).
    fn release(&mut self) {
        if let Some(pump) = self.pump.take() {
            pump.abort();
        }
        if let Some(capture) = self.capture.get_mut().take() {
            stop_capture(capture);
        }
    }
}

/// Stop a capture off the core task (the device close may block) and drop its recording.
fn stop_capture(capture: Box<dyn Capture>) {
    tokio::task::spawn_blocking(move || {
        if let Err(e) = capture.stop() {
            tracing::debug!(error = %e, "phone take capture stop failed");
        }
    });
}

fn to_pcm16(samples: &[f32], out: &mut Vec<u8>) {
    for &s in samples {
        // Full scale is ±1.0; the cast saturates.
        let v = (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
}

/// Read the tap every [`PUMP_INTERVAL`] and hand the PCM to the core until the capture stopped
/// and the tap is empty.
async fn pump(take: u32, mut live: Box<dyn LivePcm>, tx: mpsc::Sender<PhoneEvent>) {
    let mut ticker = tokio::time::interval(PUMP_INTERVAL);
    let mut buf = vec![0.0f32; PUMP_READ];
    loop {
        ticker.tick().await;
        // Read `closed` before draining: a tap that closes mid-read is drained on the next round.
        let closed = live.is_closed();
        let mut pcm = Vec::new();
        loop {
            let n = live.read(&mut buf);
            if n == 0 {
                break;
            }
            to_pcm16(&buf[..n], &mut pcm);
            if pcm.len() + PUMP_READ * 2 > MAX_TAKE_AUDIO_BYTES && tx.send(PhoneEvent::Chunk { take, pcm: std::mem::take(&mut pcm) }).await.is_err() {
                return;
            }
        }
        if !pcm.is_empty() && tx.send(PhoneEvent::Chunk { take, pcm }).await.is_err() {
            return;
        }
        if closed {
            let _ = tx.send(PhoneEvent::Drained { take }).await;
            return;
        }
    }
}

impl Runtime {
    // ---------------- both ends ----------------

    /// Seal and send the queued take messages (a peer that went offline meanwhile loses them).
    pub(super) async fn flush_takes(&mut self) {
        for (peer, msg) in std::mem::take(&mut self.take_outbox) {
            if let Err(e) = self.send_app(peer, &msg).await {
                tracing::debug!(peer = %peer.fingerprint(), error = %e, "take message not sent");
            }
        }
    }

    /// Seal `msg` for `to` on its best secure path and send it.
    pub(super) async fn send_app(&mut self, to: PublicKey, msg: &AppMessage) -> Result<(), CoreError> {
        let Some(st) = self.peers.get_mut(&to) else { return Err(CoreError::Invalid("unknown device".into())) };
        let Some(path) = st.best_secure_path() else { return Err(CoreError::Invalid("device is not online".into())) };
        let (PeerPhase::Secure(sc), Some(sid), link) = (&mut path.phase, path.session_id, path.link) else {
            return Err(CoreError::Invalid("device is not online".into()));
        };
        let bytes = sc.seal(msg)?;
        self.send_on(link, RelayFrame::forward(sid, bytes)).await
    }

    pub(super) fn peer_online(&mut self, key: &PublicKey) -> bool {
        self.peers.get_mut(key).is_some_and(|st| st.best_secure_path().is_some())
    }

    fn queue_status(&mut self, to: PublicKey, take: u32, state: TakeState) {
        self.take_outbox.push((to, AppMessage::take_status(take, state)));
    }

    /// Once per tick: drop a phone take whose other end is gone or silent.
    pub(super) fn check_takes(&mut self) {
        if let Some(rt) = &self.remote_take {
            let peer = rt.peer;
            let listening = matches!(self.dictation.status().phase, DictationPhase::Listening { .. });
            let quiet = rt.feed.quiet_for() > REMOTE_QUIET_LIMIT;
            if listening && (quiet || !self.peer_online(&peer)) {
                tracing::warn!(peer = %peer.fingerprint(), quiet, "phone take abandoned; cancelling");
                match self.dictation.cancel() {
                    Ok(effects) => self.apply_dictation(effects),
                    Err(e) => tracing::debug!(error = %e, "phone take cancel refused"),
                }
            }
        }
        let offline = self.phone_take.as_ref().filter(|t| t.running()).map(|t| t.to).is_some_and(|to| !self.peer_online(&to));
        if offline && let Some(t) = &mut self.phone_take {
            tracing::warn!("the desktop went offline during the phone take");
            t.release();
            t.view.state = PhoneTakeState::Failed { code: PhoneTakeFailure::Offline, message: "电脑已离线".into() };
            self.emit_phone_take();
        }
    }

    // ---------------- desktop ----------------

    /// A trusted phone wants to be the microphone of a take.
    pub(super) fn on_take_start(&mut self, peer: PublicKey, take: u32) {
        if !self.config.accepts_phone_takes {
            self.queue_status(peer, take, TakeState::failed(TakeFailure::Unavailable, "这台设备不接收手机的录音"));
            return;
        }
        if let Some(rt) = &self.remote_take {
            if rt.peer == peer && rt.take == take {
                return; // the same start again (a second path)
            }
            self.queue_status(peer, take, TakeState::failed(TakeFailure::Busy, "另一部手机正在录音"));
            return;
        }
        // The same readiness as a hotkey take: no recogniser, no take (the phone says why).
        if let Err(e) = self.check_recogniser() {
            let message = match e {
                CoreError::Dictation(e) => e.to_string(),
                e => e.to_string(),
            };
            self.queue_status(peer, take, TakeState::failed(TakeFailure::Unavailable, &message));
            return;
        }
        let feed = RemoteFeed::new();
        match self.dictation.start_from(feed.source()) {
            Ok(effects) => {
                let name = self.trusted.get_by_key(&peer).map_or_else(|| peer.fingerprint(), |d| d.name);
                tracing::info!(phone = %name, take, "phone take started");
                self.remote_take = Some(RemoteTake { peer, take, session: self.dictation.status().session, feed, name, reported: None, opus: None });
                self.apply_dictation(effects);
            }
            Err(DictationError::Busy) => self.queue_status(peer, take, TakeState::failed(TakeFailure::Busy, "电脑正在听写")),
            Err(e) => self.queue_status(peer, take, TakeState::failed(TakeFailure::Failed, &e.to_string())),
        }
    }

    /// The phone streaming the current take, if one is (its history entry is the phone's).
    pub(super) fn remote_take_name(&self) -> Option<String> {
        self.remote_take.as_ref().map(|rt| rt.name.clone())
    }

    fn remote_take_of(&self, peer: PublicKey, take: u32) -> Option<&RemoteTake> {
        self.remote_take.as_ref().filter(|rt| rt.peer == peer && rt.take == take)
    }

    pub(super) fn on_take_audio(&mut self, peer: PublicKey, take: u32, seq: u32, pcm: &[u8]) {
        if let Some(rt) = self.remote_take_of(peer, take) {
            rt.feed.push(seq, pcm);
        }
    }

    /// Opus chunks (docs/dictation.md §20.1): decoded into the same feed, under the same `seq`.
    pub(super) fn on_take_opus(&mut self, peer: PublicKey, take: u32, seq: u32, packets: &[ByteBuf]) {
        let Some(rt) = self.remote_take.as_mut().filter(|rt| rt.peer == peer && rt.take == take) else { return };
        if rt.opus.is_none() {
            match TakeDecoder::new() {
                Ok(decoder) => rt.opus = Some(decoder),
                Err(e) => {
                    tracing::warn!(error = %e, "phone take: no Opus decoder");
                    return;
                }
            }
        }
        if let Some(decoder) = rt.opus.as_mut() {
            let pcm = decoder.decode(packets);
            rt.feed.push(seq, &pcm);
        }
    }

    pub(super) fn on_take_stop(&mut self, peer: PublicKey, take: u32) {
        if self.remote_take_of(peer, take).is_none() {
            return;
        }
        match self.dictation.stop() {
            Ok(effects) => self.apply_dictation(effects),
            Err(e) => tracing::debug!(error = %e, "phone take stop refused"),
        }
    }

    pub(super) fn on_take_cancel(&mut self, peer: PublicKey, take: u32) {
        if self.remote_take_of(peer, take).is_none() {
            return;
        }
        match self.dictation.cancel() {
            Ok(effects) => self.apply_dictation(effects),
            Err(e) => tracing::debug!(error = %e, "phone take cancel refused"),
        }
    }

    /// Every dictation status on its way out: a phone take's is stamped with the phone's name and
    /// reported to it (each state once); `Idle` ends the phone take.
    pub(super) fn follow_remote_take(&mut self, status: &mut DictationStatus) {
        let Some(rt) = &mut self.remote_take else { return };
        if status.session != rt.session {
            return;
        }
        let Some(state) = take_state_for(&status.phase) else {
            self.remote_take = None;
            return;
        };
        status.remote = Some(rt.name.clone());
        if rt.reported.as_ref() != Some(&state) {
            let (peer, take) = (rt.peer, rt.take);
            rt.reported = Some(state.clone());
            self.queue_status(peer, take, state);
        }
    }

    // ---------------- phone ----------------

    pub(super) fn emit_phone_take(&mut self) {
        let view = self.phone_take.as_ref().map(|t| t.view.clone());
        self.emit(CoreEvent::PhoneTake(view));
    }

    /// `PhoneTakeStart`: announce the take to `to` and open the microphone.
    pub(super) fn phone_take_start(&mut self, to: PublicKey) -> Result<(), CoreError> {
        if self.phone_take.as_ref().is_some_and(PhoneTake::running) {
            return Err(CoreError::Invalid("phone take: 已有一次录音在进行".into()));
        }
        if self.trusted.get_by_key(&to).is_none() {
            return Err(CoreError::Invalid("unknown device".into()));
        }
        if !self.peer_online(&to) {
            return Err(CoreError::Invalid("device is not online".into()));
        }
        self.next_phone_take = self.next_phone_take.wrapping_add(1);
        let take = self.next_phone_take;
        self.take_outbox.push((to, AppMessage::TakeStart { version: ProtocolVersion::CURRENT, take, sample_rate_hz: TAKE_SAMPLE_RATE_HZ }));
        let audio = self.dictation.microphone();
        // The take's levels go where a dictation's do, so the phone shows its own meter.
        let levels = self.dictation.levels();
        let on_level: Box<dyn Fn(LevelFrame) + Send> = Box::new(move |frame| {
            let _ = levels.send(frame);
        });
        let tx = self.phone_tx.clone();
        tokio::spawn(async move {
            let opened = tokio::task::spawn_blocking(move || audio.start(None, on_level, Box::new(|| {}), CaptureOptions::LIVE)).await;
            let result = opened.unwrap_or_else(|e| Err(DictationError::Audio(format!("capture task failed: {e}"))));
            let _ = tx.send(PhoneEvent::Opened { take, result }).await;
        });
        self.phone_take = Some(PhoneTake {
            view: PhoneTakeView { device: to.to_hex(), take, started_at: now_ms(), state: PhoneTakeState::Starting, opus: false },
            to,
            capture: Mutex::new(None),
            pump: None,
            seq: 0,
            stop_wanted: false,
            opus: None,
        });
        self.emit_phone_take();
        Ok(())
    }

    /// `PhoneTakeStop`: close the microphone; `TakeStop` follows the last chunk.
    pub(super) fn phone_take_stop(&mut self) -> Result<(), CoreError> {
        let Some(t) = self.phone_take.as_mut().filter(|t| t.running()) else { return Err(CoreError::Invalid("phone take: 没有进行中的录音".into())) };
        match t.capture.get_mut().take() {
            Some(capture) => stop_capture(capture),
            None => t.stop_wanted = true,
        }
        Ok(())
    }

    /// `PhoneTakeCancel`: discard the take on both ends.
    pub(super) fn phone_take_cancel(&mut self) -> Result<(), CoreError> {
        let Some(t) = self.phone_take.as_mut().filter(|t| t.running()) else { return Err(CoreError::Invalid("phone take: 没有进行中的录音".into())) };
        t.release();
        t.view.state = PhoneTakeState::Cancelled;
        let (to, take) = (t.to, t.take());
        self.take_outbox.push((to, AppMessage::TakeCancel { version: ProtocolVersion::CURRENT, take }));
        self.emit_phone_take();
        Ok(())
    }

    pub(super) fn on_phone_event(&mut self, event: PhoneEvent) {
        match event {
            PhoneEvent::TextDelivered { text, result } => self.on_text_delivered(*text, result),
            PhoneEvent::Opened { take, result } => self.on_phone_opened(take, result),
            PhoneEvent::Chunk { take, pcm } => {
                let Some(t) = self.phone_take.as_mut().filter(|t| t.take() == take && t.running()) else { return };
                let to = t.to;
                let msg = match t.opus.as_mut().map(|encoder| encoder.push(&pcm)) {
                    None => Some(AppMessage::TakeAudio { version: ProtocolVersion::CURRENT, take, seq: t.seq, pcm }),
                    // Less than a frame so far: it goes out with the next chunk.
                    Some(Ok(packets)) if packets.is_empty() => None,
                    Some(Ok(packets)) => Some(AppMessage::TakeOpus { version: ProtocolVersion::CURRENT, take, seq: t.seq, packets }),
                    Some(Err(e)) => {
                        // The encoder broke: the rest of the take goes out as PCM.
                        tracing::warn!(error = %e, "phone take: Opus encoding failed; sending PCM");
                        t.opus = None;
                        t.view.opus = false;
                        Some(AppMessage::TakeAudio { version: ProtocolVersion::CURRENT, take, seq: t.seq, pcm })
                    }
                };
                if let Some(msg) = msg {
                    t.seq += 1;
                    self.take_outbox.push((to, msg));
                }
            }
            PhoneEvent::Drained { take } => {
                let Some(t) = self.phone_take.as_mut().filter(|t| t.take() == take && t.running()) else { return };
                t.pump = None;
                let to = t.to;
                // The encoder's last partial frame goes out before the stop.
                match t.opus.as_mut().map(TakeEncoder::finish) {
                    Some(Ok(packets)) if !packets.is_empty() => {
                        self.take_outbox.push((to, AppMessage::TakeOpus { version: ProtocolVersion::CURRENT, take, seq: t.seq, packets }));
                        t.seq += 1;
                    }
                    Some(Err(e)) => tracing::warn!(error = %e, "phone take: the last Opus frame was lost"),
                    _ => {}
                }
                self.take_outbox.push((to, AppMessage::TakeStop { version: ProtocolVersion::CURRENT, take }));
            }
        }
    }

    fn on_phone_opened(&mut self, take: u32, result: Result<Box<dyn Capture>, DictationError>) {
        let current = self.phone_take.as_mut().filter(|t| t.take() == take && t.running());
        let Some(t) = current else {
            // Cancelled (or superseded) while the microphone was opening.
            if let Ok(capture) = result {
                stop_capture(capture);
            }
            return;
        };
        let (to, stop_wanted) = (t.to, t.stop_wanted);
        let mut capture = match result {
            Ok(capture) => capture,
            Err(e) => {
                tracing::warn!(error = %e, "phone take: the microphone did not open");
                t.view.state = PhoneTakeState::Failed { code: PhoneTakeFailure::Microphone, message: e.to_string() };
                self.take_outbox.push((to, AppMessage::TakeCancel { version: ProtocolVersion::CURRENT, take }));
                self.emit_phone_take();
                return;
            }
        };
        let Some(live) = capture.live_pcm() else {
            stop_capture(capture);
            t.view.state = PhoneTakeState::Failed { code: PhoneTakeFailure::Microphone, message: "microphone: 没有实时音频".into() };
            self.take_outbox.push((to, AppMessage::TakeCancel { version: ProtocolVersion::CURRENT, take }));
            self.emit_phone_take();
            return;
        };
        t.pump = Some(tokio::spawn(pump(take, live, self.phone_tx.clone())));
        if stop_wanted {
            stop_capture(capture);
        } else {
            *t.capture.get_mut() = Some(capture);
        }
        tracing::info!(take, "phone take streaming");
    }

    /// The desktop reported where our take is; `opus` = it decodes Opus, so the rest of the take
    /// goes out as Opus packets (docs/dictation.md §20.1).
    pub(super) fn on_take_status(&mut self, from: PublicKey, take: u32, state: TakeState, opus: bool) {
        let Some(t) = self.phone_take.as_mut().filter(|t| t.to == from && t.take() == take) else { return };
        if opus && t.opus.is_none() && t.running() {
            match TakeEncoder::new() {
                Ok(encoder) => {
                    tracing::info!(take, "phone take: the desktop decodes Opus; switching from PCM");
                    t.opus = Some(encoder);
                    t.view.opus = true;
                }
                Err(e) => tracing::warn!(error = %e, "phone take: no Opus encoder; staying on PCM"),
            }
        }
        let state = PhoneTakeState::from(state);
        if state.is_final() {
            // The desktop is done with it (delivered, refused, cancelled): nothing more to stream.
            t.release();
        } else if !t.running() {
            return; // a late `listening` after the phone cancelled
        }
        t.view.state = state;
        self.emit_phone_take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm16_is_little_endian_and_saturates() {
        let mut out = Vec::new();
        to_pcm16(&[0.0, 1.0, -1.0, 2.0, 0.5], &mut out);
        let samples: Vec<i16> = out.chunks(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
        assert_eq!(samples, [0, i16::MAX, -i16::MAX, i16::MAX, i16::MAX / 2]);
    }
}
