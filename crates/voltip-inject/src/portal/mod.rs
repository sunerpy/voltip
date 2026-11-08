//! GNOME's remote-desktop portal as the keyboard of a paste (docs/dictation.md §14.9, issue #235).
//!
//! GNOME offers applications no virtual-keyboard protocol. A paste through XTEST on Xwayland works,
//! but Xwayland asks the portal for remote interaction on Voltip's behalf without a persistent
//! grant: the dialog came back whenever that grant had lapsed (ten minutes after the last X11
//! connection, or after a restart of Voltip or Xwayland). Voltip now asks
//! `org.freedesktop.portal.RemoteDesktop` itself, for the keyboard only and with a grant that is
//! remembered until the user revokes it (`persist_mode` 2), and keeps the restore token the portal
//! hands out in a [`RestoreTokens`] store (the system keychain in the desktop shell):
//!
//! 1. `CreateSession`, then `SelectDevices` with the saved token, if any;
//! 2. `Start`: with a valid token the portal restores the grant without a dialog, otherwise it asks
//!    the user once. The token it returns replaces the saved one; an answer without a token (the
//!    user did not tick "Remember This Selection") or without the keyboard clears it, so the next
//!    paste asks again;
//! 3. `ConnectToEIS`, the chord on that keyboard ([`eis`]), with keys looked up in the keymap the
//!    compositor announced ([`keymap`]);
//! 4. `Session.Close` right after: no session outlives its chord.
//!
//! The portal is never asked at start-up, only when a paste or a voice-editing copy needs it. A
//! token the portal refuses outright (`SelectDevices` fails) is dropped and the session asked for
//! once more without it.

mod eis;
pub mod keymap;

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures_util::StreamExt as _;
use futures_util::future::{Either, select};
use zbus::message::Type;
use zbus::names::OwnedUniqueName;
use zbus::zvariant::{ObjectPath, OwnedFd, OwnedObjectPath, OwnedValue, Value};
use zbus::{MatchRule, MessageStream};

pub use keymap::{KeyEvent, KeyLookup};

use crate::platform::KeyStep;
use crate::restore::RestoreTokens;

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const REQUESTS: &str = "/org/freedesktop/portal/desktop/request";
const REMOTE_DESKTOP: &str = "org.freedesktop.portal.RemoteDesktop";
const REQUEST: &str = "org.freedesktop.portal.Request";
const SESSION: &str = "org.freedesktop.portal.Session";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

/// `types` bit of a keyboard.
pub const KEYBOARD: u32 = 1;
/// `persist_mode`: remembered until the user revokes it.
pub const PERSISTENT: u32 = 2;
/// The interface version that has `ConnectToEIS`.
pub const EIS_VERSION: u32 = 2;
/// The `Response` code of a request the user cancelled.
const CANCELLED: u32 = 1;

/// How long each part of a paste through the portal may take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// A D-Bus call, and the portal's answer to a request without a dialog.
    pub call: Duration,
    /// The user's answer to the dialog.
    pub answer: Duration,
    /// Setting the keyboard up and pressing the chord.
    pub keyboard: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self { call: Duration::from_secs(5), answer: Duration::from_secs(60), keyboard: Duration::from_secs(3) }
    }
}

/// Why a chord did not go out through the portal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PortalError {
    /// No remote-desktop portal with a keyboard on this session bus: the next tool may try.
    Unavailable(String),
    /// The user did not allow remote interaction (cancelled, no answer in time, or a grant without
    /// the keyboard). Nothing was pressed; the next paste asks again.
    Declined(String),
    /// Allowed, but the keyboard could not be set up. Nothing was pressed.
    Broken(String),
    /// The keyboard failed after keys went out: some of the chord may have reached the application.
    Lost(String),
}

impl std::fmt::Display for PortalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(reason) => write!(f, "no remote-desktop portal: {reason}"),
            Self::Declined(reason) => write!(f, "remote interaction not allowed: {reason}"),
            Self::Broken(reason) => write!(f, "the portal's keyboard failed: {reason}"),
            Self::Lost(reason) => write!(f, "the portal's keyboard failed during the chord: {reason}"),
        }
    }
}

/// How a session ended without a chord, inside [`RemoteDesktop::press`].
enum Failure {
    Portal(PortalError),
    /// `SelectDevices` refused the saved token.
    BadToken(String),
}

impl From<PortalError> for Failure {
    fn from(e: PortalError) -> Self {
        Self::Portal(e)
    }
}

/// One session at a time in this process: a second chord (the paste injector's and the selection
/// copier's portals share a token store) waits for the first one, its dialog included.
static BUSY: Mutex<()> = Mutex::new(());

/// The remote-desktop portal of a session bus as a keyboard.
pub struct RemoteDesktop {
    address: Option<String>,
    tokens: Arc<dyn RestoreTokens>,
    limits: Limits,
}

impl std::fmt::Debug for RemoteDesktop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteDesktop").field("address", &self.address.is_some()).field("limits", &self.limits).finish_non_exhaustive()
    }
}

impl RemoteDesktop {
    /// The portal of this process's session bus (`DBUS_SESSION_BUS_ADDRESS`), with `tokens`.
    pub fn session(tokens: Arc<dyn RestoreTokens>) -> Self {
        Self { address: None, tokens, limits: Limits::default() }
    }

    /// The portal of the bus at `address` (tests).
    pub fn at(address: impl Into<String>, tokens: Arc<dyn RestoreTokens>, limits: Limits) -> Self {
        Self { address: Some(address.into()), tokens, limits }
    }

    /// Whether the bus has a remote-desktop portal with `ConnectToEIS` and a keyboard. Reads two
    /// properties: no request, no dialog.
    pub fn available(&self) -> Result<(), PortalError> {
        async_io::block_on(async {
            let bus = self.connect().await?;
            self.check(&bus).await
        })
    }

    /// Press `steps` on the portal's keyboard, asking the user first when no grant is remembered.
    pub fn press(&self, steps: &[KeyStep]) -> Result<(), PortalError> {
        let _one = BUSY.lock().unwrap_or_else(PoisonError::into_inner);
        async_io::block_on(async {
            let bus = self.connect().await?;
            self.check(&bus).await?;
            let saved = self.tokens.load();
            let restoring = saved.is_some();
            match self.paste(&bus, steps, saved.as_deref()).await {
                Ok(()) => Ok(()),
                Err(Failure::Portal(e)) => Err(e),
                Err(Failure::BadToken(reason)) => {
                    tracing::info!(%reason, restoring, "the portal refused the saved grant; asking without it");
                    self.tokens.clear();
                    self.paste(&bus, steps, None).await.map_err(|failure| match failure {
                        Failure::Portal(e) => e,
                        Failure::BadToken(reason) => PortalError::Unavailable(reason),
                    })
                }
            }
        })
    }

    async fn connect(&self) -> Result<zbus::Connection, PortalError> {
        let builder = match &self.address {
            Some(address) => zbus::connection::Builder::address(address.as_str()),
            None => zbus::connection::Builder::session(),
        }
        .map_err(|e| PortalError::Unavailable(format!("session bus: {e}")))?
        .method_timeout(self.limits.call);
        within(self.limits.call, builder.build())
            .await
            .ok_or_else(|| PortalError::Unavailable(format!("session bus: no connection within {:?}", self.limits.call)))?
            .map_err(|e| PortalError::Unavailable(format!("session bus: {e}")))
    }

    /// The portal is there, new enough and offers the keyboard.
    async fn check(&self, bus: &zbus::Connection) -> Result<(), PortalError> {
        let version: u32 = property(bus, "version").await.map_err(|e| PortalError::Unavailable(format!("{REMOTE_DESKTOP}: {e}")))?;
        if version < EIS_VERSION {
            return Err(PortalError::Unavailable(format!("{REMOTE_DESKTOP} version {version} has no ConnectToEIS")));
        }
        let types: u32 = property(bus, "AvailableDeviceTypes").await.map_err(|e| PortalError::Unavailable(format!("{REMOTE_DESKTOP}: {e}")))?;
        if types & KEYBOARD == 0 {
            return Err(PortalError::Unavailable(format!("{REMOTE_DESKTOP} offers no keyboard (device types {types})")));
        }
        Ok(())
    }

    /// One session: create, select, start, the chord, close.
    async fn paste(&self, bus: &zbus::Connection, steps: &[KeyStep], token: Option<&str>) -> Result<(), Failure> {
        let mut answers = Answers::new(bus).await?;
        let session = self.create_session(bus, &mut answers).await?;
        let result = self.in_session(bus, &mut answers, &session, steps, token).await;
        // Best effort: the portal closes a session itself when its start failed.
        if let Err(e) = bus.call_method(Some(PORTAL), session.as_str(), Some(SESSION), "Close", &()).await {
            tracing::debug!(error = %e, "Session.Close");
        }
        result
    }

    async fn create_session(&self, bus: &zbus::Connection, answers: &mut Answers) -> Result<OwnedObjectPath, PortalError> {
        let handle = handle_token();
        let options = HashMap::from([("handle_token", Value::from(handle.as_str())), ("session_handle_token", Value::from(handle_token()))]);
        let request = call_request(bus, "CreateSession", &(options,)).await.map_err(PortalError::Unavailable)?;
        let (code, results) = answers.wait(bus, &handle, &request, self.limits.call).await.map_err(PortalError::Unavailable)?;
        if code != 0 {
            return Err(PortalError::Unavailable(format!("CreateSession answered {code}")));
        }
        session_handle(&results).ok_or_else(|| PortalError::Unavailable("CreateSession answered without a session".into()))
    }

    async fn in_session(
        &self,
        bus: &zbus::Connection,
        answers: &mut Answers,
        session: &OwnedObjectPath,
        steps: &[KeyStep],
        token: Option<&str>,
    ) -> Result<(), Failure> {
        // SelectDevices: the keyboard only, remembered, restored from `token`.
        let handle = handle_token();
        let mut options =
            HashMap::from([("handle_token", Value::from(handle.as_str())), ("types", Value::from(KEYBOARD)), ("persist_mode", Value::from(PERSISTENT))]);
        if let Some(token) = token {
            options.insert("restore_token", Value::from(token));
        }
        let selected = match call_request(bus, "SelectDevices", &(session, options)).await {
            Ok(request) => answers.wait(bus, &handle, &request, self.limits.call).await,
            Err(e) => Err(e),
        };
        match selected {
            Ok((0, _)) => {}
            Ok((code, _)) if token.is_some() => return Err(Failure::BadToken(format!("SelectDevices answered {code}"))),
            Ok((code, _)) => return Err(PortalError::Unavailable(format!("SelectDevices answered {code}")).into()),
            Err(e) if token.is_some() => return Err(Failure::BadToken(e)),
            Err(e) => return Err(PortalError::Unavailable(e).into()),
        }

        // Start: the dialog, unless the grant is restored. The portal has taken an offered token out
        // of its store by now; it is good again only when Start hands it back.
        let handle = handle_token();
        let options = HashMap::from([("handle_token", Value::from(handle.as_str()))]);
        let started = match call_request(bus, "Start", &(session, "", options)).await {
            Ok(request) => answers.wait(bus, &handle, &request, self.limits.answer).await,
            Err(e) => Err(e),
        };
        let results = match started {
            Ok((0, results)) => results,
            Ok((code, _)) => {
                self.tokens.clear();
                return Err(if code == CANCELLED {
                    PortalError::Declined("the user cancelled".into())
                } else {
                    PortalError::Broken(format!("Start answered {code}"))
                }
                .into());
            }
            Err(e) => {
                self.tokens.clear();
                return Err(if e.starts_with(NO_ANSWER) { PortalError::Declined(e) } else { PortalError::Broken(e) }.into());
            }
        };
        let devices = results.get("devices").and_then(|v| v.downcast_ref::<u32>().ok()).unwrap_or(0);
        if devices & KEYBOARD == 0 {
            self.tokens.clear();
            return Err(PortalError::Declined(format!("allowed without the keyboard (devices {devices})")).into());
        }
        match results.get("restore_token").and_then(|v| v.downcast_ref::<&str>().ok()) {
            Some(new) => {
                self.tokens.save(new);
                tracing::debug!(restored = token.is_some(), "remote interaction granted; restore token saved");
            }
            None => {
                self.tokens.clear();
                tracing::info!("remote interaction granted for this paste only; the next paste asks again");
            }
        }

        // The keyboard.
        let reply = bus
            .call_method(Some(PORTAL), PORTAL_PATH, Some(REMOTE_DESKTOP), "ConnectToEIS", &(session, HashMap::<&str, Value<'_>>::new()))
            .await
            .map_err(|e| PortalError::Broken(format!("ConnectToEIS: {e}")))?;
        let fd: OwnedFd = reply.body().deserialize().map_err(|e| PortalError::Broken(format!("ConnectToEIS: {e}")))?;
        eis::press(fd.into(), steps, self.limits.keyboard).map_err(|e| match e {
            eis::EisError::Before(reason) => PortalError::Broken(reason),
            eis::EisError::After(reason) => PortalError::Lost(reason),
        })?;
        Ok(())
    }
}

/// The start of a request error for a dialog nobody answered.
const NO_ANSWER: &str = "no answer within";

/// A property of the RemoteDesktop interface.
async fn property<T: TryFrom<OwnedValue>>(bus: &zbus::Connection, name: &str) -> Result<T, String> {
    let reply = bus.call_method(Some(PORTAL), PORTAL_PATH, Some(PROPERTIES), "Get", &(REMOTE_DESKTOP, name)).await.map_err(|e| e.to_string())?;
    let value: OwnedValue = reply.body().deserialize().map_err(|e| e.to_string())?;
    T::try_from(value).map_err(|_| format!("{name}: unexpected type"))
}

/// A RemoteDesktop method that returns a request object.
async fn call_request<B>(bus: &zbus::Connection, method: &str, body: &B) -> Result<OwnedObjectPath, String>
where
    B: zbus::export::serde::Serialize + zbus::zvariant::DynamicType,
{
    let reply = bus.call_method(Some(PORTAL), PORTAL_PATH, Some(REMOTE_DESKTOP), method, body).await.map_err(|e| format!("{method}: {e}"))?;
    reply.body().deserialize::<OwnedObjectPath>().map_err(|e| format!("{method}: {e}"))
}

/// The session handle of `CreateSession`'s answer: an object path, or a string in older portals.
fn session_handle(results: &HashMap<String, OwnedValue>) -> Option<OwnedObjectPath> {
    match &**results.get("session_handle")? {
        Value::ObjectPath(path) => Some(path.clone().into()),
        Value::Str(path) => OwnedObjectPath::try_from(path.as_str()).ok(),
        _ => None,
    }
}

/// A fresh `handle_token`: letters, digits and underscores, unique in this process.
fn handle_token() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!("voltip_{}_{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed))
}

/// The request path the portal derives from our unique name and `token`, known before the call.
fn request_path(bus: &zbus::Connection, token: &str) -> Option<String> {
    let sender = bus.unique_name()?.as_str().trim_start_matches(':').replace('.', "_");
    Some(format!("{REQUESTS}/{sender}/{token}"))
}

/// `org.freedesktop.portal.Request.Response` signals from the portal, subscribed to before the
/// calls so that none is missed.
struct Answers {
    stream: MessageStream,
    portal: Option<OwnedUniqueName>,
}

impl Answers {
    async fn new(bus: &zbus::Connection) -> Result<Self, PortalError> {
        let rule = MatchRule::builder()
            .msg_type(Type::Signal)
            .interface(REQUEST)
            .and_then(|b| b.member("Response"))
            .and_then(|b| b.path_namespace(REQUESTS))
            .map(zbus::match_rule::Builder::build)
            .map_err(|e| PortalError::Unavailable(format!("match rule: {e}")))?;
        let stream = MessageStream::for_match_rule(rule, bus, Some(64)).await.map_err(|e| PortalError::Unavailable(format!("AddMatch: {e}")))?;
        let portal = match zbus::fdo::DBusProxy::new(bus).await {
            Ok(proxy) => match zbus::names::BusName::try_from(PORTAL) {
                Ok(name) => proxy.get_name_owner(name).await.ok(),
                Err(_) => None,
            },
            Err(_) => None,
        };
        Ok(Self { stream, portal })
    }

    /// The answer to the request `token` (returned as `returned`), within `limit`; past it the
    /// request is closed, which also closes its dialog.
    async fn wait(
        &mut self,
        bus: &zbus::Connection,
        token: &str,
        returned: &OwnedObjectPath,
        limit: Duration,
    ) -> Result<(u32, HashMap<String, OwnedValue>), String> {
        let expected = request_path(bus, token);
        let ours = |path: &ObjectPath<'_>| path.as_str() == returned.as_str() || expected.as_deref() == Some(path.as_str());
        let portal = self.portal.clone();
        let stream = &mut self.stream;
        let answer = async move {
            while let Some(message) = stream.next().await {
                let Ok(message) = message else { continue };
                let header = message.header();
                if !header.path().is_some_and(ours) {
                    continue;
                }
                if let (Some(portal), Some(sender)) = (&portal, header.sender())
                    && portal.as_str() != sender.as_str()
                {
                    continue;
                }
                return message.body().deserialize::<(u32, HashMap<String, OwnedValue>)>().map_err(|e| format!("Response: {e}"));
            }
            Err("the session bus connection closed".to_string())
        };
        match within(limit, answer).await {
            Some(answer) => answer,
            None => {
                if let Err(e) = bus.call_method(Some(PORTAL), returned.as_str(), Some(REQUEST), "Close", &()).await {
                    tracing::debug!(error = %e, "Request.Close");
                }
                Err(format!("{NO_ANSWER} {limit:?}"))
            }
        }
    }
}

/// `future`'s output, or `None` when `limit` passes first.
async fn within<T>(limit: Duration, future: impl Future<Output = T>) -> Option<T> {
    let future = std::pin::pin!(future);
    let timer = std::pin::pin!(async_io::Timer::after(limit));
    match select(future, timer).await {
        Either::Left((value, _)) => Some(value),
        Either::Right(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_tokens_are_unique_object_path_elements() {
        let (a, b) = (handle_token(), handle_token());
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'), "{a}");
    }

    #[test]
    fn session_handles_as_path_or_string() {
        let path = |p: &str| OwnedObjectPath::try_from(p).unwrap();
        let as_path = HashMap::from([(
            "session_handle".to_string(),
            OwnedValue::try_from(Value::from(path("/org/freedesktop/portal/desktop/session/1_2/t").into_inner())).unwrap(),
        )]);
        assert_eq!(session_handle(&as_path), Some(path("/org/freedesktop/portal/desktop/session/1_2/t")));
        let as_string =
            HashMap::from([("session_handle".to_string(), OwnedValue::try_from(Value::from("/org/freedesktop/portal/desktop/session/1_2/t")).unwrap())]);
        assert_eq!(session_handle(&as_string), Some(path("/org/freedesktop/portal/desktop/session/1_2/t")));
        let wrong = HashMap::from([("session_handle".to_string(), OwnedValue::from(7u32))]);
        assert_eq!(session_handle(&wrong), None);
        assert_eq!(session_handle(&HashMap::new()), None);
    }

    #[test]
    fn errors_say_what_happened() {
        assert_eq!(PortalError::Unavailable("x".into()).to_string(), "no remote-desktop portal: x");
        assert_eq!(PortalError::Declined("the user cancelled".into()).to_string(), "remote interaction not allowed: the user cancelled");
        assert_eq!(PortalError::Broken("y".into()).to_string(), "the portal's keyboard failed: y");
        assert_eq!(PortalError::Lost("z".into()).to_string(), "the portal's keyboard failed during the chord: z");
        assert_eq!(Limits::default(), Limits { call: Duration::from_secs(5), answer: Duration::from_secs(60), keyboard: Duration::from_secs(3) });
    }

    #[test]
    fn within_ends_at_the_limit() {
        assert_eq!(async_io::block_on(within(Duration::from_secs(5), async { 7 })), Some(7));
        assert_eq!(async_io::block_on(within(Duration::from_millis(10), std::future::pending::<()>())), None);
    }

    /// No bus at the address: unavailable, the next tool of the chain tries.
    #[test]
    fn no_bus_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let address = format!("unix:path={}", dir.path().join("none").display());
        let portal = RemoteDesktop::at(address, Arc::new(crate::restore::MemoryRestoreTokens::new()), Limits::default());
        assert!(matches!(portal.available(), Err(PortalError::Unavailable(_))));
        assert!(matches!(portal.press(&[]), Err(PortalError::Unavailable(_))));
        assert_eq!(format!("{portal:?}"), "RemoteDesktop { address: true, limits: Limits { call: 5s, answer: 60s, keyboard: 3s }, .. }");
    }
}
