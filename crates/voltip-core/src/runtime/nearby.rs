//! LAN discovery in the runtime (docs/pairing.md 「局域网发现」): announce this device, keep what
//! the browse sees, dial a trusted device where it is seen, and join a pairing desktop the phone
//! picked from 「附近的电脑」. The mDNS port itself is [`crate::discovery`].

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use voltip_crypto::PublicKey;
use voltip_protocol::MAX_DIRECT_HINTS;
use voltip_protocol::ticket::PairingTicket;

use super::{CoreEvent, JoinSpec, Pairing, Runtime};
use crate::CoreError;
use crate::discovery::{Announcement, Discovery, DiscoveryEvent, NearbyDevice, Sighting, key_tag};

/// What the runtime keeps about the LAN.
#[derive(Default)]
pub(super) struct Lan {
    /// Browsing and announcing.
    running: bool,
    announced: Option<Announcement>,
    seen: BTreeMap<String, Sighting>,
    /// A trusted device's addresses as last seen, dialled before its stored hints.
    pub(super) hints: HashMap<PublicKey, Vec<String>>,
    /// The list last sent to the UI.
    listed: Option<Vec<NearbyDevice>>,
}

impl Runtime {
    fn discovery_port(&self) -> Option<Arc<dyn Discovery>> {
        self.config.discovery.clone()
    }

    pub(super) fn lan_running(&self) -> bool {
        self.lan.running
    }

    /// Browse and announce, when a port is configured, the setting allows it and the LAN host runs.
    pub(super) fn start_discovery(&mut self) {
        let Some(port) = self.discovery_port() else { return };
        if self.lan.running || !self.settings.lan_discovery || self.host.is_none() {
            return;
        }
        if let Err(e) = port.browse(self.disc_tx.clone()) {
            tracing::warn!(error = %e, "LAN discovery unavailable");
            return;
        }
        self.lan.running = true;
        tracing::info!("LAN discovery on");
        self.refresh_announcement();
    }

    /// Stop announcing and browsing, and forget what was seen.
    pub(super) fn stop_discovery(&mut self) {
        let Some(port) = self.discovery_port() else { return };
        if !self.lan.running {
            return;
        }
        port.withdraw();
        port.stop_browsing();
        let listed = self.lan.listed.take();
        self.lan = Lan { listed, ..Lan::default() };
        tracing::info!("LAN discovery off");
        self.emit_nearby();
    }

    /// `SetLanDiscovery`: persist, then start or stop.
    pub(super) fn set_lan_discovery(&mut self, enabled: bool) -> Result<(), CoreError> {
        self.settings.lan_discovery = enabled;
        self.save_settings()?;
        if enabled {
            self.start_discovery()
        } else {
            self.stop_discovery()
        }
        Ok(())
    }

    /// What this device should announce now: its tag, name, platform and LAN port, and the
    /// pairing ticket while it waits for a peer. The ticket goes without its hints, and a flag
    /// says whether the session waits on the relay (a relay hint is how the ticket says so): the
    /// record fits one TXT string whatever relay is configured, and the relay's address is not
    /// broadcast to the LAN.
    fn announcement(&self) -> Option<Announcement> {
        let host = self.host.as_ref()?;
        let offer = match &self.pairing {
            Pairing::Initiator(i) => i.ticket().and_then(|t| {
                let bare = PairingTicket { relay_hint: None, direct_hints: Vec::new(), ..t.clone() };
                bare.to_uri().ok().map(|uri| (uri, t.relay_hint.is_some()))
            }),
            _ => None,
        };
        Some(Announcement {
            fingerprint: key_tag(&self.identity.keypair.public),
            name: self.identity.name.clone(),
            platform: self.identity.platform,
            port: host.host.local_addr().port(),
            on_relay: offer.as_ref().is_some_and(|(_, on_relay)| *on_relay),
            ticket: offer.map(|(uri, _)| uri),
        })
    }

    /// Announce again when anything announced changed (a rename, a pairing that started or ended);
    /// called after every event the runtime handles.
    pub(super) fn refresh_announcement(&mut self) {
        if !self.lan.running {
            return;
        }
        let Some(port) = self.discovery_port() else { return };
        let next = self.announcement();
        if next == self.lan.announced {
            return;
        }
        match &next {
            Some(a) => {
                if let Err(e) = port.announce(a) {
                    tracing::warn!(error = %e, "LAN announcement failed");
                    return;
                }
                tracing::debug!(port = a.port, pairing = a.ticket.is_some(), "announced on the LAN");
            }
            None => port.withdraw(),
        }
        self.lan.announced = next;
    }

    /// A browse event.
    pub(super) fn on_discovery(&mut self, event: DiscoveryEvent) {
        if !self.lan.running {
            return;
        }
        let own = key_tag(&self.identity.keypair.public);
        match event {
            DiscoveryEvent::Seen(s) if s.fingerprint == own => {}
            DiscoveryEvent::Seen(s) => {
                if let Some(d) = self.trusted.list().into_iter().find(|d| key_tag(&d.public_key) == s.fingerprint) {
                    let hints: Vec<String> = s.addrs.iter().map(ToString::to_string).collect();
                    if self.lan.hints.get(&d.public_key) != Some(&hints) {
                        tracing::info!(peer = %d.name, ?hints, "trusted device seen on the LAN");
                        self.lan.hints.insert(d.public_key, hints);
                        let initial = self.config.direct_retry;
                        if let Some(st) = self.peers.get_mut(&d.public_key)
                            && !st.has_active_direct_path()
                        {
                            st.reset_dial_backoff(initial);
                        }
                    }
                }
                if self.lan.seen.get(&s.fingerprint) != Some(&s) {
                    self.lan.seen.insert(s.fingerprint.clone(), s);
                    self.emit_nearby();
                }
            }
            DiscoveryEvent::Gone { fingerprint } => {
                if self.lan.seen.remove(&fingerprint).is_some() {
                    self.lan.hints.retain(|key, _| key_tag(key) != fingerprint);
                    self.emit_nearby();
                }
            }
        }
    }

    /// Send the nearby list again even though it did not change (`RefreshDevices`).
    pub(super) fn resend_nearby(&mut self) {
        if self.lan.running {
            self.lan.listed = None;
            self.emit_nearby();
        }
    }

    /// Send the nearby list to the UI when it changed (a sighting, or a device trusted or forgotten
    /// since); called after every event the runtime handles.
    pub(super) fn emit_nearby(&mut self) {
        let trusted: Vec<String> = self.trusted.list().iter().map(|d| key_tag(&d.public_key)).collect();
        let list: Vec<NearbyDevice> = self
            .lan
            .seen
            .values()
            .map(|s| NearbyDevice {
                fingerprint: s.fingerprint.clone(),
                name: s.name.clone(),
                platform: s.platform,
                pairing: s.ticket.is_some(),
                trusted: trusted.contains(&s.fingerprint),
            })
            .collect();
        if self.lan.listed.as_ref() == Some(&list) {
            return;
        }
        self.lan.listed = Some(list.clone());
        self.emit(CoreEvent::Nearby(list));
    }

    /// `PairingJoinNearby`: join the pairing the nearby desktop `fingerprint` waits for. Its ticket
    /// carries no hints: the addresses it was seen at stand in for the LAN ones, and a session
    /// that waits on the relay is met on this device's relay, as after a scan.
    pub(super) async fn join_nearby(&mut self, fingerprint: &str) -> Result<(), CoreError> {
        let Some(s) = self.lan.seen.get(fingerprint) else { return Err(CoreError::Invalid("pairing: 附近没有找到此设备".into())) };
        let Some(uri) = &s.ticket else { return Err(CoreError::Invalid("pairing: 此设备当前没有等待配对".into())) };
        let mut ticket = PairingTicket::from_uri(uri)?;
        ticket.direct_hints = s.addrs.iter().filter(|a| a.is_ipv4()).take(MAX_DIRECT_HINTS).map(ToString::to_string).collect();
        if s.on_relay {
            let relay = self.relay.as_ref().filter(|_| self.relay_connected()).map(|(_, endpoint)| endpoint.url().clone());
            let Some(url) = relay else { return Err(CoreError::Invalid("pairing: 此电脑正通过中继等待配对，但本机尚未连接中继".into())) };
            ticket.relay_hint = Some(url);
        } else if ticket.direct_hints.is_empty() {
            return Err(CoreError::Invalid("pairing: 此设备没有可用的 IPv4 地址".into()));
        }
        let uri = ticket.to_uri()?;
        self.join(JoinSpec::Ticket(uri)).await
    }
}
