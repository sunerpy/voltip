//! The connectivity self-check (`CoreCommand::CheckConnectivity`, [`crate::connectivity`]): the
//! relay is probed on a task (a fresh connection and `hello`); every online device gets an
//! encrypted ping on its channel. The report goes out when both are in, or at [`CHECK_DEADLINE`]
//! with what arrived.

use std::collections::HashMap;
use std::time::Instant;

use voltip_crypto::PublicKey;
use voltip_protocol::app::AppMessage;
use voltip_transport::LinkConfig;

use super::{CoreEvent, Runtime, now_ms, summarize};
use crate::CoreError;
use crate::connectivity::{CHECK_DEADLINE, ConnectivityReport, ConnectivityStatus, PROBE_TIMEOUT, PeerCheck, ProbeResult, RelayCheck, shown_relay_result};
use crate::view::DeviceConnection;

/// What the probe task found.
#[derive(Debug)]
pub(super) struct Probed {
    /// The check it belongs to.
    check: u64,
    relay: Option<ProbeResult>,
}

/// A check in flight.
pub(super) struct Pending {
    id: u64,
    deadline: Instant,
    /// Encrypted pings awaiting their pong, by sequence number.
    pings: HashMap<u64, (PublicKey, Instant)>,
    rtts: HashMap<PublicKey, u32>,
    probed: Option<Probed>,
    relay_configured: bool,
}

impl Runtime {
    /// Start a check; refused while one runs.
    pub(super) async fn check_connectivity(&mut self) -> Result<(), CoreError> {
        if self.check.is_some() {
            return Err(CoreError::Invalid("connectivity: 自检正在进行".into()));
        }
        self.next_check = self.next_check.wrapping_add(1);
        let id = self.next_check;
        let relay = self.relay.as_ref().map(|(_, endpoint)| {
            let mut cfg = LinkConfig::new(endpoint.clone());
            cfg.client_version = self.config.client_version.clone();
            cfg.connect_timeout = PROBE_TIMEOUT;
            cfg
        });
        let relay_configured = relay.is_some();
        let tx = self.check_tx.clone();
        tokio::spawn(async move {
            let relay = match relay {
                Some(cfg) => Some(ProbeResult::from_probe(voltip_transport::probe(&cfg).await)),
                None => None,
            };
            let _ = tx.send(Probed { check: id, relay }).await;
        });
        // The relay link checks its own socket at once too: a check is often asked for right
        // after something went wrong (docs/pairing.md 「重连」).
        if let Some((link, _)) = &self.relay {
            link.reconnect_now();
        }
        let mut pending =
            Pending { id, deadline: Instant::now() + CHECK_DEADLINE, pings: HashMap::new(), rtts: HashMap::new(), probed: None, relay_configured };
        let online: Vec<PublicKey> = self.peers.keys().copied().filter(|key| self.trusted.get_by_key(key).is_some()).collect();
        for (i, key) in online.into_iter().enumerate() {
            let seq = (id << 16) | (i as u64 & 0xffff);
            if self.send_app(key, &AppMessage::ping(seq)).await.is_ok() {
                pending.pings.insert(seq, (key, Instant::now()));
            }
        }
        self.check = Some(pending);
        self.emit(CoreEvent::Connectivity(ConnectivityStatus { running: true, report: self.last_check.clone() }));
        Ok(())
    }

    /// A pong from `key`: the round trip of the check's ping.
    pub(super) fn on_pong(&mut self, key: PublicKey, seq: u64) {
        let Some(p) = self.check.as_mut() else { return };
        if let Some((from, sent)) = p.pings.remove(&seq)
            && from == key
        {
            p.rtts.insert(key, u32::try_from(sent.elapsed().as_millis()).unwrap_or(u32::MAX));
        }
        self.finish_check_if_complete();
    }

    /// The probe task reported.
    pub(super) fn on_probed(&mut self, probed: Probed) {
        let Some(p) = self.check.as_mut().filter(|p| p.id == probed.check) else { return };
        p.probed = Some(probed);
        self.finish_check_if_complete();
    }

    /// Once per tick: a check past its deadline reports what it has.
    pub(super) fn check_deadline(&mut self) {
        if self.check.as_ref().is_some_and(|p| Instant::now() >= p.deadline) {
            self.finish_check();
        }
    }

    fn finish_check_if_complete(&mut self) {
        if self.check.as_ref().is_some_and(|p| p.probed.is_some() && p.pings.is_empty()) {
            self.finish_check();
        }
    }

    fn finish_check(&mut self) {
        let Some(p) = self.check.take() else { return };
        let relay = match p.probed {
            Some(probed) => probed.relay,
            // The task never reported: the probe counts as unanswered.
            None => p.relay_configured.then_some(ProbeResult::Timeout),
        };
        let relay = shown_relay_result(relay, self.relay_status.source == crate::RelaySource::Builtin);
        let peers = self
            .trusted
            .list()
            .into_iter()
            .map(|device| PeerCheck {
                public_key: device.public_key.to_hex(),
                name: device.name,
                online: self.peers.get(&device.public_key).is_some_and(|st| summarize(&st.paths) == DeviceConnection::Online),
                rtt_ms: p.rtts.get(&device.public_key).copied(),
            })
            .collect();
        let report = ConnectivityReport { checked_at: now_ms(), relay: RelayCheck { configured: p.relay_configured, result: relay }, peers };
        let relay_ok = matches!(report.relay.result, Some(ProbeResult::Ok { .. }));
        tracing::info!(peers = report.peers.len(), relay_configured = report.relay.configured, relay_ok, "connectivity check done");
        self.last_check = Some(report.clone());
        self.emit(CoreEvent::Connectivity(ConnectivityStatus { running: false, report: Some(report) }));
    }
}
