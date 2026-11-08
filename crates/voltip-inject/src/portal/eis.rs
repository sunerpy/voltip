//! The EIS keyboard of a remote-desktop session (docs/dictation.md §14.9): the libei protocol over
//! the socket `ConnectToEIS` hands out, through `reis`, as a sender context. Bind the seat's
//! keyboard, wait until the compositor has resumed it and announced its keymap and active layout,
//! press the chord in one emulation sequence, and wait for the compositor to confirm it handled
//! every key (`ei_connection.sync`) before disconnecting: the session is closed right after, and a
//! key the compositor had not read yet would be lost.

use std::cell::Cell;
use std::os::fd::OwnedFd;
use std::os::unix::fs::FileExt as _;
use std::os::unix::net::UnixStream;
use std::rc::Rc;
use std::time::{Duration, Instant};

use reis::PendingRequestResult;
use reis::ei;
use reis::event::{DeviceCapability, EiEvent, EiEventConverter};
use reis::handshake::{EiHandshaker, HandshakeResp};
use rustix::event::{PollFd, PollFlags};

use super::keymap::{KeyEvent, KeyLookup};
use crate::platform::KeyStep;

/// The client name the compositor sees.
const NAME: &str = "Voltip";
/// A keymap larger than this is not read (an XKB keymap is some 60 KiB): the US positions then.
const MAX_KEYMAP: u32 = 4 << 20;

/// How far the chord got when the keyboard failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum EisError {
    /// Nothing was pressed.
    Before(String),
    /// Keys went out: some of the chord may have reached the application.
    After(String),
}

/// Press `steps` on the keyboard behind `fd`, within `limit` for everything.
pub(super) fn press(fd: OwnedFd, steps: &[KeyStep], limit: Duration) -> Result<(), EisError> {
    let deadline = Instant::now() + limit;
    let context = ei::Context::new(UnixStream::from(fd)).map_err(|e| EisError::Before(format!("EIS socket: {e}")))?;
    let resp = handshake(&context, deadline).map_err(EisError::Before)?;
    let mut client = Client::new(&context, resp, deadline);
    let (keyboard, lookup) = client.keyboard().map_err(EisError::Before)?;
    let events = lookup.plan(steps).map_err(EisError::Before)?;
    client.emulate(&keyboard, &events)?;
    client.sync(|_| Ok(())).map_err(EisError::After)?;
    client.connection().disconnect();
    // The keys are in: a disconnect that does not go out is the compositor's to clean up.
    let _ = client.flush();
    Ok(())
}

/// Wait until `fd` is ready for `flags`, at most until `deadline`.
fn wait(fd: &ei::Context, flags: PollFlags, deadline: Instant) -> Result<(), String> {
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err("the compositor's keyboard did not answer in time".into());
        }
        let timeout = rustix::time::Timespec { tv_sec: i64::try_from(left.as_secs()).unwrap_or(i64::MAX), tv_nsec: i64::from(left.subsec_nanos()) };
        match rustix::event::poll(&mut [PollFd::new(fd, flags)], Some(&timeout)) {
            Ok(0) | Err(rustix::io::Errno::INTR) => {}
            Ok(_) => return Ok(()),
            Err(e) => return Err(format!("EIS socket: {e}")),
        }
    }
}

/// Read what arrived; the compositor hanging up is an error.
fn read(context: &ei::Context) -> Result<(), String> {
    match context.read() {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Err("the compositor closed the keyboard".into()),
        Err(e) => Err(format!("EIS socket: {e}")),
    }
}

/// The ei handshake: our name, a sender context, every interface reis speaks.
fn handshake(context: &ei::Context, deadline: Instant) -> Result<HandshakeResp, String> {
    let mut handshaker = EiHandshaker::new(NAME, ei::handshake::ContextType::Sender);
    loop {
        while let Some(pending) = context.pending_event() {
            let PendingRequestResult::Request(event) = pending else {
                return Err(format!("bad EIS handshake message: {pending:?}"));
            };
            if let Some(resp) = handshaker.handle_event(event).map_err(|e| format!("EIS handshake: {e}"))? {
                return Ok(resp);
            }
        }
        wait(context, PollFlags::IN, deadline)?;
        read(context)?;
    }
}

/// The connection after the handshake.
struct Client<'a> {
    context: &'a ei::Context,
    converter: EiEventConverter,
    deadline: Instant,
}

/// The keyboard as the compositor set it up.
struct Keyboard {
    device: ei::Device,
    keys: ei::Keyboard,
}

impl<'a> Client<'a> {
    fn new(context: &'a ei::Context, resp: HandshakeResp, deadline: Instant) -> Self {
        Self { context, converter: EiEventConverter::new(context, resp), deadline }
    }

    fn connection(&self) -> &ei::Connection {
        self.converter.connection().connection()
    }

    fn serial(&self) -> u32 {
        self.converter.connection().serial()
    }

    /// Send what is buffered, waiting while the socket is full.
    fn flush(&self) -> Result<(), String> {
        loop {
            match self.context.flush() {
                Ok(()) => return Ok(()),
                Err(rustix::io::Errno::AGAIN | rustix::io::Errno::INTR) => wait(self.context, PollFlags::OUT, self.deadline)?,
                Err(e) => return Err(format!("EIS socket: {e}")),
            }
        }
    }

    /// One step: the next high-level event when one is ready; otherwise one message handled (a
    /// callback's `done` sets its flag and yields no event) or the socket read, and `None`.
    fn step(&mut self) -> Result<Option<EiEvent>, String> {
        if let Some(event) = self.converter.next_event() {
            return Ok(Some(event));
        }
        match self.context.pending_event() {
            Some(PendingRequestResult::Request(event)) => self.converter.handle_event(event).map_err(|e| format!("EIS protocol: {e}"))?,
            Some(other) => return Err(format!("bad EIS message: {other:?}")),
            None => {
                wait(self.context, PollFlags::IN, self.deadline)?;
                read(self.context)?;
            }
        }
        Ok(self.converter.next_event())
    }

    /// `ei_connection.sync`: returns once the compositor has handled everything sent before, and
    /// shows `on` every event that arrives meanwhile.
    fn sync(&mut self, mut on: impl FnMut(&EiEvent) -> Result<(), String>) -> Result<(), String> {
        let done = Rc::new(Cell::new(false));
        let callback = self.connection().sync(1);
        let seen = done.clone();
        self.converter.add_callback_handler(callback, move |_| seen.set(true));
        self.flush()?;
        while !done.get() {
            match self.step()? {
                Some(EiEvent::Disconnected(d)) => return Err(disconnected(&d)),
                Some(event) => on(&event)?,
                None => {}
            }
        }
        Ok(())
    }

    /// Bind the seat's keyboard and wait until the compositor has resumed it, then one more round
    /// trip so the active layout it sends after the device has arrived too.
    fn keyboard(&mut self) -> Result<(Keyboard, KeyLookup), String> {
        let mut device: Option<reis::event::Device> = None;
        let mut group = 0;
        loop {
            let Some(event) = self.step()? else { continue };
            match event {
                EiEvent::SeatAdded(added) => {
                    added.seat.bind_capabilities(DeviceCapability::Keyboard.into());
                    self.flush()?;
                }
                EiEvent::DeviceAdded(added) if added.device.has_capability(DeviceCapability::Keyboard) => device = Some(added.device),
                EiEvent::KeyboardModifiers(modifiers) if device.as_ref() == Some(&modifiers.device) => group = modifiers.group,
                EiEvent::DeviceResumed(resumed) if device.as_ref() == Some(&resumed.device) => break,
                EiEvent::DeviceRemoved(removed) if device.as_ref() == Some(&removed.device) => device = None,
                EiEvent::Disconnected(d) => return Err(disconnected(&d)),
                _ => {}
            }
        }
        // `ei_keyboard.modifiers` follows the device's resume in Mutter (send_keyboard_modifiers).
        let device = device.ok_or("the compositor offered no keyboard")?;
        self.sync(|event| match event {
            EiEvent::KeyboardModifiers(modifiers) if modifiers.device == device => {
                group = modifiers.group;
                Ok(())
            }
            EiEvent::DevicePaused(paused) if paused.device == device => Err("the compositor paused the keyboard".into()),
            EiEvent::DeviceRemoved(removed) if removed.device == device => Err("the compositor removed the keyboard".into()),
            _ => Ok(()),
        })?;
        let keys = device.interface::<ei::Keyboard>().ok_or("the keyboard device has no ei_keyboard")?;
        let lookup = match device.keymap() {
            Some(keymap) if keymap.type_ == ei::keyboard::KeymapType::Xkb => {
                match keymap_text(&keymap.fd, keymap.size).and_then(|text| KeyLookup::new(&text, group)) {
                    Some(lookup) => lookup,
                    None => {
                        tracing::warn!(size = keymap.size, "the EIS keymap could not be read; pressing US key positions");
                        KeyLookup::us()
                    }
                }
            }
            _ => KeyLookup::us(),
        };
        Ok((Keyboard { device: device.device().clone(), keys }, lookup))
    }

    /// One emulation sequence with every event of the chord, a frame after each key.
    fn emulate(&mut self, keyboard: &Keyboard, events: &[KeyEvent]) -> Result<(), EisError> {
        keyboard.device.start_emulating(self.serial(), 1);
        let mut sent = false;
        for event in events {
            match *event {
                KeyEvent::Key { code, pressed } => {
                    let state = if pressed { ei::keyboard::KeyState::Press } else { ei::keyboard::KeyState::Released };
                    keyboard.keys.key(code, state);
                    keyboard.device.frame(self.serial(), now_micros());
                    let flushed = self.flush();
                    sent = true;
                    flushed.map_err(EisError::After)?;
                }
                KeyEvent::Hold(duration) => {
                    self.flush().map_err(|e| if sent { EisError::After(e) } else { EisError::Before(e) })?;
                    // The chord's own timing (macOS steps hold a key down), not a wait for a state.
                    std::thread::sleep(duration);
                }
            }
        }
        keyboard.device.stop_emulating(self.serial());
        self.flush().map_err(EisError::After)
    }
}

fn disconnected(d: &reis::event::Disconnected) -> String {
    match &d.explanation {
        Some(explanation) => format!("the compositor disconnected the keyboard ({:?}: {explanation})", d.reason),
        None => format!("the compositor disconnected the keyboard ({:?})", d.reason),
    }
}

/// The keymap the compositor shared: `size` bytes from the start of `fd`, NUL padding dropped.
fn keymap_text(fd: &OwnedFd, size: u32) -> Option<String> {
    if size == 0 || size > MAX_KEYMAP {
        return None;
    }
    let file = std::fs::File::from(fd.try_clone().ok()?);
    let mut bytes = vec![0; usize::try_from(size).ok()?];
    file.read_exact_at(&mut bytes, 0).ok()?;
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    bytes.truncate(end);
    String::from_utf8(bytes).ok()
}

/// The monotonic clock in microseconds, the time base of `ei_device.frame`.
fn now_micros() -> u64 {
    let now = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    let secs = u64::try_from(now.tv_sec).unwrap_or(0);
    let nanos = u64::try_from(now.tv_nsec).unwrap_or(0);
    secs.saturating_mul(1_000_000).saturating_add(nanos / 1_000)
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    #[test]
    fn keymap_text_reads_the_shared_file() {
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(b"xkb_keymap {};\0\0\0").unwrap();
        let fd = OwnedFd::from(file);
        assert_eq!(keymap_text(&fd, 17).as_deref(), Some("xkb_keymap {};"));
        assert_eq!(keymap_text(&fd, 0), None, "an empty keymap");
        assert_eq!(keymap_text(&fd, 64), None, "shorter than announced");
        assert_eq!(keymap_text(&fd, MAX_KEYMAP + 1), None, "too large to read");
    }

    #[test]
    fn the_clock_moves_forward() {
        let first = now_micros();
        assert!(first > 0);
        assert!(now_micros() >= first);
    }
}
