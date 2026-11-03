use std::sync::atomic::{AtomicUsize, Ordering};

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use super::*;

const DEVICE: &str = "00000000-0000-4000-8000-000000000001";
const SECRET: &str = "202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f";

/// The shared vectors: `voltip-auth`'s tests (auth/test) check the same inputs and outputs.
#[test]
fn the_shared_vectors_hold() {
    let key = BuildKey::new(std::array::from_fn(|i| i as u8), "0.3.1");
    let client = client_name("0.3.1", "linux");
    assert_eq!(client, "voltip/0.3.1 (linux)");
    let nonce = "0123456789abcdef0123456789abcdef";
    assert_eq!(proof(&key, DEVICE, &client, 1_800_000_000, nonce), "66cdb343466bcb525628765e2ef9e61f5f578b534502c1fff63c29300b7620fd");
    let secret = DeviceSecret::new(SECRET);
    assert_eq!(device_mac(&secret, "ticket", DEVICE, &client, 1_800_000_000, nonce), "d02d0abe3713d014112776f44fb7aaedd1171b9e0bf949408c4d047ccb1b6ca9");
}

#[test]
fn secrets_never_print() {
    let key = BuildKey::new([0xab; 32], "0.3.1");
    assert_eq!(format!("{key:?}"), r#"BuildKey { version: "0.3.1", .. }"#);
    assert_eq!(format!("{:?}", DeviceSecret::new(SECRET)), "DeviceSecret(..)");
    assert_eq!(format!("{:?}", Ticket::new("payload.mac")), "Ticket(..)");
}

/// A build without `VOLTIP_BUILD_KEY` (every test and PR build) has no key; a keyed one
/// (`scripts/check-build-key.sh` sets `VOLTIP_EXPECT_PROOF` to the proof its key makes of
/// `prove|d|c|1|n`, so the test binary holds no copy of the key) gets back exactly that key.
#[test]
fn the_build_key_is_the_one_ci_baked_in() {
    match option_env!("VOLTIP_EXPECT_PROOF") {
        None => assert_eq!(build_key().is_some(), KEYED),
        Some(expected) => {
            let built = build_key().expect("a keyed build");
            assert_eq!(built.version(), APP_VERSION);
            assert_eq!(proof(&built, "d", "c", 1, "n"), expected);
        }
    }
}

#[derive(Default)]
struct MemoryStore(Mutex<Option<DeviceSecret>>);

impl CredentialStore for MemoryStore {
    fn load(&self) -> Option<DeviceSecret> {
        self.0.lock().clone()
    }
    fn save(&self, secret: &DeviceSecret) -> Result<(), String> {
        *self.0.lock() = Some(secret.clone());
        Ok(())
    }
    fn clear(&self) {
        *self.0.lock() = None;
    }
}

/// `voltip-auth` as a test double: checks the proof with the test build key and the mac with the
/// secret it gives out; `ticket_refusals` answers the first tickets with these codes.
struct FakeAuth {
    proves: AtomicUsize,
    tickets: AtomicUsize,
    ticket_refusals: Mutex<Vec<&'static str>>,
    lifetime: u64,
}

impl FakeAuth {
    fn new(lifetime: u64, ticket_refusals: Vec<&'static str>) -> Arc<Self> {
        Arc::new(Self { proves: AtomicUsize::new(0), tickets: AtomicUsize::new(0), ticket_refusals: Mutex::new(ticket_refusals), lifetime })
    }
}

struct Prove(Arc<FakeAuth>);
struct Tickets(Arc<FakeAuth>);

fn key() -> BuildKey {
    BuildKey::new([3; 32], "0.3.1")
}

fn refusal(code: &str) -> ResponseTemplate {
    ResponseTemplate::new(403).set_body_json(serde_json::json!({ "error": code }))
}

impl Respond for Prove {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        self.0.proves.fetch_add(1, Ordering::SeqCst);
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        let (device, client, ts, nonce) =
            (body["device_id"].as_str().unwrap(), body["client"].as_str().unwrap(), body["ts"].as_u64().unwrap(), body["nonce"].as_str().unwrap());
        if body["proof"] != proof(&key(), device, client, ts, nonce) {
            return refusal("bad_proof");
        }
        if let Some(mac) = body.get("mac")
            && mac != &device_mac(&DeviceSecret::new(SECRET), "prove", device, client, ts, nonce)
        {
            return refusal("bad_mac");
        }
        ResponseTemplate::new(200).set_body_json(serde_json::json!({ "device_secret": SECRET }))
    }
}

impl Respond for Tickets {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        self.0.tickets.fetch_add(1, Ordering::SeqCst);
        if let Some(code) = self.0.ticket_refusals.lock().pop() {
            return refusal(code);
        }
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        let (device, client, ts, nonce) =
            (body["device_id"].as_str().unwrap(), body["client"].as_str().unwrap(), body["ts"].as_u64().unwrap(), body["nonce"].as_str().unwrap());
        if body["mac"] != device_mac(&DeviceSecret::new(SECRET), "ticket", device, client, ts, nonce) {
            return refusal("bad_mac");
        }
        let n = self.0.tickets.load(Ordering::SeqCst);
        ResponseTemplate::new(200).set_body_json(serde_json::json!({ "ticket": format!("t{n}"), "exp": 1_000 + self.0.lifetime, "now": 1_000 }))
    }
}

async fn auth(fake: &Arc<FakeAuth>) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/auth/v1/prove")).respond_with(Prove(fake.clone())).mount(&server).await;
    Mock::given(method("POST")).and(path("/auth/v1/ticket")).respond_with(Tickets(fake.clone())).mount(&server).await;
    server
}

fn wall() -> u64 {
    1_000
}

fn fast() -> Timing {
    Timing { min_backoff: Duration::from_millis(50), max_backoff: Duration::from_millis(200), min_gap: Duration::from_millis(100) }
}

fn client(server: &MockServer, store: Arc<MemoryStore>, built: Arc<AtomicUsize>) -> Arc<AccessClient> {
    let config = AccessConfig { auth_url: format!("{}/auth/v1/", server.uri()), device_id: DEVICE.into(), platform: "linux".into() };
    let http: HttpFactory = Arc::new(move || {
        built.fetch_add(1, Ordering::SeqCst);
        reqwest::Client::builder().build()
    });
    AccessClient::tuned(config, key(), store, http, fast(), wall)
}

#[tokio::test]
async fn a_first_renewal_proves_keeps_the_secret_and_holds_a_ticket() {
    let fake = FakeAuth::new(3600, Vec::new());
    let server = auth(&fake).await;
    let (store, built) = (Arc::new(MemoryStore::default()), Arc::new(AtomicUsize::new(0)));
    let access = client(&server, store.clone(), built.clone());
    let mut changes = access.subscribe();
    assert_eq!(access.current(), None);
    access.renew().await.unwrap();
    assert_eq!(access.current().unwrap().as_str(), "t1");
    assert_eq!(store.load().unwrap().as_str(), SECRET);
    assert!(changes.has_changed().unwrap());
    changes.mark_unchanged();
    assert_eq!((fake.proves.load(Ordering::SeqCst), fake.tickets.load(Ordering::SeqCst)), (1, 1));
    assert_eq!(built.load(Ordering::SeqCst), 2, "the shell's client builder, once a request");
    // Next time the secret is there: a ticket only.
    access.renew().await.unwrap();
    assert_eq!((fake.proves.load(Ordering::SeqCst), fake.tickets.load(Ordering::SeqCst)), (1, 2));
    assert!(changes.has_changed().unwrap());
}

#[tokio::test]
async fn asked_to_prove_again_it_proves_and_retries_the_ticket() {
    for code in ["prove_required", "unknown_device", "bad_mac"] {
        let fake = FakeAuth::new(3600, vec![code]);
        let server = auth(&fake).await;
        let store = Arc::new(MemoryStore::default());
        store.save(&DeviceSecret::new(SECRET)).unwrap();
        let access = client(&server, store.clone(), Arc::new(AtomicUsize::new(0)));
        access.renew().await.unwrap();
        assert_eq!((fake.proves.load(Ordering::SeqCst), fake.tickets.load(Ordering::SeqCst)), (1, 2), "{code}");
        assert!(access.current().is_some(), "{code}");
    }
}

#[tokio::test]
async fn a_lost_secret_is_dropped_and_proved_afresh() {
    let fake = FakeAuth::new(3600, Vec::new());
    let server = auth(&fake).await;
    let store = Arc::new(MemoryStore::default());
    store.save(&DeviceSecret::new("00".repeat(32))).unwrap();
    let access = client(&server, store.clone(), Arc::new(AtomicUsize::new(0)));
    access.renew().await.unwrap();
    assert_eq!(store.load().unwrap().as_str(), SECRET, "the right secret replaced the wrong one");
    assert_eq!((fake.proves.load(Ordering::SeqCst), fake.tickets.load(Ordering::SeqCst)), (1, 2), "the ticket with the wrong mac, a prove, a ticket");
}

#[tokio::test]
async fn refusals_and_a_broken_client_are_errors_that_keep_no_ticket() {
    let fake = FakeAuth::new(3600, vec!["version_not_allowed"]);
    let server = auth(&fake).await;
    let access = client(&server, Arc::new(MemoryStore::default()), Arc::new(AtomicUsize::new(0)));
    assert_eq!(access.renew().await, Err(AccessError::Refused("version_not_allowed".into())));
    assert_eq!(access.current(), None);

    let config = AccessConfig { auth_url: server.uri(), device_id: DEVICE.into(), platform: "linux".into() };
    let broken: HttpFactory = Arc::new(|| reqwest::Client::new().get("not a url").build().map(|_| reqwest::Client::new()));
    let access = AccessClient::tuned(config, key(), Arc::new(MemoryStore::default()), broken, fast(), wall);
    assert!(matches!(access.renew().await, Err(AccessError::Network(_))));
}

async fn until(what: &str, check: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !check() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{what} within 10 s"));
}

/// The ticket is renewed before it runs out (at 75 % of its lifetime), by one task.
#[tokio::test]
async fn the_background_task_renews_before_the_ticket_runs_out() {
    let fake = FakeAuth::new(1, Vec::new());
    let server = auth(&fake).await;
    let access = client(&server, Arc::new(MemoryStore::default()), Arc::new(AtomicUsize::new(0)));
    let task = access.spawn();
    until("three tickets", || fake.tickets.load(Ordering::SeqCst) >= 3).await;
    assert!(access.current().is_some());
    drop(access);
    tokio::time::timeout(Duration::from_secs(5), task).await.expect("the task ends with its client").unwrap();
}

/// `refresh_now` renews at once, but never sooner than `min_gap` after the last try, however
/// often it is asked.
#[tokio::test]
async fn refresh_now_renews_soon_and_is_paced() {
    let fake = FakeAuth::new(3600, Vec::new());
    let server = auth(&fake).await;
    let access = client(&server, Arc::new(MemoryStore::default()), Arc::new(AtomicUsize::new(0)));
    let _task = access.spawn();
    until("the first ticket", || access.current().is_some()).await;
    let started = Instant::now();
    for _ in 0..20 {
        access.refresh_now();
    }
    until("a second ticket", || fake.tickets.load(Ordering::SeqCst) >= 2).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(fake.tickets.load(Ordering::SeqCst) <= 3, "twenty asks are one or two renewals, not twenty");
    assert!(started.elapsed() < Duration::from_secs(5));
}

/// Failures wait longer each time, up to the most.
#[tokio::test]
async fn failures_back_off() {
    let fake = FakeAuth::new(3600, vec!["busy"; 20]);
    let server = auth(&fake).await;
    let access = client(&server, Arc::new(MemoryStore::default()), Arc::new(AtomicUsize::new(0)));
    let _task = access.spawn();
    tokio::time::sleep(Duration::from_millis(700)).await;
    let tries = fake.tickets.load(Ordering::SeqCst);
    // 50 + 100 + 200 + 200 ms … : about 4 or 5 tries in 700 ms, never one every few milliseconds.
    assert!((2..=7).contains(&tries), "{tries} tries");
}

#[test]
fn the_file_store_keeps_the_secret_privately() {
    let dir = std::env::temp_dir().join(format!("voltip-access-{}", uuid::Uuid::new_v4().simple()));
    let store = FileStore::new(dir.join("serve").join("access.json"));
    assert_eq!(store.load(), None);
    store.save(&DeviceSecret::new(SECRET)).unwrap();
    assert_eq!(store.load().unwrap().as_str(), SECRET);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(dir.join("serve").join("access.json")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    store.clear();
    assert_eq!(store.load(), None);
    let _ = std::fs::remove_dir_all(dir);
}

/// The core starts the renewal once, however often it is asked.
#[tokio::test]
async fn start_runs_one_renewal_task() {
    let fake = FakeAuth::new(3600, Vec::new());
    let server = auth(&fake).await;
    let access = client(&server, Arc::new(MemoryStore::default()), Arc::new(AtomicUsize::new(0)));
    access.start();
    access.start();
    until("a ticket", || access.current().is_some()).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(fake.tickets.load(Ordering::SeqCst), 1, "one task, one ticket");
}
