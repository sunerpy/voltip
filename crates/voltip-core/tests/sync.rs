#![allow(clippy::unwrap_used, clippy::expect_used)]
//! A computer's history and settings on its phones, and the phones' own records on the computer
//! (docs/dictation.md §20.8): real cores, a real relay, real sockets.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use uuid::Uuid;
use voltip_core::sync::{MirrorFiles, MirrorSyncState, MirrorView, SyncRole, SyncStats, read_copied_profile};
use voltip_core::{
    AppCore, CoreCommand, CoreConfig, CoreEvent, CoreHandle, DeviceConnection, HistoryEntry, HistoryQuery, HistoryStore, OriginKind, Settings, SettingsStore,
};
use voltip_identity::{MemorySecretStore, TrustedDevice};
use voltip_pairing::PairingState;
use voltip_relay::RelayConfig;
use voltip_relay::server::RelayHandle;
use voltip_transport::ConnectionState;

/// How long a wait below gives the state it waits for. A hang guard, not a speed check: CI's
/// coverage run is several times slower than a laptop, and the Windows build host took 2.4 times
/// as long as one for this file (2026-10-02).
const PATIENCE: Duration = Duration::from_secs(90);

struct Node {
    handle: CoreHandle,
    events: mpsc::Receiver<CoreEvent>,
    dir: tempfile::TempDir,
    store: Arc<MemorySecretStore>,
    mirrors: Arc<MirrorFiles>,
    stats: Arc<SyncStats>,
}

async fn relay() -> (String, tokio::sync::oneshot::Sender<()>, RelayHandle) {
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let handle = RelayHandle::new(RelayConfig::default());
    let (addr, _task) = handle
        .clone()
        .serve("127.0.0.1:0".parse().unwrap(), async move {
            let _ = stop_rx.await;
        })
        .await
        .unwrap();
    (format!("ws://{addr}/ws"), stop_tx, handle)
}

fn trace_init() {
    if std::env::var_os("E2E_TRACE").is_some() {
        let _ = tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::new("voltip=debug")).with_test_writer().try_init();
    }
}

/// A relay-only node (the LAN side off: one path per peer, the relay's).
fn start(dir: tempfile::TempDir, store: Arc<MemorySecretStore>, name: &str, url: &str, role: SyncRole, tune: impl FnOnce(&mut CoreConfig)) -> Node {
    trace_init();
    let settings = Settings { relay_url: Some(url.to_owned()), relay_enabled: true, ..Settings::default() };
    SettingsStore::new(dir.path()).save(&settings).unwrap();
    let mut cfg = CoreConfig::new(dir.path().to_path_buf());
    cfg.default_device_name = name.into();
    cfg.tick = Duration::from_millis(50);
    cfg.direct_enabled = false;
    // Six busy cores share one test process: a handshake may take longer than e2e.rs's 600 ms.
    cfg.peer_handshake_timeout = Duration::from_secs(5);
    cfg.sync_role = role;
    cfg.sync_request_timeout = Duration::from_secs(2);
    cfg.sync_upload_timeout = Duration::from_secs(2);
    tune(&mut cfg);
    let mirrors = cfg.mirror_files.clone();
    let stats = cfg.sync_stats.clone();
    let (handle, events) = AppCore::start(cfg, store.clone()).unwrap();
    Node { handle, events, dir, store, mirrors, stats }
}

fn node(name: &str, url: &str, role: SyncRole) -> Node {
    start(tempfile::tempdir().unwrap(), Arc::new(MemorySecretStore::new()), name, url, role, |_| {})
}

/// The same device started again, as after its process was killed: the old core is dropped
/// without a word, its identity and data stay.
fn restart(old: Node, name: &str, url: &str, role: SyncRole, tune: impl FnOnce(&mut CoreConfig)) -> Node {
    let Node { handle, events, dir, store, .. } = old;
    drop(handle);
    drop(events);
    start(dir, store, name, url, role, tune)
}

async fn wait<T>(node: &mut Node, mut pick: impl FnMut(&CoreEvent) -> Option<T>) -> T {
    wait_for(node, PATIENCE, &mut pick).await
}

async fn wait_for<T>(node: &mut Node, limit: Duration, pick: &mut impl FnMut(&CoreEvent) -> Option<T>) -> T {
    loop {
        let ev = tokio::time::timeout(limit, node.events.recv()).await.expect("event in time").expect("core alive");
        if std::env::var_os("E2E_TRACE").is_some() {
            eprintln!("[event] {ev:?}");
        }
        if let Some(v) = pick(&ev) {
            return v;
        }
    }
}

async fn connected(node: &mut Node) {
    wait(node, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Connected).then_some(())).await;
}

/// Pair by code; each side's record of the other.
async fn pair(desk: &mut Node, phone: &mut Node) -> (TrustedDevice, TrustedDevice) {
    desk.handle.send(CoreCommand::StartPairing).await.unwrap();
    let code = wait(desk, |e| match e {
        CoreEvent::Pairing(s) if s.state == PairingState::WaitingForPeer => s.code.clone(),
        _ => None,
    })
    .await;
    phone.handle.send(CoreCommand::JoinWithCode(code)).await.unwrap();
    for n in [&mut *desk, &mut *phone] {
        wait(n, |e| matches!(e, CoreEvent::Pairing(s) if s.state == PairingState::AwaitingVerification).then_some(())).await;
    }
    desk.handle.send(CoreCommand::ConfirmPairing).await.unwrap();
    phone.handle.send(CoreCommand::ConfirmPairing).await.unwrap();
    let td = wait(desk, |e| if let CoreEvent::Trusted(d) = e { Some(d.clone()) } else { None }).await;
    let tp = wait(phone, |e| if let CoreEvent::Trusted(d) = e { Some(d.clone()) } else { None }).await;
    desk.handle.send(CoreCommand::ResetPairing).await.unwrap();
    (td, tp)
}

fn entry(text: &str, at_ms: u64) -> HistoryEntry {
    serde_json::from_value(serde_json::json!({
        "id": Uuid::new_v4(), "at_ms": at_ms, "raw_text": text, "text": format!("{text}。"), "refined": true, "asr_model": "Qwen/Qwen3-ASR-1.7B",
        "duration_ms": 1500, "asr_ms": 400, "outcome": { "kind": "inserted", "via": "paste" },
        "segments": [{ "text": text, "start_ms": 0, "end_ms": 1500 }]
    }))
    .unwrap()
}

/// `n` entries a second apart with `size` characters of text each, written before the node starts.
fn seed(dir: &std::path::Path, n: usize, size: usize, label: &str) -> Vec<HistoryEntry> {
    let mut store = HistoryStore::open(dir);
    let entries: Vec<HistoryEntry> = (0..n).map(|i| entry(&format!("{label}{i} {}", "字".repeat(size)), 1_758_700_000_000 + i as u64 * 1000)).collect();
    for e in &entries {
        store.push(e.clone(), 20_000).unwrap();
    }
    entries
}

/// Wait until the phone's copy of `desk` passes `ok`.
async fn wait_mirror(phone: &mut Node, desk: &str, mut ok: impl FnMut(&MirrorView) -> bool) -> MirrorView {
    let desk = desk.to_owned();
    wait(phone, move |e| match e {
        CoreEvent::Mirrors(views) => views.iter().find(|v| v.desktop == desk && ok(v)).cloned(),
        _ => None,
    })
    .await
}

/// The ids and starred flags in the phone's copy of `desk`, newest first.
fn copy(phone: &Node, desk: &str) -> Vec<(Uuid, bool)> {
    let mut out = Vec::new();
    let mut offset = 0;
    loop {
        let page = phone.mirrors.read(desk, |reader, _| reader.query(&HistoryQuery { limit: 200, offset, ..HistoryQuery::default() })).unwrap();
        let Some(page) = page else { return out };
        let n = page.entries.len();
        out.extend(page.entries.into_iter().map(|e| (e.id, e.starred)));
        if n < 200 {
            return out;
        }
        offset += 200;
    }
}

/// The desk's history total as last reported.
async fn wait_total(node: &mut Node, total: u32) {
    wait(node, |e| matches!(e, CoreEvent::History { total: t, .. } if *t == total).then_some(())).await;
}

fn uploaded_rows(dir: &std::path::Path) -> usize {
    let conn = rusqlite::Connection::open(dir.join("history.sqlite3")).unwrap();
    conn.query_row("SELECT COUNT(*) FROM uploaded", [], |r| r.get::<_, i64>(0)).unwrap() as usize
}

#[tokio::test]
async fn the_computers_history_reaches_the_phone_and_follows_every_change() {
    let (url, _stop, relay) = relay().await;
    let dir = tempfile::tempdir().unwrap();
    let seeded = seed(dir.path(), 5, 10, "第");
    let mut desk = start(dir, Arc::new(MemorySecretStore::new()), "Desk", &url, SyncRole::Computer, |_| {});
    let mut phone = node("Pixel", &url, SyncRole::Phone);
    connected(&mut desk).await;
    connected(&mut phone).await;
    let (_, td) = pair(&mut desk, &mut phone).await;
    let key = td.public_key.to_hex();
    let view = wait_mirror(&mut phone, &key, |v| v.entries == 5 && v.state == MirrorSyncState::UpToDate).await;
    assert_eq!(view.name, "Desk");
    let ids: Vec<Uuid> = copy(&phone, &key).into_iter().map(|(id, _)| id).collect();
    assert_eq!(ids, seeded.iter().rev().map(|e| e.id).collect::<Vec<_>>(), "newest first, every entry");
    let first = phone.mirrors.read(&key, |_, path| voltip_core::sync::read_entry(path, seeded[0].id)).unwrap().unwrap().unwrap();
    assert!(first.0.segments.is_none(), "segments stay on the computer");
    assert_eq!(first.0.text, seeded[0].text);
    assert!(!first.1, "not shortened");
    assert!(view.profile.is_some(), "the settings came along");

    desk.handle.send(CoreCommand::HistoryStar(seeded[1].id, true)).await.unwrap();
    wait_mirror(&mut phone, &key, |v| v.state == MirrorSyncState::UpToDate && v.entries == 5).await;
    let deadline = tokio::time::Instant::now() + PATIENCE;
    while !copy(&phone, &key).contains(&(seeded[1].id, true)) {
        assert!(tokio::time::Instant::now() < deadline, "the star arrives");
        wait(&mut phone, |e| matches!(e, CoreEvent::Mirrors(_)).then_some(())).await;
    }
    desk.handle.send(CoreCommand::HistoryDelete(seeded[2].id)).await.unwrap();
    wait_mirror(&mut phone, &key, |v| v.entries == 4).await;
    assert!(!copy(&phone, &key).iter().any(|(id, _)| *id == seeded[2].id));
    desk.handle.send(CoreCommand::HistoryClear).await.unwrap();
    wait_mirror(&mut phone, &key, |v| v.entries == 0 && v.state == MirrorSyncState::UpToDate).await;
    assert_eq!(relay.stats().dropped, 0);
}

#[tokio::test]
async fn three_thousand_entries_arrive_in_batches() {
    let (url, _stop, _relay) = relay().await;
    let dir = tempfile::tempdir().unwrap();
    let seeded = seed(dir.path(), 3000, 300, "条");
    let mut desk = start(dir, Arc::new(MemorySecretStore::new()), "Desk", &url, SyncRole::Computer, |_| {});
    let mut phone = node("Pixel", &url, SyncRole::Phone);
    connected(&mut desk).await;
    connected(&mut phone).await;
    let (_, td) = pair(&mut desk, &mut phone).await;
    let key = td.public_key.to_hex();
    let mut sizes = Vec::new();
    wait_mirror(&mut phone, &key, |v| {
        sizes.push(v.entries);
        v.entries == 3000 && v.state == MirrorSyncState::UpToDate
    })
    .await;
    sizes.dedup();
    assert!(sizes.iter().filter(|n| **n > 0 && **n < 3000).count() >= 2, "several batches: {sizes:?}");
    let mut ids: Vec<Uuid> = copy(&phone, &key).into_iter().map(|(id, _)| id).collect();
    ids.sort_unstable();
    let mut want: Vec<Uuid> = seeded.iter().map(|e| e.id).collect();
    want.sort_unstable();
    assert_eq!(ids, want);
    assert!(phone.stats.max_in_flight.load(std::sync::atomic::Ordering::Relaxed) <= 4);
    assert!(desk.stats.max_in_flight.load(std::sync::atomic::Ordering::Relaxed) <= 4);
    assert!(desk.stats.max_in_flight.load(std::sync::atomic::Ordering::Relaxed) > 0, "the window was used");
}

#[tokio::test]
async fn the_computers_settings_reach_the_phone_and_the_phone_keeps_its_own_look() {
    let (url, _stop, _relay) = relay().await;
    let mut desk = node("Desk", &url, SyncRole::Computer);
    let mut phone = node("Pixel", &url, SyncRole::Phone);
    connected(&mut desk).await;
    connected(&mut phone).await;
    desk.handle.send(CoreCommand::SetTheme { theme: voltip_core::ThemeId::Graphite, follow_system: false }).await.unwrap();
    desk.handle.send(CoreCommand::SetLocale(voltip_core::Locale::En)).await.unwrap();
    desk.handle
        .send(CoreCommand::DictionaryAdd {
            draft: voltip_core::DictionaryDraft { term: "Voltip".into(), heard_as: vec!["沃尔提普".into()], enabled: true },
            source: voltip_core::EntrySource::Manual,
        })
        .await
        .unwrap();
    desk.handle.send(CoreCommand::PresetAdd(voltip_core::PresetDraft { name: "会议纪要".into(), prompt: "整理成要点。".into() })).await.unwrap();
    let (_, td) = pair(&mut desk, &mut phone).await;
    let key = td.public_key.to_hex();
    let view = wait_mirror(&mut phone, &key, |v| v.profile.as_ref().is_some_and(|p| p.theme == voltip_core::ThemeId::Graphite)).await;
    let summary = view.profile.unwrap();
    assert_eq!(summary.locale, voltip_core::Locale::En);
    assert!(summary.presets.is_empty() && summary.dictionary.is_empty(), "the view leaves the lists out");
    let profile = phone.mirrors.read(&key, |_, path| read_copied_profile(path)).unwrap().unwrap().unwrap();
    assert_eq!(profile.dictionary.iter().map(|d| d.term.as_str()).collect::<Vec<_>>(), ["Voltip"]);
    assert!(profile.presets.iter().any(|p| p.name == "会议纪要"));
    // A change on the computer reaches the phone.
    desk.handle
        .send(CoreCommand::DictionaryAdd {
            draft: voltip_core::DictionaryDraft { term: "Codex".into(), heard_as: Vec::new(), enabled: true },
            source: voltip_core::EntrySource::Manual,
        })
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + PATIENCE;
    loop {
        let terms: Vec<String> = phone
            .mirrors
            .read(&key, |_, path| read_copied_profile(path))
            .unwrap()
            .flatten()
            .map(|p| p.dictionary.into_iter().map(|d| d.term).collect())
            .unwrap_or_default();
        if terms.len() == 2 {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "the new term arrives: {terms:?}");
        wait(&mut phone, |e| matches!(e, CoreEvent::Mirrors(_)).then_some(())).await;
    }
    // The phone's own look is its own.
    let own = SettingsStore::new(phone.dir.path()).load().unwrap();
    assert_eq!((own.theme, own.locale), (voltip_core::ThemeId::Light, voltip_core::Locale::System));
}

#[tokio::test]
async fn the_phones_own_records_go_to_the_computer_as_copies() {
    let (url, _stop, _relay) = relay().await;
    let phone_dir = tempfile::tempdir().unwrap();
    let own = seed(phone_dir.path(), 3, 5, "手机上说的");
    let mut desk = node("Desk", &url, SyncRole::Computer);
    let mut phone = start(phone_dir, Arc::new(MemorySecretStore::new()), "Pixel", &url, SyncRole::Phone, |_| {});
    connected(&mut desk).await;
    connected(&mut phone).await;
    let (tp, td) = pair(&mut desk, &mut phone).await;
    wait_total(&mut desk, 3).await;
    let desk_store = HistoryStore::open(desk.dir.path());
    let copies = desk_store.recent(10);
    assert_eq!(copies.len(), 3);
    for c in &copies {
        let origin = c.origin.as_ref().unwrap();
        assert_eq!((origin.device.as_str(), origin.kind), ("Pixel", OriginKind::Standalone));
        assert!(c.segments.is_none());
    }
    drop(desk_store);
    // The phone keeps its records and remembers the computer has them.
    let deadline = tokio::time::Instant::now() + PATIENCE;
    while uploaded_rows(phone.dir.path()) < 3 {
        assert!(tokio::time::Instant::now() < deadline, "the confirmation is recorded");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(HistoryStore::open(phone.dir.path()).total(), 3);
    // The copies come back with the computer's history.
    let key = td.public_key.to_hex();
    wait_mirror(&mut phone, &key, |v| v.entries == 3).await;
    // Again online: nothing is uploaded twice. The phone deletes one of its own; the copy stays.
    phone.handle.send(CoreCommand::SetRelay { url: Some(url.clone()), enabled: false }).await.unwrap();
    wait(&mut desk, |e| matches!(e, CoreEvent::Devices(l) if l.iter().all(|d| d.connection == DeviceConnection::Offline)).then_some(())).await;
    phone.handle.send(CoreCommand::HistoryDelete(own[0].id)).await.unwrap();
    phone.handle.send(CoreCommand::SetRelay { url: Some(url.clone()), enabled: true }).await.unwrap();
    wait_mirror(&mut phone, &key, |v| v.state == MirrorSyncState::UpToDate && v.entries == 3).await;
    assert_eq!(HistoryStore::open(desk.dir.path()).total(), 3, "no second upload, the copy stays");
    // The computer deletes a copy: the phone's record stays and is not uploaded again.
    desk.handle.send(CoreCommand::HistoryDelete(own[1].id)).await.unwrap();
    wait_mirror(&mut phone, &key, |v| v.entries == 2).await;
    assert_eq!(HistoryStore::open(phone.dir.path()).total(), 2);
    // Nothing to wait for: give an upload that should not happen a few ticks (50 ms each) to start.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(HistoryStore::open(desk.dir.path()).total(), 2);
    let _ = tp;
}

#[tokio::test]
async fn a_computer_that_does_not_sync_yet_is_named_and_left_alone() {
    let (url, _stop, _relay) = relay().await;
    let mut desk = node("Old-Desk", &url, SyncRole::Off);
    let phone_dir = tempfile::tempdir().unwrap();
    seed(phone_dir.path(), 2, 5, "本机");
    let mut phone = start(phone_dir, Arc::new(MemorySecretStore::new()), "Pixel", &url, SyncRole::Phone, |_| {});
    connected(&mut desk).await;
    connected(&mut phone).await;
    let (_, td) = pair(&mut desk, &mut phone).await;
    let key = td.public_key.to_hex();
    wait_mirror(&mut phone, &key, |v| v.state == MirrorSyncState::NeedsUpgrade).await;
    // Nothing to wait for: give an upload that should not happen a few ticks (50 ms each) to start.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(HistoryStore::open(desk.dir.path()).total(), 0, "nothing uploaded to a computer that does not sync");
    assert!(!phone.mirrors.path(&key).exists(), "no copy");
}

/// The ids in a node's own history.
fn own_ids(dir: &std::path::Path) -> Vec<Uuid> {
    HistoryStore::open(dir).recent(20_000).into_iter().map(|e| e.id).collect()
}

/// Wait until `cond` holds, re-checking on every event of `node` (at most [`PATIENCE`]).
async fn until(node: &mut Node, what: &str, mut cond: impl FnMut(&Node) -> bool) {
    let deadline = tokio::time::Instant::now() + PATIENCE;
    while !cond(node) {
        assert!(tokio::time::Instant::now() < deadline, "{what}");
        let _ = tokio::time::timeout(Duration::from_millis(200), node.events.recv()).await;
    }
}

#[tokio::test]
async fn a_phone_killed_mid_transfer_completes_without_duplicates() {
    let (url, _stop, relay) = relay().await;
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path(), 3000, 300, "条");
    let mut desk = start(dir, Arc::new(MemorySecretStore::new()), "Desk", &url, SyncRole::Computer, |_| {});
    let mut phone = node("Pixel", &url, SyncRole::Phone);
    connected(&mut desk).await;
    connected(&mut phone).await;
    let (_, td) = pair(&mut desk, &mut phone).await;
    let key = td.public_key.to_hex();
    wait_mirror(&mut phone, &key, |v| v.entries > 0 && v.entries < 3000).await;
    // As if the process were killed: the core is gone mid-transfer, its files stay.
    let mut phone = restart(phone, "Pixel", &url, SyncRole::Phone, |_| {});
    wait_mirror(&mut phone, &key, |v| v.entries == 3000 && v.state == MirrorSyncState::UpToDate).await;
    let ids = copy(&phone, &key);
    let mut unique: Vec<Uuid> = ids.iter().map(|(id, _)| *id).collect();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), 3000, "no duplicates");
    assert_eq!(relay.stats().dropped, 0);
}

/// Drop the first `n` messages `which` picks (the test's transport losing them).
fn drop_first(n: usize, which: fn(&voltip_protocol::app::AppMessage) -> bool) -> voltip_core::TestHooks {
    let left = Arc::new(std::sync::atomic::AtomicUsize::new(n));
    voltip_core::TestHooks {
        drop_app: Some(Arc::new(move |msg| {
            which(msg) && left.fetch_update(std::sync::atomic::Ordering::SeqCst, std::sync::atomic::Ordering::SeqCst, |l| l.checked_sub(1)).is_ok()
        })),
        ..voltip_core::TestHooks::default()
    }
}

fn is_records_ack(msg: &voltip_protocol::app::AppMessage) -> bool {
    matches!(msg, voltip_protocol::app::AppMessage::PhoneRecordsAck { .. })
}

#[tokio::test]
async fn a_lost_confirmation_is_answered_again_without_a_second_copy() {
    let (url, _stop, _relay) = relay().await;
    let phone_dir = tempfile::tempdir().unwrap();
    seed(phone_dir.path(), 3, 5, "手机");
    let mut desk = start(tempfile::tempdir().unwrap(), Arc::new(MemorySecretStore::new()), "Desk", &url, SyncRole::Computer, |cfg| {
        cfg.test_hooks = drop_first(1, is_records_ack)
    });
    let mut phone =
        start(phone_dir, Arc::new(MemorySecretStore::new()), "Pixel", &url, SyncRole::Phone, |cfg| cfg.sync_upload_timeout = Duration::from_millis(500));
    connected(&mut desk).await;
    connected(&mut phone).await;
    pair(&mut desk, &mut phone).await;
    wait_total(&mut desk, 3).await;
    until(&mut phone, "the second confirmation is recorded", |n| uploaded_rows(n.dir.path()) == 3).await;
    assert_eq!(HistoryStore::open(desk.dir.path()).total(), 3, "the batch sent again wrote nothing");
}

/// regression (plan gate, M7 design): the confirmation was lost and the computer deleted the record
/// meanwhile; the phone sends it again, and it must not come back.
#[tokio::test]
async fn regression_a_record_deleted_while_its_confirmation_was_lost_stays_deleted() {
    let (url, _stop, _relay) = relay().await;
    let phone_dir = tempfile::tempdir().unwrap();
    let own = seed(phone_dir.path(), 3, 5, "手机");
    let mut desk = start(tempfile::tempdir().unwrap(), Arc::new(MemorySecretStore::new()), "Desk", &url, SyncRole::Computer, |cfg| {
        cfg.test_hooks = drop_first(1, is_records_ack)
    });
    let mut phone =
        start(phone_dir, Arc::new(MemorySecretStore::new()), "Pixel", &url, SyncRole::Phone, |cfg| cfg.sync_upload_timeout = Duration::from_millis(500));
    connected(&mut desk).await;
    connected(&mut phone).await;
    pair(&mut desk, &mut phone).await;
    wait_total(&mut desk, 3).await;
    desk.handle.send(CoreCommand::HistoryDelete(own[0].id)).await.unwrap();
    wait_total(&mut desk, 2).await;
    until(&mut phone, "the phone records the confirmation of the batch sent again", |n| uploaded_rows(n.dir.path()) == 3).await;
    assert!(!own_ids(desk.dir.path()).contains(&own[0].id), "not written back");
    assert_eq!(HistoryStore::open(desk.dir.path()).total(), 2);
}

#[tokio::test]
async fn the_sync_switch_deletes_the_copy_and_brings_it_back() {
    let (url, _stop, _relay) = relay().await;
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path(), 4, 5, "电脑");
    let phone_dir = tempfile::tempdir().unwrap();
    seed(phone_dir.path(), 1, 5, "手机");
    let mut desk = start(dir, Arc::new(MemorySecretStore::new()), "Desk", &url, SyncRole::Computer, |_| {});
    let mut phone = start(phone_dir, Arc::new(MemorySecretStore::new()), "Pixel", &url, SyncRole::Phone, |_| {});
    connected(&mut desk).await;
    connected(&mut phone).await;
    let (tp, td) = pair(&mut desk, &mut phone).await;
    let key = td.public_key.to_hex();
    wait_mirror(&mut phone, &key, |v| v.entries == 5 && v.state == MirrorSyncState::UpToDate).await;
    desk.handle.send(CoreCommand::SetDeviceSync { key: tp.public_key, on: false }).await.unwrap();
    let off = wait(&mut desk, |e| match e {
        CoreEvent::Devices(l) => l.iter().find(|d| d.device.public_key == tp.public_key && !d.device.sync).map(|d| d.device.clone()),
        _ => None,
    })
    .await;
    assert_eq!(off.sync_gen, 1);
    wait_mirror(&mut phone, &key, |v| v.state == MirrorSyncState::Revoked && v.entries == 0).await;
    assert!(!phone.mirrors.path(&key).exists(), "the copy is deleted");
    // A new record on the phone is not uploaded while the computer does not sync.
    // (The phone's records are seeded; a fresh one would come from a take.)
    desk.handle.send(CoreCommand::SetDeviceSync { key: tp.public_key, on: true }).await.unwrap();
    wait_mirror(&mut phone, &key, |v| v.entries == 5 && v.state == MirrorSyncState::UpToDate).await;
    assert_eq!(HistoryStore::open(desk.dir.path()).total(), 5);
}

/// regression (plan gate, M7 design): the switch messages may arrive out of order. The revoke of
/// turning sync off is lost; the phone only hears that sync is on again, and ends up syncing.
#[tokio::test]
async fn regression_a_lost_revoke_leaves_the_phone_in_the_newest_state() {
    let (url, _stop, _relay) = relay().await;
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path(), 2, 5, "电脑");
    let revoke = |m: &voltip_protocol::app::AppMessage| matches!(m, voltip_protocol::app::AppMessage::MirrorRevoke { .. });
    let mut desk = start(dir, Arc::new(MemorySecretStore::new()), "Desk", &url, SyncRole::Computer, |cfg| cfg.test_hooks = drop_first(1, revoke));
    let mut phone = node("Pixel", &url, SyncRole::Phone);
    connected(&mut desk).await;
    connected(&mut phone).await;
    let (tp, td) = pair(&mut desk, &mut phone).await;
    let key = td.public_key.to_hex();
    wait_mirror(&mut phone, &key, |v| v.entries == 2 && v.state == MirrorSyncState::UpToDate).await;
    desk.handle.send(CoreCommand::SetDeviceSync { key: tp.public_key, on: false }).await.unwrap();
    desk.handle.send(CoreCommand::SetDeviceSync { key: tp.public_key, on: true }).await.unwrap();
    wait(&mut desk, |e| matches!(e, CoreEvent::Devices(l) if l.iter().any(|d| d.device.sync_gen == 2 && d.device.sync)).then_some(())).await;
    desk.handle.send(CoreCommand::HistoryClear).await.unwrap();
    wait_mirror(&mut phone, &key, |v| v.entries == 0 && v.state == MirrorSyncState::UpToDate).await;
}

/// regression (plan gate, M7 design round 3): pairing the same phone again raises the generation, the
/// phone starts over from 0 and accepts what the computer sends from then on.
#[tokio::test]
async fn regression_pairing_again_starts_the_copy_over() {
    let (url, _stop, _relay) = relay().await;
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path(), 3, 5, "电脑");
    let mut desk = start(dir, Arc::new(MemorySecretStore::new()), "Desk", &url, SyncRole::Computer, |_| {});
    let mut phone = node("Pixel", &url, SyncRole::Phone);
    connected(&mut desk).await;
    connected(&mut phone).await;
    let (tp, td) = pair(&mut desk, &mut phone).await;
    let key = td.public_key.to_hex();
    wait_mirror(&mut phone, &key, |v| v.entries == 3).await;
    desk.handle.send(CoreCommand::SetDeviceSync { key: tp.public_key, on: false }).await.unwrap();
    wait_mirror(&mut phone, &key, |v| v.state == MirrorSyncState::Revoked).await;
    let (again, _) = pair(&mut desk, &mut phone).await;
    assert!(again.sync, "a re-pair syncs again");
    assert_eq!(again.sync_gen, 2, "the generation went up past the switch's");
    wait_mirror(&mut phone, &key, |v| v.entries == 3 && v.state == MirrorSyncState::UpToDate).await;
    desk.handle.send(CoreCommand::HistoryClear).await.unwrap();
    wait_mirror(&mut phone, &key, |v| v.entries == 0 && v.state == MirrorSyncState::UpToDate).await;
    // Forget, then pair: a new record starts from 0 and is accepted as well.
    desk.handle.send(CoreCommand::ForgetDevice(tp.public_key)).await.unwrap();
    wait(&mut phone, |e| matches!(e, CoreEvent::Unpaired(_)).then_some(())).await;
    assert!(!phone.mirrors.path(&key).exists(), "forgetting deletes the copy");
    // Forgetting leaves both on their relay channel (the relay has no way out of one but closing
    // the connection, an existing limitation): reconnect both, as any later start would.
    for n in [&mut desk, &mut phone] {
        n.handle.send(CoreCommand::SetRelay { url: Some(url.clone()), enabled: false }).await.unwrap();
        n.handle.send(CoreCommand::SetRelay { url: Some(url.clone()), enabled: true }).await.unwrap();
        connected(n).await;
    }
    let (fresh, _) = pair(&mut desk, &mut phone).await;
    assert_eq!(fresh.sync_gen, 0);
    wait_mirror(&mut phone, &key, |v| v.state == MirrorSyncState::UpToDate).await;
}

/// regression (M7 design): the oldest record too large to upload does not hold the others back.
#[tokio::test]
async fn regression_a_record_too_large_to_upload_holds_nothing_back() {
    let (url, _stop, _relay) = relay().await;
    let phone_dir = tempfile::tempdir().unwrap();
    let big = {
        let mut store = HistoryStore::open(phone_dir.path());
        let big = entry(&"大".repeat(2000), 1_758_600_000_000);
        store.push(big.clone(), 20_000).unwrap();
        big
    };
    let small = seed(phone_dir.path(), 2, 5, "小");
    let mut desk = node("Desk", &url, SyncRole::Computer);
    let mut phone = start(phone_dir, Arc::new(MemorySecretStore::new()), "Pixel", &url, SyncRole::Phone, |cfg| cfg.sync_max_entry_bytes = 4000);
    connected(&mut desk).await;
    connected(&mut phone).await;
    pair(&mut desk, &mut phone).await;
    let too_large = wait(&mut phone, |e| if let CoreEvent::PhoneOutbox { too_large } = e { Some(too_large.clone()) } else { None }).await;
    assert_eq!(too_large, [big.id]);
    wait_total(&mut desk, 2).await;
    let on_desk = own_ids(desk.dir.path());
    assert!(small.iter().all(|e| on_desk.contains(&e.id)) && !on_desk.contains(&big.id));
}

#[tokio::test]
async fn a_slow_phone_gets_everything_and_the_relay_drops_nothing() {
    let (url, _stop, relay) = relay().await;
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path(), 3000, 600, "慢");
    let mut desk = start(dir, Arc::new(MemorySecretStore::new()), "Desk", &url, SyncRole::Computer, |_| {});
    let mut phone = start(tempfile::tempdir().unwrap(), Arc::new(MemorySecretStore::new()), "Pixel", &url, SyncRole::Phone, |cfg| {
        cfg.sync_apply_delay = Duration::from_millis(150);
    });
    connected(&mut desk).await;
    connected(&mut phone).await;
    let (tp, td) = pair(&mut desk, &mut phone).await;
    let key = td.public_key.to_hex();
    wait_mirror(&mut phone, &key, |v| v.entries == 3000 && v.state == MirrorSyncState::UpToDate).await;
    assert_eq!(relay.stats().dropped, 0, "the windows kept the relay's queue from overflowing");
    // The sessions are intact: plain messages still go through, both ways.
    desk.handle.send(CoreCommand::SendText { to: tp.public_key, body: "之后的消息".into() }).await.unwrap();
    assert_eq!(wait(&mut phone, |e| if let CoreEvent::Message { body, .. } = e { Some(body.clone()) } else { None }).await, "之后的消息");
    phone.handle.send(CoreCommand::SendText { to: td.public_key, body: "收到".into() }).await.unwrap();
    assert_eq!(wait(&mut desk, |e| if let CoreEvent::Message { body, .. } = e { Some(body.clone()) } else { None }).await, "收到");
}

/// regression (plan gate, M7 design): five phones upload at once to one computer through one relay
/// connection while it sends them its history. Every path keeps its window, so the connection never
/// holds more than 5 × 4 parts and the relay drops nothing.
#[tokio::test]
async fn regression_five_phones_at_once_stay_within_the_windows() {
    let (url, _stop, relay) = relay().await;
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path(), 400, 300, "电脑");
    let mut desk = start(dir, Arc::new(MemorySecretStore::new()), "Desk", &url, SyncRole::Computer, |_| {});
    connected(&mut desk).await;
    let mut phones = Vec::new();
    for i in 0..voltip_identity::MAX_SYNC_PEERS {
        let phone_dir = tempfile::tempdir().unwrap();
        seed(phone_dir.path(), 300, 300, &format!("手机{i}·"));
        // The phones stay offline from the sync until all are paired: the switch is on only once
        // each is trusted, so they start uploading together when the last one pairs.
        let mut phone = start(phone_dir, Arc::new(MemorySecretStore::new()), &format!("Phone-{i}"), &url, SyncRole::Phone, |_| {});
        connected(&mut phone).await;
        pair(&mut desk, &mut phone).await;
        phones.push(phone);
    }
    // Five cores' events pile up while the test watches one of them: look at the databases.
    let all = 400 + 300 * voltip_identity::MAX_SYNC_PEERS;
    until(&mut desk, "every phone's records arrive", |n| HistoryStore::open(n.dir.path()).total() == all).await;
    let desk_key = {
        let d = voltip_identity::TrustedDeviceStore::open(phones[0].dir.path()).unwrap();
        d.list()[0].public_key.to_hex()
    };
    for phone in &mut phones {
        until(phone, "the phone has the computer's whole history", |n| copy(n, &desk_key).len() == all).await;
        assert!(phone.stats.max_in_flight.load(std::sync::atomic::Ordering::Relaxed) <= 4);
    }
    assert!(desk.stats.max_in_flight.load(std::sync::atomic::Ordering::Relaxed) <= 4);
    assert_eq!(relay.stats().dropped, 0);
}

/// Relay and LAN both (the in-memory LAN finds the peer, the LAN host takes its dial).
fn relay_and_lan(lan: &voltip_core::discovery::fake::LocalLan) -> impl FnOnce(&mut CoreConfig) + use<> {
    let discovery: Arc<dyn voltip_core::discovery::Discovery> = lan.join();
    move |cfg: &mut CoreConfig| {
        cfg.direct_enabled = true;
        cfg.direct_bind = "127.0.0.1:0".parse().unwrap();
        cfg.direct_retry = Duration::from_millis(100);
        cfg.direct_retry_max = Duration::from_millis(400);
        cfg.direct_connect_timeout = Duration::from_secs(2);
        cfg.discovery = Some(discovery);
    }
}

/// regression (M7 design): with a LAN path and a relay path, a batch travels on the LAN; the LAN
/// path drops halfway, the request goes unanswered and is asked again, and the history completes
/// without a duplicate.
#[tokio::test]
async fn regression_a_lan_path_that_drops_mid_transfer_is_made_up() {
    let (url, _stop, relay) = relay().await;
    let lan = voltip_core::discovery::fake::LocalLan::default();
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path(), 3000, 300, "条");
    let mut desk = start(dir, Arc::new(MemorySecretStore::new()), "Desk", &url, SyncRole::Computer, relay_and_lan(&lan));
    let tune = relay_and_lan(&lan);
    let mut phone = start(tempfile::tempdir().unwrap(), Arc::new(MemorySecretStore::new()), "Pixel", &url, SyncRole::Phone, move |cfg| {
        tune(cfg);
        cfg.sync_request_timeout = Duration::from_secs(1);
        cfg.sync_apply_delay = Duration::from_millis(100);
    });
    connected(&mut desk).await;
    connected(&mut phone).await;
    let (_, td) = pair(&mut desk, &mut phone).await;
    let key = td.public_key.to_hex();
    wait(&mut phone, |e| match e {
        CoreEvent::Devices(l) => l.iter().any(|d| d.connection == DeviceConnection::Online { via: voltip_identity::ConnectionKind::Direct }).then_some(()),
        _ => None,
    })
    .await;
    wait_mirror(&mut phone, &key, |v| v.entries > 0 && v.entries < 3000).await;
    phone.handle.send(CoreCommand::DropDirectLinks).await.unwrap();
    until(&mut phone, "the history completes", |n| copy(n, &key).len() == 3000).await;
    let mut ids: Vec<Uuid> = copy(&phone, &key).into_iter().map(|(id, _)| id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 3000, "no duplicates");
    assert_eq!(relay.stats().dropped, 0);
}

/// The phone's relay connection goes and comes back (`restart`: its process does) while the
/// computer's relay link holds parts sealed for the old session: the relay keeps the channel and its
/// id, so those parts reach the phone's new connection ahead of the new handshake.
async fn stale_frames_at_reattach(restart_phone: bool) {
    let (url, _stop, relay) = relay().await;
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path(), 3000, 300, "条");
    let gate = Arc::new(tokio::sync::RwLock::new(()));
    let hooks = voltip_core::TestHooks { relay_write_gate: Some(gate.clone()), answer_delay: Duration::from_millis(500), ..voltip_core::TestHooks::default() };
    let mut desk = start(dir, Arc::new(MemorySecretStore::new()), "Desk", &url, SyncRole::Computer, move |cfg| cfg.test_hooks = hooks);
    let mut phone = start(tempfile::tempdir().unwrap(), Arc::new(MemorySecretStore::new()), "Pixel", &url, SyncRole::Phone, |cfg| {
        cfg.sync_request_timeout = Duration::from_secs(1)
    });
    connected(&mut desk).await;
    connected(&mut phone).await;
    let (_, td) = pair(&mut desk, &mut phone).await;
    let key = td.public_key.to_hex();
    // A batch is in; the next request is out and the computer answers it in 500 ms, behind the gate.
    wait_mirror(&mut phone, &key, |v| v.entries > 0 && v.entries < 3000).await;
    let held = gate.write().await;
    // The answer is sealed into the queue meanwhile; then the phone's connection goes.
    tokio::time::sleep(Duration::from_millis(700)).await;
    let mut phone = if restart_phone {
        restart(phone, "Pixel", &url, SyncRole::Phone, |cfg| cfg.sync_request_timeout = Duration::from_secs(1))
    } else {
        phone.handle.send(CoreCommand::SetRelay { url: Some(url.clone()), enabled: false }).await.unwrap();
        wait(&mut phone, |e| matches!(e, CoreEvent::Relay(r) if r.state == ConnectionState::Disconnected).then_some(())).await;
        phone.handle.send(CoreCommand::SetRelay { url: Some(url.clone()), enabled: true }).await.unwrap();
        phone
    };
    connected(&mut phone).await;
    // Give the new connection time to attach to the channel before the old frames come.
    tokio::time::sleep(Duration::from_millis(300)).await;
    drop(held);
    until(&mut phone, "the history completes after the stale frames", |n| copy(n, &key).len() == 3000).await;
    let mut ids: Vec<Uuid> = copy(&phone, &key).into_iter().map(|(id, _)| id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 3000, "no duplicates");
    assert_eq!(relay.stats().dropped, 0);
    // The new session works both ways.
    phone.handle.send(CoreCommand::SendText { to: td.public_key, body: "新会话".into() }).await.unwrap();
    assert_eq!(wait(&mut desk, |e| if let CoreEvent::Message { body, .. } = e { Some(body.clone()) } else { None }).await, "新会话");
}

/// regression (plan gate, M7 design round 6): frames of the ended session must neither add up
/// with the new one's nor break its handshake.
#[tokio::test]
async fn regression_stale_frames_at_reattach_do_not_break_the_new_session() {
    stale_frames_at_reattach(false).await;
}

/// regression (plan gate, M7 design round 7): the same after the phone's process restarted, when it
/// has nothing of the old session to recognise its frames by.
#[tokio::test]
async fn regression_stale_frames_reach_a_restarted_phone_without_harm() {
    stale_frames_at_reattach(true).await;
}
