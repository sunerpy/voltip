#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The core's events to the interface are never dropped (docs/dictation.md §33.2): a shell that
//! reads them late still folds them into the state the core has. Until 2026-11-02 the queue held
//! 512 events and dropped what did not fit, so the shell's state, and every window's, kept what
//! the lost events would have changed: the relay at 认证中 while it was connected among them (user
//! report 2026-11-02).

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use voltip_core::ui::UiState;
use voltip_core::{AppCore, CoreCommand, CoreConfig, CoreEvent, Settings, SettingsStore, ThemeId};
use voltip_identity::MemorySecretStore;
use voltip_relay::RelayConfig;
use voltip_relay::server::RelayHandle;
use voltip_transport::ConnectionState;

/// Far more events than the queue held before the fix.
const BURST: usize = 2_000;
const STEP: Duration = Duration::from_secs(30);

#[tokio::test]
async fn regression_the_relay_connecting_while_the_interface_is_behind_still_reaches_it() {
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let relay = RelayHandle::new(RelayConfig::default());
    let (addr, _task) = relay
        .clone()
        .serve("127.0.0.1:0".parse().unwrap(), async move {
            let _ = stop_rx.await;
        })
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    SettingsStore::new(dir.path()).save(&Settings { relay_enabled: false, ..Settings::default() }).unwrap();
    let mut cfg = CoreConfig::new(dir.path().to_path_buf());
    cfg.default_device_name = "Behind".into();
    cfg.tick = Duration::from_millis(50);
    let (handle, mut events) = AppCore::start(cfg, Arc::new(MemorySecretStore::new())).unwrap();

    // The interface reads nothing while the core says a great deal, then the relay is switched on.
    for i in 0..BURST {
        let theme = if i % 2 == 0 { ThemeId::Graphite } else { ThemeId::Warm };
        handle.send(CoreCommand::SetTheme { theme, follow_system: false }).await.unwrap();
    }
    handle.send(CoreCommand::SetRelay { url: Some(format!("ws://{addr}/ws")), enabled: true }).await.unwrap();
    // Connected for sure once a pairing session exists on the relay: the core opens one only over
    // a connected link. Still nothing is read.
    tokio::time::timeout(STEP, async {
        while relay.stats().sessions == 0 {
            handle.send(CoreCommand::StartPairing).await.unwrap();
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .expect("a pairing session on the relay within 30 s");
    handle.send(CoreCommand::SetTheme { theme: ThemeId::Dark, follow_system: false }).await.unwrap();

    // Now the interface reads everything, folding it as the bridge does, up to the last change.
    let mut state = UiState::default();
    let mut themes = 0;
    tokio::time::timeout(STEP, drain(&mut events, &mut state, &mut themes)).await.expect("the last change within 30 s");
    assert_eq!(state.relay.state, ConnectionState::Connected, "the interface shows the relay connected: {:?}", state.relay);
    assert_eq!(themes, BURST + 2, "every settings change reached the interface (the relay's too)");
    assert_eq!(state.settings.theme, ThemeId::Dark);

    handle.send(CoreCommand::CancelPairing).await.unwrap();
    handle.send(CoreCommand::Shutdown).await.unwrap();
    let _ = stop_tx.send(());
}

/// Fold events into `state` until the settings say `Dark`, counting the settings events.
async fn drain(events: &mut mpsc::Receiver<CoreEvent>, state: &mut UiState, themes: &mut usize) {
    loop {
        let event = events.recv().await.expect("core alive");
        if matches!(event, CoreEvent::Settings(_)) {
            *themes += 1;
        }
        let done = matches!(&event, CoreEvent::Settings(s) if s.theme == ThemeId::Dark);
        state.apply(event);
        if done {
            return;
        }
    }
}
