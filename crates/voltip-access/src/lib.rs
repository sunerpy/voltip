//! Device credentials and short-lived tickets (docs/pairing.md 「设备票据」).
//!
//! An official build carries a build key for its version ([`build_key`], baked in by CI). With it
//! the device proves to `voltip-auth` which version it runs (`POST /prove`) and receives its own
//! device secret, kept in the system keychain ([`CredentialStore`]). With the secret it renews a
//! ticket every 45 minutes or so (`POST /ticket`), which it shows to the build's own relay
//! (`auth`) and to the built-in service (`X-Voltip-Ticket`). A user's relay or provider never
//! sees one.
//!
//! [`AccessClient`] does all of it in the background and is the [`TicketSource`] the transport and
//! the metering read. A build without a key has no client: it sends no tickets, and the hosted
//! relay serves it as long as it does not require them.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::sync::Arc;
use std::time::Duration;

use hmac::{Hmac, KeyInit as _, Mac as _};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use tokio::sync::{Notify, watch};
use tokio::time::Instant;

include!(concat!(env!("OUT_DIR"), "/build_key.rs"));

/// This build's version (the root `package.json`, which release-please bumps).
pub const APP_VERSION: &str = env!("VOLTIP_APP_VERSION");
/// The request header that carries a ticket to the built-in service.
pub const TICKET_HEADER: &str = "x-voltip-ticket";
/// A ticket is renewed when this share of its lifetime has passed.
const RENEW_AT: f64 = 0.75;
/// How the renewals are paced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    /// The first wait after a failed renewal; it doubles up to `max_backoff`.
    pub min_backoff: Duration,
    /// The longest wait between renewals that fail (and the wait after `version_not_allowed`
    /// or `revoked`, which only an update or the admin can change).
    pub max_backoff: Duration,
    /// [`TicketSource::refresh_now`] tries at once only when the last attempt is this old.
    pub min_gap: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self { min_backoff: Duration::from_secs(60), max_backoff: Duration::from_secs(30 * 60), min_gap: Duration::from_secs(10) }
    }
}
/// One request's deadline.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// A ticket, opaque to the client. `Debug` hides it.
#[derive(Clone, PartialEq, Eq)]
pub struct Ticket(Arc<str>);

impl Ticket {
    /// A ticket as `voltip-auth` issued it.
    pub fn new(value: impl Into<Arc<str>>) -> Self {
        Self(value.into())
    }

    /// The ticket's text, for the `auth` frame or the request header.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Ticket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Ticket(..)")
    }
}

/// Where the transport and the metering read the current ticket.
pub trait TicketSource: Send + Sync + std::fmt::Debug {
    /// The ticket in force, when there is one that has not run out.
    fn current(&self) -> Option<Ticket>;
    /// Ask for a new ticket soon (a relay said `ticket_required`, or there is none); never waits.
    fn refresh_now(&self);
    /// Changes whenever a new ticket comes.
    fn subscribe(&self) -> watch::Receiver<u64>;
}

/// The device secret `voltip-auth` gave this device. `Debug` hides it.
#[derive(Clone, PartialEq, Eq)]
pub struct DeviceSecret(String);

impl DeviceSecret {
    /// The secret as hex text.
    pub fn new(hex: impl Into<String>) -> Self {
        Self(hex.into())
    }

    /// The hex text, for the store.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for DeviceSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DeviceSecret(..)")
    }
}

/// Where the device secret is kept: the system keychain on the desktop and the phone, a private
/// file for `voltip-server`.
pub trait CredentialStore: Send + Sync {
    /// The secret kept, if any (an unreadable one counts as none).
    fn load(&self) -> Option<DeviceSecret>;
    /// Keep `secret`.
    fn save(&self, secret: &DeviceSecret) -> Result<(), String>;
    /// Forget the secret.
    fn clear(&self);
}

/// The build key of one version. `Debug` hides it.
#[derive(Clone, PartialEq, Eq)]
pub struct BuildKey {
    bytes: [u8; 32],
    version: String,
}

impl std::fmt::Debug for BuildKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuildKey").field("version", &self.version).finish_non_exhaustive()
    }
}

impl BuildKey {
    /// `bytes` for `version` (tests; a build reads its own with [`build_key`]).
    pub fn new(bytes: [u8; 32], version: impl Into<String>) -> Self {
        Self { bytes, version: version.into() }
    }

    /// The version the key proves.
    pub fn version(&self) -> &str {
        &self.version
    }
}

/// This build's key, when CI baked one in.
pub fn build_key() -> Option<BuildKey> {
    if !KEYED {
        return None;
    }
    // Neither array is folded into the other at compile time: the key is never whole in the binary.
    let pad = std::hint::black_box(&PAD);
    let masked = std::hint::black_box(&MASKED);
    let mut bytes = [0u8; 32];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = masked[i] ^ pad[i];
    }
    // The marker stays in the binary: the release scan finds it to know the key was taken.
    std::hint::black_box(MARKER);
    Some(BuildKey { bytes, version: APP_VERSION.to_owned() })
}

/// The `client` a request names: `voltip/<version> (<platform>)`.
pub fn client_name(version: &str, platform: &str) -> String {
    format!("voltip/{version} ({platform})")
}

fn hmac_hex(key: &[u8], message: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).unwrap_or_else(|_| unreachable!("HMAC takes a key of any length"));
    mac.update(message.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// `proof`: the build key's MAC of `prove|device|client|ts|nonce`, hex.
pub fn proof(key: &BuildKey, device_id: &str, client: &str, ts: u64, nonce: &str) -> String {
    hmac_hex(&key.bytes, &format!("prove|{device_id}|{client}|{ts}|{nonce}"))
}

/// `mac`: the device secret's MAC (its 32 bytes) of `<what>|device|client|ts|nonce`, hex (`what` is
/// `prove` or `ticket`).
pub fn device_mac(secret: &DeviceSecret, what: &str, device_id: &str, client: &str, ts: u64, nonce: &str) -> String {
    let key = hex::decode(secret.as_str()).unwrap_or_else(|_| secret.as_str().as_bytes().to_vec());
    hmac_hex(&key, &format!("{what}|{device_id}|{client}|{ts}|{nonce}"))
}

/// Why a renewal failed (`voltip-auth`'s answer, or the way to it).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AccessError {
    /// The HTTP client could not be built or the request did not get an answer.
    #[error("network: {0}")]
    Network(String),
    /// `voltip-auth` refused with this code (`prove_required`, `unknown_device`, `bad_mac`,
    /// `version_not_allowed`, `revoked`, `register_limited`…).
    #[error("refused: {0}")]
    Refused(String),
    /// An answer that is not `voltip-auth`'s.
    #[error("bad answer: {0}")]
    BadAnswer(String),
    /// The device secret could not be kept.
    #[error("store: {0}")]
    Store(String),
}

impl AccessError {
    fn refused(&self, code: &str) -> bool {
        matches!(self, Self::Refused(c) if c == code)
    }
}

/// Builds the HTTP client of one request: the shell's builder for the built-in service.
pub type HttpFactory = Arc<dyn Fn() -> reqwest::Result<reqwest::Client> + Send + Sync>;

/// What [`AccessClient`] needs to know.
#[derive(Clone, Debug)]
pub struct AccessConfig {
    /// `voltip-auth` behind the gateway, e.g. `https://<gateway>/auth/v1` (the build's
    /// `VOLTIP_AUTH_URL`).
    pub auth_url: String,
    /// This device's usage id.
    pub device_id: String,
    /// `linux`, `windows`, `macos`, `android`, `server`…
    pub platform: String,
}

#[derive(Serialize)]
struct ProveRequest<'a> {
    device_id: &'a str,
    client: &'a str,
    ts: u64,
    nonce: &'a str,
    proof: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    mac: Option<String>,
}

#[derive(Serialize)]
struct TicketRequest<'a> {
    device_id: &'a str,
    client: &'a str,
    ts: u64,
    nonce: &'a str,
    mac: String,
}

#[derive(Deserialize)]
struct ProveAnswer {
    device_secret: String,
}

#[derive(Deserialize)]
struct TicketAnswer {
    ticket: String,
    exp: u64,
    now: u64,
}

#[derive(Deserialize)]
struct Refusal {
    error: String,
}

/// A ticket held, with when it runs out and when to renew it on this device's clock (from the
/// server's own `now`, so the device's wall clock may be off).
#[derive(Clone, Debug)]
struct Held {
    ticket: Ticket,
    valid_until: Instant,
    renew_at: Instant,
}

/// The background renewal of this device's ticket.
pub struct AccessClient {
    config: AccessConfig,
    client: String,
    key: BuildKey,
    store: Arc<dyn CredentialStore>,
    http: HttpFactory,
    held: Mutex<Option<Held>>,
    wake: Arc<Notify>,
    tickets: watch::Sender<u64>,
    timing: Timing,
    /// Unix seconds (tests turn it).
    wall: fn() -> u64,
}

impl std::fmt::Debug for AccessClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccessClient").field("client", &self.client).field("has_ticket", &self.held.lock().is_some()).finish_non_exhaustive()
    }
}

impl AccessClient {
    /// A client for `config` proving with `key`. Nothing happens until [`AccessClient::spawn`].
    pub fn new(config: AccessConfig, key: BuildKey, store: Arc<dyn CredentialStore>, http: HttpFactory) -> Arc<Self> {
        Self::tuned(config, key, store, http, Timing::default(), unix_now)
    }

    /// [`AccessClient::new`] with its own `timing`, on `wall` (Unix seconds; tests).
    pub fn tuned(config: AccessConfig, key: BuildKey, store: Arc<dyn CredentialStore>, http: HttpFactory, timing: Timing, wall: fn() -> u64) -> Arc<Self> {
        let client = client_name(key.version(), &config.platform);
        Arc::new(Self {
            config,
            client,
            key,
            store,
            http,
            held: Mutex::new(None),
            wake: Arc::new(Notify::new()),
            tickets: watch::channel(0).0,
            timing,
            wall,
        })
    }

    /// Renew in the background on the current Tokio runtime until the client is dropped.
    pub fn spawn(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let weak = Arc::downgrade(self);
        let (wake, timing) = (self.wake.clone(), self.timing);
        tokio::spawn(async move {
            let mut backoff = timing.min_backoff;
            loop {
                let Some(this) = weak.upgrade() else { return };
                let attempt = Instant::now();
                let wait = match this.renew().await {
                    Ok(renew_at) => {
                        backoff = timing.min_backoff;
                        renew_at.saturating_duration_since(Instant::now())
                    }
                    Err(e) => {
                        let wait = if e.refused("version_not_allowed") || e.refused("revoked") { timing.max_backoff } else { backoff };
                        backoff = (backoff * 2).min(timing.max_backoff);
                        tracing::info!(error = %e, ?wait, "device ticket not renewed");
                        wait
                    }
                };
                drop(this);
                let woken = tokio::select! {
                    () = tokio::time::sleep(wait) => false,
                    () = wake.notified() => true,
                };
                // Asked at once: still no sooner than `min_gap` after the last try.
                if woken && attempt.elapsed() < timing.min_gap {
                    tokio::time::sleep(timing.min_gap - attempt.elapsed()).await;
                }
            }
        })
    }

    /// One renewal: prove the version when there is no secret or `voltip-auth` asks for it, then
    /// fetch a ticket. Returns when to renew next.
    pub async fn renew(&self) -> Result<Instant, AccessError> {
        if self.store.load().is_none() {
            self.prove().await?;
        }
        let held = match self.fetch_ticket().await {
            Err(e) if e.refused("prove_required") || e.refused("unknown_device") || e.refused("bad_mac") => {
                if e.refused("bad_mac") {
                    self.store.clear();
                }
                self.prove().await?;
                self.fetch_ticket().await?
            }
            other => other?,
        };
        let renew_at = held.renew_at;
        *self.held.lock() = Some(held);
        self.tickets.send_modify(|n| *n += 1);
        Ok(renew_at)
    }

    /// `POST /prove`: show the build key's proof and keep the device secret that comes back.
    async fn prove(&self) -> Result<(), AccessError> {
        let ts = (self.wall)();
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let device = &self.config.device_id;
        let secret = self.store.load();
        let request = ProveRequest {
            device_id: device,
            client: &self.client,
            ts,
            nonce: &nonce,
            proof: proof(&self.key, device, &self.client, ts, &nonce),
            mac: secret.as_ref().map(|s| device_mac(s, "prove", device, &self.client, ts, &nonce)),
        };
        let answer: ProveAnswer = match self.post("prove", &request).await {
            Err(e) if e.refused("bad_mac") => {
                // A secret this device no longer has right: forget it and prove afresh.
                self.store.clear();
                let request = ProveRequest { mac: None, ..request };
                self.post("prove", &request).await?
            }
            other => other?,
        };
        if answer.device_secret.len() != 64 || !answer.device_secret.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(AccessError::BadAnswer("device_secret".into()));
        }
        self.store.save(&DeviceSecret::new(answer.device_secret)).map_err(AccessError::Store)?;
        tracing::info!(client = %self.client, "device proved its version");
        Ok(())
    }

    /// `POST /ticket`.
    async fn fetch_ticket(&self) -> Result<Held, AccessError> {
        let Some(secret) = self.store.load() else { return Err(AccessError::Refused("prove_required".into())) };
        let ts = (self.wall)();
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let device = &self.config.device_id;
        let request = TicketRequest { device_id: device, client: &self.client, ts, nonce: &nonce, mac: device_mac(&secret, "ticket", device, &self.client, ts, &nonce) };
        let answer: TicketAnswer = self.post("ticket", &request).await?;
        if answer.ticket.is_empty() || answer.ticket.len() > 1024 || answer.exp <= answer.now {
            return Err(AccessError::BadAnswer("ticket".into()));
        }
        let lifetime = Duration::from_secs(answer.exp - answer.now);
        let now = Instant::now();
        Ok(Held { ticket: Ticket::new(answer.ticket), valid_until: now + lifetime, renew_at: now + lifetime.mul_f64(RENEW_AT) })
    }

    async fn post<T: serde::de::DeserializeOwned>(&self, path: &str, body: &impl Serialize) -> Result<T, AccessError> {
        let http = (self.http)().map_err(|e| AccessError::Network(e.to_string()))?;
        let url = format!("{}/{path}", self.config.auth_url.trim_end_matches('/'));
        let response = http.post(&url).timeout(REQUEST_TIMEOUT).json(body).send().await.map_err(|e| AccessError::Network(kind_of(&e)))?;
        let status = response.status();
        let text = response.text().await.map_err(|e| AccessError::Network(kind_of(&e)))?;
        if !status.is_success() {
            return Err(match serde_json::from_str::<Refusal>(&text) {
                Ok(r) if r.error.len() <= 64 => AccessError::Refused(r.error),
                _ => AccessError::BadAnswer(format!("status {}", status.as_u16())),
            });
        }
        serde_json::from_str(&text).map_err(|_| AccessError::BadAnswer(format!("{path}: not JSON")))
    }
}

/// A request failure's kind, without the URL (the build's auth address is a build secret).
fn kind_of(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "timeout".into()
    } else if error.is_connect() {
        "connect".into()
    } else {
        "request".into()
    }
}

impl TicketSource for AccessClient {
    fn current(&self) -> Option<Ticket> {
        let held = self.held.lock();
        held.as_ref().filter(|h| Instant::now() < h.valid_until).map(|h| h.ticket.clone())
    }

    fn refresh_now(&self) {
        self.wake.notify_one();
    }

    fn subscribe(&self) -> watch::Receiver<u64> {
        self.tickets.subscribe()
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests;
