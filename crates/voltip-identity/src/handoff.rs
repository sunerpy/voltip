//! Handing the secrets a running build holds to the build that replaces it (macOS in-app update;
//! docs/runbook.md 发布 · macOS 签名与钥匙串, [`crate::PerBuildStore`]).
//!
//! The old build starts the new one with one end of a socket pair as its stdin, sends what its
//! keychain store knows ([`Entries`]) and waits for one byte back. The new build reads it at the
//! very start of the process, before anything else runs. Each side first checks that the other
//! process is this app, signed as this build is ([`PeerCheck`]), so the secrets only ever go from
//! one release to the next. Nothing is written to disk, and nothing goes through the command line
//! or the environment.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use zeroize::Zeroizing;

use crate::Entries;

/// Environment variable that tells a new build to read a hand-over from its stdin.
pub const HANDOFF_ENV: &str = "VOLTIP_SECRET_HANDOFF";

const MAGIC: &[u8; 8] = b"VOLTIPH1";
const ACK: u8 = 0x06;
/// Limits far above what a hand-over holds (a few entries of a few hundred bytes).
const MAX_ENTRIES: usize = 256;
const MAX_NAME: usize = 256;
const MAX_VALUE: usize = 64 * 1024;
const MAX_PAYLOAD: usize = 1024 * 1024;

/// Why a hand-over was not taken or not delivered.
#[derive(Debug, thiserror::Error)]
pub enum HandoffError {
    /// The process at the other end is not this app, signed as this build is.
    #[error("the other process is not this app, signed as this build is")]
    NotTrusted,
    /// The bytes are not a hand-over this build reads.
    #[error("malformed hand-over: {0}")]
    Malformed(&'static str),
    /// The socket failed or timed out.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Checks the process at the other end of a hand-over.
pub trait PeerCheck {
    /// `true` when process `pid` is this app, signed as this build is.
    fn trusted(&self, pid: u32) -> bool;
}

/// The wire form of `entries` (without the length prefix [`send`] adds).
pub fn encode(entries: &Entries) -> Result<Zeroizing<Vec<u8>>, HandoffError> {
    if entries.len() > MAX_ENTRIES {
        return Err(HandoffError::Malformed("too many entries"));
    }
    let mut out = Zeroizing::new(Vec::with_capacity(64));
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&u32::try_from(entries.len()).map_err(|_| HandoffError::Malformed("too many entries"))?.to_be_bytes());
    for (name, value) in entries {
        if name.is_empty() || name.len() > MAX_NAME {
            return Err(HandoffError::Malformed("entry name"));
        }
        out.extend_from_slice(&u16::try_from(name.len()).map_err(|_| HandoffError::Malformed("entry name"))?.to_be_bytes());
        out.extend_from_slice(name.as_bytes());
        match value {
            Some(value) => {
                if value.len() > MAX_VALUE {
                    return Err(HandoffError::Malformed("entry value"));
                }
                out.push(1);
                out.extend_from_slice(&u32::try_from(value.len()).map_err(|_| HandoffError::Malformed("entry value"))?.to_be_bytes());
                out.extend_from_slice(value);
            }
            None => out.push(0),
        }
    }
    Ok(out)
}

/// Reads the wire form back; anything short, long or out of bounds is refused whole.
pub fn decode(bytes: &[u8]) -> Result<Entries, HandoffError> {
    let mut rest = bytes;
    if take(&mut rest, MAGIC.len())? != MAGIC {
        return Err(HandoffError::Malformed("not a hand-over"));
    }
    let count = u32::from_be_bytes(array(take(&mut rest, 4)?)) as usize;
    if count > MAX_ENTRIES {
        return Err(HandoffError::Malformed("too many entries"));
    }
    let mut entries = Entries::with_capacity(count);
    for _ in 0..count {
        let len = usize::from(u16::from_be_bytes(array(take(&mut rest, 2)?)));
        if len == 0 || len > MAX_NAME {
            return Err(HandoffError::Malformed("entry name"));
        }
        let name = std::str::from_utf8(take(&mut rest, len)?).map_err(|_| HandoffError::Malformed("entry name"))?.to_owned();
        let value = match take(&mut rest, 1)?[0] {
            0 => None,
            1 => {
                let len = u32::from_be_bytes(array(take(&mut rest, 4)?)) as usize;
                if len > MAX_VALUE {
                    return Err(HandoffError::Malformed("entry value"));
                }
                Some(Zeroizing::new(take(&mut rest, len)?.to_vec()))
            }
            _ => return Err(HandoffError::Malformed("entry state")),
        };
        entries.push((name, value));
    }
    if !rest.is_empty() {
        return Err(HandoffError::Malformed("trailing bytes"));
    }
    Ok(entries)
}

fn take<'a>(rest: &mut &'a [u8], n: usize) -> Result<&'a [u8], HandoffError> {
    if rest.len() < n {
        return Err(HandoffError::Malformed("truncated"));
    }
    let (head, tail) = rest.split_at(n);
    *rest = tail;
    Ok(head)
}

fn array<const N: usize>(bytes: &[u8]) -> [u8; N] {
    let mut out = [0u8; N];
    out.copy_from_slice(bytes);
    out
}

/// The old build's side: check the new build's process `peer`, send `entries` and wait up to
/// `timeout` for its acknowledgement.
pub fn send(stream: &mut UnixStream, entries: &Entries, check: &dyn PeerCheck, peer: u32, timeout: Duration) -> Result<(), HandoffError> {
    if !check.trusted(peer) {
        return Err(HandoffError::NotTrusted);
    }
    let payload = encode(entries)?;
    stream.set_write_timeout(Some(timeout))?;
    stream.set_read_timeout(Some(timeout))?;
    let len = u32::try_from(payload.len()).map_err(|_| HandoffError::Malformed("too long"))?;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(&payload)?;
    let mut ack = [0u8; 1];
    stream.read_exact(&mut ack)?;
    if ack[0] != ACK {
        return Err(HandoffError::Malformed("acknowledgement"));
    }
    Ok(())
}

/// The new build's side: check the old build's process `peer` (the parent), read the hand-over
/// and acknowledge it. Nothing is read from a peer that fails the check.
pub fn receive(stream: &mut UnixStream, check: &dyn PeerCheck, peer: u32, timeout: Duration) -> Result<Entries, HandoffError> {
    if !check.trusted(peer) {
        return Err(HandoffError::NotTrusted);
    }
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let mut len = [0u8; 4];
    stream.read_exact(&mut len)?;
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_PAYLOAD {
        return Err(HandoffError::Malformed("too long"));
    }
    let mut payload = Zeroizing::new(vec![0u8; len]);
    stream.read_exact(&mut payload)?;
    let entries = decode(&payload)?;
    stream.write_all(&[ACK])?;
    Ok(entries)
}

/// Why [`start`] did not hand over.
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    /// The new build was not started at all.
    #[error("the new build could not be started: {0}")]
    NotStarted(std::io::Error),
    /// The new build runs, but without the hand-over (it reads an older build's items once).
    #[error("the new build started without the hand-over: {0}")]
    WithoutHandOver(HandoffError),
}

/// The old build's side of an update: start `exe` with `args`, a socket as its stdin and
/// [`HANDOFF_ENV`] set, and hand it `entries` once `check` trusts it.
pub fn start(exe: &Path, args: impl IntoIterator<Item = OsString>, entries: &Entries, check: &dyn PeerCheck, timeout: Duration) -> Result<Child, StartError> {
    let (mut ours, theirs) = UnixStream::pair().map_err(StartError::NotStarted)?;
    let child = Command::new(exe).args(args).env(HANDOFF_ENV, "1").stdin(Stdio::from(OwnedFd::from(theirs))).spawn().map_err(StartError::NotStarted)?;
    match send(&mut ours, entries, check, child.id(), timeout) {
        Ok(()) => Ok(child),
        Err(e) => Err(StartError::WithoutHandOver(e)),
    }
}

/// The new build's side, given that [`HANDOFF_ENV`] asked for it: read the hand-over from stdin
/// (the socket [`start`] passed) once `check` trusts the parent.
pub fn take_from_stdin(check: &dyn PeerCheck, timeout: Duration) -> Result<Entries, HandoffError> {
    let mut stream = UnixStream::from(std::io::stdin().as_fd().try_clone_to_owned()?);
    receive(&mut stream, check, std::os::unix::process::parent_id(), timeout)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHORT: Duration = Duration::from_millis(300);
    const LONG: Duration = Duration::from_secs(10);

    struct Pids(Vec<u32>);

    impl PeerCheck for Pids {
        fn trusted(&self, pid: u32) -> bool {
            self.0.contains(&pid)
        }
    }

    fn entries() -> Entries {
        vec![
            ("voltip.identity.meta".to_owned(), Some(Zeroizing::new(br#"{"name":"Mac"}"#.to_vec()))),
            ("voltip.identity.x25519".to_owned(), Some(Zeroizing::new(vec![7u8; 32]))),
            ("voltip.provider.openai.llm".to_owned(), None),
        ]
    }

    fn same(a: &Entries, b: &Entries) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.0 == y.0 && x.1.as_deref() == y.1.as_deref())
    }

    #[test]
    fn entries_and_absent_entries_travel_intact() {
        let (mut old, mut new) = UnixStream::pair().unwrap();
        let sent = entries();
        let sender = std::thread::spawn(move || send(&mut old, &sent, &Pids(vec![2]), 2, LONG));
        let got = receive(&mut new, &Pids(vec![1]), 1, LONG).unwrap();
        sender.join().unwrap().unwrap();
        assert!(same(&got, &entries()), "{:?}", got.iter().map(|e| &e.0).collect::<Vec<_>>());
    }

    /// The new build takes nothing from a parent that is not this app, signed as it is, and so
    /// never acknowledges: the sender gives up after its timeout.
    #[test]
    fn a_parent_that_fails_the_check_hands_over_nothing() {
        let (mut old, mut new) = UnixStream::pair().unwrap();
        let sent = entries();
        let sender = std::thread::spawn(move || send(&mut old, &sent, &Pids(vec![2]), 2, SHORT));
        assert!(matches!(receive(&mut new, &Pids(vec![]), 1, LONG), Err(HandoffError::NotTrusted)));
        assert!(matches!(sender.join().unwrap(), Err(HandoffError::Io(_))), "no acknowledgement");
    }

    /// The old build sends nothing to a new process that is not this app, signed as it is.
    #[test]
    fn a_child_that_fails_the_check_is_sent_nothing() {
        let (mut old, mut new) = UnixStream::pair().unwrap();
        assert!(matches!(send(&mut old, &entries(), &Pids(vec![]), 2, SHORT), Err(HandoffError::NotTrusted)));
        drop(old);
        let mut rest = Vec::new();
        new.read_to_end(&mut rest).unwrap();
        assert!(rest.is_empty(), "nothing was written");
    }

    #[test]
    fn malformed_hand_overs_are_refused_whole() {
        let good = encode(&entries()).unwrap();
        assert!(same(&decode(&good).unwrap(), &entries()));
        assert!(matches!(decode(&good[..good.len() - 1]), Err(HandoffError::Malformed("truncated"))));
        let mut long = good.to_vec();
        long.push(0);
        assert!(matches!(decode(&long), Err(HandoffError::Malformed("trailing bytes"))));
        let mut magic = good.to_vec();
        magic[0] = b'X';
        assert!(matches!(decode(&magic), Err(HandoffError::Malformed("not a hand-over"))));
        let mut state = encode(&vec![("k".to_owned(), None)]).unwrap().to_vec();
        *state.last_mut().unwrap() = 9;
        assert!(matches!(decode(&state), Err(HandoffError::Malformed("entry state"))));
        assert!(matches!(encode(&vec![(String::new(), None)]), Err(HandoffError::Malformed("entry name"))));
        assert!(matches!(encode(&vec![("k".to_owned(), Some(Zeroizing::new(vec![0; MAX_VALUE + 1])))]), Err(HandoffError::Malformed("entry value"))));
        let many: Entries = (0..=MAX_ENTRIES).map(|i| (format!("k{i}"), None)).collect();
        assert!(matches!(encode(&many), Err(HandoffError::Malformed("too many entries"))));
        let mut count = good.to_vec();
        count[8..12].copy_from_slice(&u32::try_from(MAX_ENTRIES + 1).unwrap().to_be_bytes());
        assert!(matches!(decode(&count), Err(HandoffError::Malformed("too many entries"))));
    }

    #[test]
    fn a_hand_over_longer_than_the_limit_is_not_read() {
        let (mut old, mut new) = UnixStream::pair().unwrap();
        old.write_all(&u32::try_from(MAX_PAYLOAD + 1).unwrap().to_be_bytes()).unwrap();
        assert!(matches!(receive(&mut new, &Pids(vec![1]), 1, SHORT), Err(HandoffError::Malformed("too long"))));
    }

    /// `start` runs the new build with the socket as its stdin and the variable set; `sh` reads
    /// the hand-over and answers as a new build does.
    #[test]
    fn start_hands_the_entries_to_the_process_it_starts() {
        let script = "test \"$VOLTIP_SECRET_HANDOFF\" = 1 || exit 3; head -c 4 >/dev/null; printf '\\006' >&0; cat >/dev/null";
        let mut child = start(Path::new("/bin/sh"), ["-c".into(), script.into()], &entries(), &Pids(vec![]), SHORT).map(Some).unwrap_or_else(|e| {
            assert!(matches!(e, StartError::WithoutHandOver(HandoffError::NotTrusted)), "{e}");
            None
        });
        assert!(child.is_none(), "an untrusted process is sent nothing");
        let trusted = AnyPid;
        child = Some(start(Path::new("/bin/sh"), ["-c".into(), script.into()], &entries(), &trusted, LONG).unwrap());
        assert!(child.take().unwrap().wait().unwrap().success());
        assert!(matches!(start(Path::new("/nonexistent/voltip"), Vec::<OsString>::new(), &entries(), &trusted, SHORT), Err(StartError::NotStarted(_))));
    }

    struct AnyPid;

    impl PeerCheck for AnyPid {
        fn trusted(&self, _pid: u32) -> bool {
            true
        }
    }

    #[test]
    fn a_wrong_acknowledgement_is_an_error() {
        let (mut old, mut new) = UnixStream::pair().unwrap();
        let reader = std::thread::spawn(move || {
            let mut len = [0u8; 4];
            new.read_exact(&mut len).unwrap();
            let mut payload = vec![0u8; u32::from_be_bytes(len) as usize];
            new.read_exact(&mut payload).unwrap();
            new.write_all(&[0]).unwrap();
        });
        assert!(matches!(send(&mut old, &entries(), &Pids(vec![2]), 2, LONG), Err(HandoffError::Malformed("acknowledgement"))));
        reader.join().unwrap();
    }
}
