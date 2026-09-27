//! The connectivity self-check (`CoreCommand::CheckConnectivity`, [`crate::connectivity`]): the
//! relay and every LAN address a paired device announced are probed on a task (a fresh connection
//! and `hello` each, all at once); every online device gets an encrypted ping on its live channel.
//! The report goes out when both are in, or at [`CHECK_DEADLINE`] with what arrived.

use std::collections::HashMap;
use std::time::Instant;

use voltip_crypto::PublicKey;
use voltip_protocol::app::AppMessage;
use voltip_transport::{LinkConfig, RelayEndpoint};

use super::{CoreEvent, Runtime, now_ms, summarize};
use crate::CoreError;
use crate::connectivity::{
    AddressCheck, CHECK_DEADLINE, ConnectivityReport, ConnectivityStatus, LanHostCheck, PROBE_TIMEOUT, PeerCheck, ProbeResult, RelayCheck, same_subnet,
    shown_relay_result,
};
use crate::view::DeviceConnection;

/// What the probe task found.
#[derive(Debug)]
pub(super) struct Probed {
    /// The check it belongs to.
    check: u64,
    relay: Option<ProbeResult>,
    /// Every probed address, with the device that announced it.
    addresses: Vec<(PublicKey, String, ProbeResult)>,
}

/// A check in flight.
pub(super) struct Pending {
    id: u64,
    deadline: Instant,
    /// Encrypted pings awaiting their pong, by sequence number.
    pings: HashMap<u64, (PublicKey, Instant)>,
    rtts: HashMap<PublicKey, u32>,
    probed: Option<Probed>,
    /// The addresses the task was given, so a task that never reports still lists them.
    targets: Vec<(PublicKey, String)>,
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
        let config = |endpoint: RelayEndpoint, client_version: &str| {
            let mut cfg = LinkConfig::new(endpoint);
            cfg.client_version = client_version.to_owned();
            cfg.connect_timeout = PROBE_TIMEOUT;
            cfg
        };
        let relay = self.relay.as_ref().map(|(_, endpoint)| config(endpoint.clone(), &self.config.client_version));
        let mut targets = Vec::new();
        let mut probes = Vec::new();
        for device in self.trusted.list() {
            for hint in &device.direct_hints {
                targets.push((device.public_key, hint.clone()));
                probes.push((
                    device.public_key,
                    hint.clone(),
                    RelayEndpoint::parse(&format!("ws://{hint}/ws")).map(|ep| config(ep, &self.config.client_version)),
                ));
            }
        }
        let relay_configured = relay.is_some();
        let tx = self.check_tx.clone();
        tokio::spawn(async move {
            let relay = tokio::spawn(async move {
                match relay {
                    Some(cfg) => Some(ProbeResult::from_probe(voltip_transport::probe(&cfg).await)),
                    None => None,
                }
            });
            let mut set = tokio::task::JoinSet::new();
            for (i, (key, address, cfg)) in probes.into_iter().enumerate() {
                set.spawn(async move {
                    let result = match cfg {
                        Ok(cfg) => ProbeResult::from_probe(voltip_transport::probe(&cfg).await),
                        Err(e) => ProbeResult::Failed { reason: e.to_string() },
                    };
                    (i, key, address, result)
                });
            }
            let mut done = Vec::new();
            while let Some(joined) = set.join_next().await {
                if let Ok(probe) = joined {
                    done.push(probe);
                }
            }
            done.sort_by_key(|(i, ..)| *i);
            let addresses = done.into_iter().map(|(_, key, address, result)| (key, address, result)).collect();
            let relay = relay.await.unwrap_or(None);
            let _ = tx.send(Probed { check: id, relay, addresses }).await;
        });
        let mut pending =
            Pending { id, deadline: Instant::now() + CHECK_DEADLINE, pings: HashMap::new(), rtts: HashMap::new(), probed: None, targets, relay_configured };
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
        let own = voltip_transport::primary_lan_ip();
        let (relay, mut addresses) = match p.probed {
            Some(probed) => (probed.relay, probed.addresses),
            // The task never reported: every probe it held counts as unanswered.
            None => (p.relay_configured.then_some(ProbeResult::Timeout), p.targets.into_iter().map(|(k, a)| (k, a, ProbeResult::Timeout)).collect()),
        };
        let relay = shown_relay_result(relay, self.relay_status.source == crate::RelaySource::Builtin);
        let peers = self
            .trusted
            .list()
            .into_iter()
            .map(|device| {
                let via = match self.peers.get(&device.public_key).map(|st| summarize(&st.paths)) {
                    Some(DeviceConnection::Online { via }) => Some(via),
                    _ => None,
                };
                let checked = addresses
                    .iter_mut()
                    .filter(|(key, ..)| *key == device.public_key)
                    .map(|(_, address, result)| AddressCheck {
                        same_subnet: same_subnet(address, own),
                        address: std::mem::take(address),
                        result: std::mem::replace(result, ProbeResult::Timeout),
                    })
                    .collect();
                PeerCheck {
                    public_key: device.public_key.to_hex(),
                    name: device.name,
                    via,
                    rtt_ms: p.rtts.get(&device.public_key).copied(),
                    addresses: checked,
                }
            })
            .collect();
        let report = ConnectivityReport {
            checked_at: now_ms(),
            lan: LanHostCheck { listening: self.host.is_some(), addresses: self.lan_hints() },
            relay: RelayCheck { configured: p.relay_configured, result: relay },
            peers,
        };
        let relay_ok = matches!(report.relay.result, Some(ProbeResult::Ok { .. }));
        tracing::info!(peers = report.peers.len(), relay_configured = report.relay.configured, relay_ok, "connectivity check done");
        self.last_check = Some(report.clone());
        self.emit(CoreEvent::Connectivity(ConnectivityStatus { running: false, report: Some(report) }));
    }
}
