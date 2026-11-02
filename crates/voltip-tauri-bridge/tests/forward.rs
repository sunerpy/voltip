#![allow(clippy::unwrap_used, clippy::expect_used)]
//! What the windows get (docs/dictation.md §33.2): every event in order, and after falling behind
//! the fan-out the whole state, so a window never keeps what the events it lost would have
//! changed (user report 2026-11-02: the relay shown 认证中 for good while it was connected).

use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use voltip_core::ui::{HardwareStatus, UiEvent, UpdateStatus};
use voltip_core::{CoreConfig, Settings, SettingsStore};
use voltip_identity::MemorySecretStore;
use voltip_tauri_bridge::Bridge;

/// More events than the fan-out keeps for a receiver that is behind.
const BURST: usize = 1_000;

#[tokio::test]
async fn regression_a_window_that_fell_behind_takes_the_whole_state_again() {
    let dir = tempfile::tempdir().unwrap();
    SettingsStore::new(dir.path()).save(&Settings { relay_enabled: false, ..Settings::default() }).unwrap();
    let bridge = Bridge::start(CoreConfig::new(dir.path().to_path_buf()), Arc::new(MemorySecretStore::new())).unwrap();
    let mut up = bridge.events();
    tokio::time::timeout(Duration::from_secs(10), async {
        while bridge.state().identity.is_none() {
            let _ = up.recv().await;
        }
    })
    .await
    .expect("the core is up within 10 s");

    // The windows' receiver, not read while a change they must see is followed by many more.
    let behind = bridge.events();
    bridge.publish(UiEvent::Hardware(HardwareStatus { cpu_threads: 7, gpus: Vec::new() }));
    for _ in 0..BURST {
        bridge.publish(UiEvent::Update(UpdateStatus::Checking));
    }
    bridge.publish(UiEvent::Update(UpdateStatus::Idle));

    let window = Arc::new(Mutex::new(Vec::<UiEvent>::new()));
    let seen = Arc::clone(&window);
    let forwarding = bridge.clone();
    let task = tokio::spawn(async move {
        forwarding
            .forward(behind, |event| {
                seen.lock().push(event.clone());
                Ok::<(), String>(())
            })
            .await;
    });
    tokio::time::timeout(Duration::from_secs(10), async {
        while !window.lock().iter().any(|e| matches!(e, UiEvent::Update(UpdateStatus::Idle))) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the last event reaches the window within 10 s");

    // What the window folds: a whole state replaces its own, an event sets its part.
    let mut cpu_threads = 0;
    for event in window.lock().iter() {
        match event {
            UiEvent::State(state) => cpu_threads = state.hardware.cpu_threads,
            UiEvent::Hardware(hardware) => cpu_threads = hardware.cpu_threads,
            _ => {}
        }
    }
    assert_eq!(cpu_threads, 7, "the window knows the machine the events it lost reported");
    task.abort();
    bridge.shutdown();
}

#[tokio::test]
async fn regression_an_event_a_window_could_not_get_is_followed_by_the_whole_state() {
    // Tauri sends an event to the windows one after the other and stops at the first that fails:
    // the windows after it never get that event.
    let dir = tempfile::tempdir().unwrap();
    SettingsStore::new(dir.path()).save(&Settings { relay_enabled: false, ..Settings::default() }).unwrap();
    let bridge = Bridge::start(CoreConfig::new(dir.path().to_path_buf()), Arc::new(MemorySecretStore::new())).unwrap();
    let events = bridge.events();
    let window = Arc::new(Mutex::new(Vec::<UiEvent>::new()));
    let seen = Arc::clone(&window);
    let forwarding = bridge.clone();
    let task = tokio::spawn(async move {
        let mut failed_once = false;
        forwarding
            .forward(events, |event| {
                if !failed_once && matches!(event, UiEvent::Hardware(h) if h.cpu_threads == 7) {
                    failed_once = true;
                    return Err("the event loop is gone".to_owned());
                }
                seen.lock().push(event.clone());
                Ok(())
            })
            .await;
    });
    // The core's first state has reached the window: what comes now is the lost event's.
    tokio::time::timeout(Duration::from_secs(10), async {
        while !window.lock().iter().any(|e| matches!(e, UiEvent::State(_))) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the core is up within 10 s");
    let before = window.lock().len();
    bridge.publish(UiEvent::Hardware(HardwareStatus { cpu_threads: 7, gpus: Vec::new() }));
    tokio::time::timeout(Duration::from_secs(10), async {
        while !window.lock()[before..].iter().any(|e| matches!(e, UiEvent::State(state) if state.hardware.cpu_threads == 7)) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the whole state, with the machine the lost event reported, within 10 s");
    task.abort();
    bridge.shutdown();
}
