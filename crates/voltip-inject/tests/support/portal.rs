//! A fake `xdg-desktop-portal` on a private D-Bus session bus (docs/dictation.md §14.9): the
//! `org.freedesktop.portal.RemoteDesktop` interface with its Request and Session objects, and a
//! fake EIS keyboard behind `ConnectToEIS` that records the keys it receives. The dialog of the
//! real portal is a scripted answer here; [`Record::dialogs`] counts how often the real one would
//! have asked the user.
//!
//! Needs `dbus-daemon` (Ubuntu: `dbus-daemon`) and the XKB data (`xkb-data`) the keymaps are
//! compiled from; both are on CI's Ubuntu runners.

#![allow(dead_code)] // every test file uses a part of it

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{BufRead as _, BufReader, Write as _};
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use reis::PendingRequestResult;
use reis::eis;
use reis::event::DeviceCapability;
use reis::request::EisRequest;
use xkbcommon::xkb;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

/// `types` bit of a keyboard (xdg-desktop-portal's `org.freedesktop.portal.RemoteDesktop`).
pub const KEYBOARD: u32 = 1;
/// `types` bit of a pointer.
pub const POINTER: u32 = 2;

/// How long the fake EIS keyboard waits for its client before giving up.
const EIS_PATIENCE: Duration = Duration::from_secs(30);

/// A session bus of its own: no service files, so nothing is activated behind the tests' back.
const BUS_CONFIG: &str = r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:path=SOCKET</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
"#;

/// A `dbus-daemon` that lives as long as the value.
pub struct PrivateBus {
    daemon: Child,
    /// Kept open: the daemon must not write into a closed pipe.
    _stdout: BufReader<ChildStdout>,
    /// The address to connect to (`DBUS_SESSION_BUS_ADDRESS`).
    pub address: String,
    _dir: tempfile::TempDir,
}

impl PrivateBus {
    /// Start one.
    pub fn start() -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let config = dir.path().join("session.conf");
        let socket = dir.path().join("bus");
        std::fs::write(&config, BUS_CONFIG.replace("SOCKET", &socket.display().to_string())).expect("bus config");
        let mut daemon = Command::new("dbus-daemon")
            .arg(format!("--config-file={}", config.display()))
            .args(["--nofork", "--print-address"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .expect("dbus-daemon (Ubuntu: apt install dbus-daemon)");
        let mut stdout = BufReader::new(daemon.stdout.take().expect("dbus-daemon stdout"));
        let mut address = String::new();
        stdout.read_line(&mut address).expect("dbus-daemon address");
        let address = address.trim().to_string();
        if address.is_empty() {
            let _ = daemon.kill();
            let _ = daemon.wait();
            panic!("dbus-daemon printed no address");
        }
        Self { daemon, _stdout: stdout, address, _dir: dir }
    }
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
    }
}

/// What the user answers in the dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// "Share" with "Remember This Selection" ticked (the default answer).
    Allow,
    /// "Share" with "Remember This Selection" unticked: no restore token.
    AllowOnce,
    /// "Cancel": response 1.
    Deny,
    /// The request ends some other way: response 2.
    End,
    /// Nobody answers: no response until the client closes the request.
    Ignore,
}

/// The keyboard layout the fake EIS keyboard announces: RMLVO names compiled with libxkbcommon,
/// as Mutter compiles the user's input sources, and the active group (layout index).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    /// `XKB_DEFAULT_LAYOUT`-style list (`us`, `ru,us`).
    pub layout: String,
    /// Variants, one per layout (`dvorak`, `,`).
    pub variant: String,
    /// XKB options (`ctrl:swapcaps`).
    pub options: Option<String>,
    /// The active group sent with `ei_keyboard.modifiers`.
    pub group: u32,
}

impl Layout {
    /// US QWERTY.
    pub fn us() -> Self {
        Self::names("us", "", None, 0)
    }

    /// Any layout.
    pub fn names(layout: &str, variant: &str, options: Option<&str>, group: u32) -> Self {
        Self { layout: layout.into(), variant: variant.into(), options: options.map(str::to_owned), group }
    }
}

/// One `SelectDevices` call.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Select {
    /// `types`.
    pub types: u32,
    /// `persist_mode`.
    pub persist_mode: u32,
    /// `restore_token`.
    pub restore_token: Option<String>,
}

/// Everything the fake saw.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Record {
    /// `CreateSession` calls.
    pub sessions: usize,
    /// `Session.Close` calls.
    pub closed_sessions: usize,
    /// `Start` calls that would have shown the dialog (no valid restore token).
    pub dialogs: usize,
    /// `Start` calls restored from a restore token, without a dialog.
    pub restored: usize,
    /// `Request.Close` calls (a client that stopped waiting for an answer).
    pub closed_requests: usize,
    /// Every `SelectDevices` call, in order.
    pub selects: Vec<Select>,
    /// `ConnectToEIS` calls.
    pub eis_connections: usize,
    /// The keys of each EIS connection, in order: evdev code and pressed.
    pub keys: Vec<Vec<(u32, bool)>>,
    /// Every restore token handed out, in order.
    pub issued: Vec<String>,
}

#[derive(Default)]
struct SessionState {
    select: Option<Select>,
    started: bool,
}

struct State {
    version: u32,
    device_types: u32,
    answers: VecDeque<Answer>,
    /// Restore tokens the permission store still knows; each is good for one `Start`.
    granted: HashSet<String>,
    next_token: u64,
    sessions: HashMap<String, SessionState>,
    layout: Layout,
    record: Record,
}

type Shared = Arc<Mutex<State>>;

fn lock(state: &Shared) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The fake portal: `org.freedesktop.portal.Desktop` on `bus` while the value lives.
pub struct FakePortal {
    state: Shared,
    _conn: zbus::blocking::Connection,
}

impl FakePortal {
    /// A GNOME 50 portal: RemoteDesktop version 2 with keyboard and pointer.
    pub fn start(bus: &PrivateBus) -> Self {
        Self::with(bus, 2, KEYBOARD | POINTER)
    }

    /// A portal of `version` offering `device_types`.
    pub fn with(bus: &PrivateBus, version: u32, device_types: u32) -> Self {
        let state = Arc::new(Mutex::new(State {
            version,
            device_types,
            answers: VecDeque::new(),
            granted: HashSet::new(),
            next_token: 0,
            sessions: HashMap::new(),
            layout: Layout::us(),
            record: Record::default(),
        }));
        let conn = zbus::blocking::connection::Builder::address(bus.address.as_str())
            .and_then(|b| b.name("org.freedesktop.portal.Desktop"))
            .and_then(|b| b.serve_at("/org/freedesktop/portal/desktop", RemoteDesktopIface { state: state.clone() }))
            .and_then(zbus::blocking::connection::Builder::build)
            .expect("fake portal on the private bus");
        Self { state, _conn: conn }
    }

    /// The next dialogs' answers, in order; [`Answer::Allow`] once they run out.
    pub fn answer(&self, answers: &[Answer]) {
        lock(&self.state).answers.extend(answers.iter().copied());
    }

    /// The user removed the permission: every restore token is unknown from now on.
    pub fn revoke(&self) {
        lock(&self.state).granted.clear();
    }

    /// The keyboard layout of the EIS keyboards created from now on.
    pub fn set_layout(&self, layout: Layout) {
        lock(&self.state).layout = layout;
    }

    /// What it saw so far.
    pub fn record(&self) -> Record {
        lock(&self.state).record.clone()
    }
}

/// `/org/freedesktop/portal/desktop/<kind>/<sender>/<token>`, `sender` without the colon and with
/// dots as underscores — the path a client can predict before its call.
fn handle(header: &zbus::message::Header<'_>, kind: &str, options: &HashMap<String, OwnedValue>, key: &str) -> zbus::fdo::Result<OwnedObjectPath> {
    let sender = header.sender().ok_or_else(|| zbus::fdo::Error::Failed("no sender".into()))?;
    let sender = sender.as_str().trim_start_matches(':').replace('.', "_");
    let token = string_option(options, key).ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("{key} missing")))?;
    OwnedObjectPath::try_from(format!("/org/freedesktop/portal/desktop/{kind}/{sender}/{token}")).map_err(|e| zbus::fdo::Error::InvalidArgs(e.to_string()))
}

fn string_option(options: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    options.get(key).and_then(|v| v.downcast_ref::<&str>().ok().map(str::to_owned))
}

fn u32_option(options: &HashMap<String, OwnedValue>, key: &str) -> Option<u32> {
    options.get(key).and_then(|v| v.downcast_ref::<u32>().ok())
}

/// `org.freedesktop.portal.Request.Response` to the caller.
async fn respond(conn: &zbus::Connection, header: &zbus::message::Header<'_>, request: &OwnedObjectPath, code: u32, results: HashMap<&str, Value<'_>>) -> zbus::fdo::Result<()> {
    conn.emit_signal(header.sender().map(|s| s.as_str()), request.as_str(), "org.freedesktop.portal.Request", "Response", &(code, results))
        .await
        .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))
}

struct RemoteDesktopIface {
    state: Shared,
}

#[zbus::interface(name = "org.freedesktop.portal.RemoteDesktop")]
impl RemoteDesktopIface {
    async fn create_session(
        &self,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let request = handle(&header, "request", &options, "handle_token")?;
        let session = handle(&header, "session", &options, "session_handle_token")?;
        server.at(request.as_ref(), RequestIface { state: self.state.clone() }).await?;
        server.at(session.as_ref(), SessionIface { state: self.state.clone(), path: session.to_string() }).await?;
        {
            let mut state = lock(&self.state);
            state.record.sessions += 1;
            state.sessions.insert(session.to_string(), SessionState::default());
        }
        let results = HashMap::from([("session_handle", Value::from(session.as_str()))]);
        respond(conn, &header, &request, 0, results).await?;
        Ok(request)
    }

    async fn select_devices(
        &self,
        session_handle: ObjectPath<'_>,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let request = handle(&header, "request", &options, "handle_token")?;
        server.at(request.as_ref(), RequestIface { state: self.state.clone() }).await?;
        let select = Select {
            types: u32_option(&options, "types").unwrap_or(0),
            persist_mode: u32_option(&options, "persist_mode").unwrap_or(0),
            restore_token: string_option(&options, "restore_token"),
        };
        let known = {
            let mut state = lock(&self.state);
            state.record.selects.push(select.clone());
            match state.sessions.get_mut(session_handle.as_str()) {
                Some(session) => {
                    session.select = Some(select);
                    true
                }
                None => false,
            }
        };
        respond(conn, &header, &request, if known { 0 } else { 2 }, HashMap::new()).await?;
        Ok(request)
    }

    async fn start(
        &self,
        session_handle: ObjectPath<'_>,
        _parent_window: &str,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let request = handle(&header, "request", &options, "handle_token")?;
        server.at(request.as_ref(), RequestIface { state: self.state.clone() }).await?;
        let mut results: HashMap<&str, Value<'_>> = HashMap::new();
        let code = {
            let mut state = lock(&self.state);
            let device_types = state.device_types;
            let select = match state.sessions.get(session_handle.as_str()) {
                Some(session) => session.select.clone().unwrap_or_default(),
                None => return Err(zbus::fdo::Error::Failed("unknown session".into())),
            };
            // xdg-desktop-portal takes the permission store's entry for a restore token out
            // before it starts the session: a token is good for one start.
            let restored = select.restore_token.as_ref().is_some_and(|token| state.granted.remove(token));
            let answer = if restored {
                state.record.restored += 1;
                Answer::Allow
            } else {
                state.record.dialogs += 1;
                state.answers.pop_front().unwrap_or(Answer::Allow)
            };
            match answer {
                Answer::Allow | Answer::AllowOnce => {
                    if answer == Answer::Allow && select.persist_mode == 2 {
                        state.next_token += 1;
                        let token = format!("restore-{}", state.next_token);
                        state.granted.insert(token.clone());
                        state.record.issued.push(token.clone());
                        results.insert("restore_token", Value::from(token));
                    }
                    results.insert("devices", Value::from(select.types & device_types));
                    if let Some(session) = state.sessions.get_mut(session_handle.as_str()) {
                        session.started = true;
                    }
                    Some(0)
                }
                Answer::Deny => Some(1),
                Answer::End => Some(2),
                Answer::Ignore => None,
            }
        };
        if let Some(code) = code {
            respond(conn, &header, &request, code, results).await?;
        }
        Ok(request)
    }

    #[zbus(name = "ConnectToEIS")]
    async fn connect_to_eis(&self, session_handle: ObjectPath<'_>, _options: HashMap<String, OwnedValue>) -> zbus::fdo::Result<zbus::zvariant::OwnedFd> {
        let (layout, index) = {
            let mut state = lock(&self.state);
            if state.version < 2 {
                return Err(zbus::fdo::Error::UnknownMethod("ConnectToEIS needs version 2".into()));
            }
            if !state.sessions.get(session_handle.as_str()).is_some_and(|s| s.started) {
                return Err(zbus::fdo::Error::Failed("session not started".into()));
            }
            state.record.eis_connections += 1;
            state.record.keys.push(Vec::new());
            (state.layout.clone(), state.record.keys.len() - 1)
        };
        let (ours, theirs) = UnixStream::pair().map_err(|e| zbus::fdo::Error::IOError(e.to_string()))?;
        let state = self.state.clone();
        std::thread::spawn(move || {
            if let Err(e) = serve_eis(ours, &layout, &state, index) {
                eprintln!("fake EIS keyboard: {e}");
            }
        });
        Ok(std::os::fd::OwnedFd::from(theirs).into())
    }

    #[zbus(property)]
    fn available_device_types(&self) -> u32 {
        lock(&self.state).device_types
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        lock(&self.state).version
    }
}

struct RequestIface {
    state: Shared,
}

#[zbus::interface(name = "org.freedesktop.portal.Request")]
impl RequestIface {
    async fn close(&self) {
        lock(&self.state).record.closed_requests += 1;
    }
}

struct SessionIface {
    state: Shared,
    path: String,
}

#[zbus::interface(name = "org.freedesktop.portal.Session")]
impl SessionIface {
    async fn close(&self) {
        let mut state = lock(&self.state);
        state.record.closed_sessions += 1;
        state.sessions.remove(&self.path);
    }

    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }
}

/// `layout` compiled to a keymap and put in a file, NUL-terminated as Mutter sends it.
fn keymap_file(layout: &Layout) -> Result<(std::fs::File, u32), String> {
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let keymap = xkb::Keymap::new_from_names(&context, "evdev", "pc105", layout.layout.as_str(), layout.variant.as_str(), layout.options.clone(), xkb::KEYMAP_COMPILE_NO_FLAGS)
        .ok_or_else(|| format!("no keymap for {layout:?} (is xkb-data installed?)"))?;
    let text = keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1);
    let mut file = tempfile::tempfile().map_err(|e| e.to_string())?;
    file.write_all(text.as_bytes()).and_then(|()| file.write_all(&[0])).map_err(|e| e.to_string())?;
    let size = u32::try_from(text.len() + 1).map_err(|e| e.to_string())?;
    Ok((file, size))
}

/// Wait until `fd` can be read, at most until `deadline`.
fn readable(fd: &impl AsFd, deadline: Instant) -> Result<(), String> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Err("client went quiet".into());
    }
    let timeout = rustix::time::Timespec { tv_sec: i64::try_from(left.as_secs()).unwrap_or(i64::MAX), tv_nsec: i64::from(left.subsec_nanos()) };
    let mut fds = [rustix::event::PollFd::new(fd, rustix::event::PollFlags::IN)];
    match rustix::event::poll(&mut fds, Some(&timeout)) {
        Ok(_) | Err(rustix::io::Errno::INTR) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// One EIS client: the handshake, a seat with a keyboard, the keyboard (keymap, active group,
/// resumed) once the client binds it, then every key until the client disconnects.
fn serve_eis(stream: UnixStream, layout: &Layout, state: &Shared, index: usize) -> Result<(), String> {
    let context = eis::Context::new(stream).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + EIS_PATIENCE;
    let mut handshaker = reis::handshake::EisHandshaker::new(&context, 1);
    let resp = 'handshake: loop {
        readable(&context, deadline)?;
        context.read().map_err(|e| e.to_string())?;
        while let Some(pending) = context.pending_request() {
            let PendingRequestResult::Request(request) = pending else {
                return Err(format!("bad handshake message: {pending:?}"));
            };
            if let Some(resp) = handshaker.handle_request(request).map_err(|e| e.to_string())? {
                break 'handshake resp;
            }
        }
    };
    let mut converter = reis::request::EisRequestConverter::new(&context, resp, 1);
    let _seat = converter.handle().add_seat(Some("seat0"), DeviceCapability::Keyboard.into());
    context.flush().map_err(|e| e.to_string())?;
    let (file, size) = keymap_file(layout)?;
    loop {
        readable(&context, deadline)?;
        match context.read() {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e.to_string()),
        }
        while let Some(pending) = context.pending_request() {
            let PendingRequestResult::Request(request) = pending else {
                return Err(format!("bad message: {pending:?}"));
            };
            converter.handle_request(request).map_err(|e| e.to_string())?;
        }
        while let Some(request) = converter.next_request() {
            match request {
                EisRequest::Bind(bind) if bind.capabilities.contains(DeviceCapability::Keyboard) => {
                    let device = bind.seat.add_device(Some("fake keyboard"), eis::device::DeviceType::Virtual, DeviceCapability::Keyboard.into(), |device| {
                        if let Some(keyboard) = device.interface::<eis::Keyboard>() {
                            keyboard.keymap(eis::keyboard::KeymapType::Xkb, size, file.as_fd());
                        }
                    });
                    if let Some(keyboard) = device.interface::<eis::Keyboard>() {
                        converter.handle().with_next_serial(|serial| keyboard.modifiers(serial, 0, 0, 0, layout.group));
                    }
                    device.resumed();
                    context.flush().map_err(|e| e.to_string())?;
                }
                EisRequest::KeyboardKey(key) => {
                    let pressed = key.state == eis::keyboard::KeyState::Press;
                    if let Some(keys) = lock(state).record.keys.get_mut(index) {
                        keys.push((key.key, pressed));
                    }
                }
                EisRequest::Disconnect => return Ok(()),
                _ => {}
            }
        }
    }
}
