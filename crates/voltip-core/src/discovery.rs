//! Finding devices on the LAN without typing anything (docs/pairing.md 「局域网发现」).
//!
//! Every device announces itself over multicast DNS as a `_voltip._tcp.local.` service on its
//! LAN host's port, with the public-key fingerprint, name and platform in its TXT record, and
//! browses for the others:
//!
//! - A **trusted** device seen at a new address is dialled there right away (the address is only
//!   a hint: the Noise handshake still proves the key, and only an authenticated peer's own
//!   `device_info_update` is ever stored).
//! - A desktop **waiting for a pairing** adds its ticket to the record, without the relay and LAN
//!   hints: a flag says whether the session waits on the relay (the phone then meets it on its
//!   own relay, as after a scan) or on the desktop's LAN host (the phone dials the address it
//!   saw). The record stays small whatever relay is configured, and the relay's address is not
//!   broadcast to the LAN. A phone lists it under 「附近的电脑」 and joins with one tap; the safety
//!   code is compared as for a scanned QR code.
//!
//! The core talks to a [`Discovery`] port: [`MdnsDiscovery`] (mdns-sd) in the shells, an
//! in-memory LAN ([`fake::LocalLan`]) in the tests. `Settings.lan_discovery` switches both the
//! announcement and the browsing off.

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::sync::mpsc;
use voltip_protocol::Platform;

/// The service type.
pub const SERVICE_TYPE: &str = "_voltip._tcp.local.";
/// TXT schema version (`v`).
pub const TXT_VERSION: &str = "1";
/// Longest TXT value mDNS carries in one string (RFC 6763 §6.1: 255 bytes including `key=`).
pub const MAX_TXT_VALUE_BYTES: usize = 250;

/// The key's tag on the LAN: `SHA-256(public key)`, first 8 bytes, 16 upper-case hex digits (the
/// device list's fingerprint without its separators, which host and instance names cannot carry).
pub fn key_tag(key: &voltip_crypto::PublicKey) -> String {
    use sha2::Digest as _;
    sha2::Sha256::digest(key.as_bytes())[..8].iter().map(|b| format!("{b:02X}")).collect()
}

/// What this device announces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Announcement {
    /// [`key_tag`]: how peers recognise a trusted device.
    pub fingerprint: String,
    /// The device's name.
    pub name: String,
    /// Its platform.
    pub platform: Platform,
    /// The LAN host's port.
    pub port: u16,
    /// A pairing ticket URI (no relay or LAN hints) while this device waits for a peer to pair.
    pub ticket: Option<String>,
    /// That pairing session waits on the relay rather than on the LAN host.
    pub on_relay: bool,
}

/// A device another browse found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sighting {
    /// Its [`key_tag`].
    pub fingerprint: String,
    /// Its name.
    pub name: String,
    /// Its platform.
    pub platform: Platform,
    /// Where its LAN host answers (IPv4 first).
    pub addrs: Vec<SocketAddr>,
    /// Its pairing ticket, while it waits for a peer.
    pub ticket: Option<String>,
    /// That pairing session waits on the relay.
    pub on_relay: bool,
}

/// What a browse reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiscoveryEvent {
    /// A device was seen, or its record changed.
    Seen(Sighting),
    /// A device stopped announcing.
    Gone {
        /// Its fingerprint.
        fingerprint: String,
    },
}

/// A device the LAN browse sees, as the UI lists it (`UiState.nearby`).
#[derive(Clone, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub struct NearbyDevice {
    /// Its [`key_tag`]; `pairing_join_nearby` names it.
    pub fingerprint: String,
    /// Its name.
    pub name: String,
    /// Its platform.
    pub platform: Platform,
    /// It waits for a peer to pair: a tap joins it.
    pub pairing: bool,
    /// It is one of this device's trusted devices.
    pub trusted: bool,
}

/// The LAN discovery port.
pub trait Discovery: Send + Sync {
    /// Announce `announcement`, replacing what was announced before.
    fn announce(&self, announcement: &Announcement) -> Result<(), String>;
    /// Stop announcing.
    fn withdraw(&self);
    /// Browse; every sighting goes to `sink` until [`Discovery::stop_browsing`].
    fn browse(&self, sink: mpsc::Sender<DiscoveryEvent>) -> Result<(), String>;
    /// Stop browsing.
    fn stop_browsing(&self);
}

impl std::fmt::Debug for dyn Discovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Discovery")
    }
}

/// The TXT properties of `a` (`v`, `fp`, `n`, `pl`, and `t` while it has a ticket that fits,
/// with `r=1` when that session waits on the relay).
pub fn txt(a: &Announcement) -> Vec<(String, String)> {
    let mut props = vec![
        ("v".to_owned(), TXT_VERSION.to_owned()),
        ("fp".to_owned(), a.fingerprint.clone()),
        ("n".to_owned(), truncate_bytes(&a.name, MAX_TXT_VALUE_BYTES).to_owned()),
        ("pl".to_owned(), platform_name(a.platform).to_owned()),
    ];
    if let Some(ticket) = a.ticket.as_ref().filter(|t| t.len() <= MAX_TXT_VALUE_BYTES) {
        props.push(("t".to_owned(), ticket.clone()));
        if a.on_relay {
            props.push(("r".to_owned(), "1".to_owned()));
        }
    }
    props
}

/// A sighting from a record's TXT properties and addresses; `None` for another schema or a
/// record without a fingerprint.
pub fn sighting(get: impl Fn(&str) -> Option<String>, addrs: Vec<SocketAddr>) -> Option<Sighting> {
    if get("v").as_deref() != Some(TXT_VERSION) {
        return None;
    }
    let fingerprint = get("fp").filter(|f| !f.is_empty())?;
    let name = get("n").filter(|n| !n.is_empty()).unwrap_or_else(|| fingerprint.clone());
    let platform = get("pl").and_then(|p| platform_from(&p)).unwrap_or(Platform::Other);
    let ticket = get("t").filter(|t| !t.is_empty());
    let on_relay = ticket.is_some() && get("r").as_deref() == Some("1");
    let mut addrs = addrs;
    addrs.sort_by_key(|a| (!a.is_ipv4(), *a));
    addrs.dedup();
    Some(Sighting { fingerprint, name, platform, addrs, ticket, on_relay })
}

fn truncate_bytes(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn platform_name(p: Platform) -> &'static str {
    match p {
        Platform::Windows => "windows",
        Platform::Macos => "macos",
        Platform::Linux => "linux",
        Platform::Android => "android",
        Platform::Ios => "ios",
        Platform::Other => "other",
    }
}

fn platform_from(name: &str) -> Option<Platform> {
    Some(match name {
        "windows" => Platform::Windows,
        "macos" => Platform::Macos,
        "linux" => Platform::Linux,
        "android" => Platform::Android,
        "ios" => Platform::Ios,
        "other" => Platform::Other,
        _ => return None,
    })
}

/// Real multicast DNS (mdns-sd). The daemon runs on a thread of its own; the browse is forwarded
/// onto the runtime's channel from a blocking thread.
pub struct MdnsDiscovery {
    daemon: mdns_sd::ServiceDaemon,
    registered: parking_lot::Mutex<Option<String>>,
}

impl MdnsDiscovery {
    /// Start the daemon (binds UDP 5353 on every interface, loopback included).
    pub fn new() -> Result<Arc<Self>, String> {
        let daemon = mdns_sd::ServiceDaemon::new().map_err(|e| e.to_string())?;
        Ok(Arc::new(Self { daemon, registered: parking_lot::Mutex::new(None) }))
    }
}

impl Discovery for MdnsDiscovery {
    fn announce(&self, a: &Announcement) -> Result<(), String> {
        let instance = format!("Voltip-{}", a.fingerprint);
        let host = format!("voltip-{}.local.", a.fingerprint.to_ascii_lowercase());
        let props = txt(a);
        let props: Vec<(&str, &str)> = props.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let info = mdns_sd::ServiceInfo::new(SERVICE_TYPE, &instance, &host, "", a.port, &props[..]).map_err(|e| e.to_string())?.enable_addr_auto();
        let fullname = info.get_fullname().to_owned();
        self.daemon.register(info).map_err(|e| e.to_string())?;
        *self.registered.lock() = Some(fullname);
        Ok(())
    }

    fn withdraw(&self) {
        if let Some(fullname) = self.registered.lock().take()
            && let Err(e) = self.daemon.unregister(&fullname)
        {
            tracing::debug!(error = %e, "mdns: unregister failed");
        }
    }

    fn browse(&self, sink: mpsc::Sender<DiscoveryEvent>) -> Result<(), String> {
        let events = self.daemon.browse(SERVICE_TYPE).map_err(|e| e.to_string())?;
        std::thread::Builder::new()
            .name("voltip-mdns-browse".into())
            .spawn(move || {
                // Service full name → fingerprint, for `ServiceRemoved` (which names only the record).
                let mut names = std::collections::HashMap::<String, String>::new();
                while let Ok(event) = events.recv() {
                    let out = match event {
                        mdns_sd::ServiceEvent::ServiceResolved(info) => {
                            let addrs = info.get_addresses().iter().map(|ip| SocketAddr::new(ip.to_ip_addr(), info.get_port())).collect();
                            let found = sighting(|k| info.get_property_val_str(k).map(str::to_owned), addrs);
                            if let Some(s) = &found {
                                names.insert(info.get_fullname().to_owned(), s.fingerprint.clone());
                            }
                            found.map(DiscoveryEvent::Seen)
                        }
                        mdns_sd::ServiceEvent::ServiceRemoved(_, fullname) => names.remove(&fullname).map(|fingerprint| DiscoveryEvent::Gone { fingerprint }),
                        mdns_sd::ServiceEvent::SearchStopped(_) => break,
                        _ => None,
                    };
                    if let Some(out) = out
                        && sink.blocking_send(out).is_err()
                    {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn stop_browsing(&self) {
        let _ = self.daemon.stop_browse(SERVICE_TYPE);
    }
}

impl Drop for MdnsDiscovery {
    fn drop(&mut self) {
        self.withdraw();
        let _ = self.daemon.shutdown();
    }
}

/// An in-memory LAN for tests: every [`fake::LocalLan::join`]ed node sees the others' announcements.
pub mod fake {
    use super::*;

    #[derive(Default)]
    struct Bus {
        /// Node id → what it announces.
        records: std::collections::HashMap<u64, Announcement>,
        /// Node id → its browse sink.
        browsers: std::collections::HashMap<u64, mpsc::Sender<DiscoveryEvent>>,
        next: u64,
    }

    /// The shared medium.
    #[derive(Clone, Default)]
    pub struct LocalLan(Arc<parking_lot::Mutex<Bus>>);

    /// One node's port on it; its host answers on `127.0.0.1`.
    pub struct LanNode {
        lan: LocalLan,
        id: u64,
    }

    impl LocalLan {
        /// A new node on this LAN.
        pub fn join(&self) -> Arc<LanNode> {
            let mut bus = self.0.lock();
            bus.next += 1;
            Arc::new(LanNode { lan: self.clone(), id: bus.next })
        }

        /// What the nodes announce now.
        pub fn announced(&self) -> Vec<Announcement> {
            self.0.lock().records.values().cloned().collect()
        }
    }

    fn seen(a: &Announcement) -> DiscoveryEvent {
        let ticket = a.ticket.clone().filter(|t| t.len() <= MAX_TXT_VALUE_BYTES);
        DiscoveryEvent::Seen(Sighting {
            fingerprint: a.fingerprint.clone(),
            name: a.name.clone(),
            platform: a.platform,
            addrs: vec![SocketAddr::from(([127, 0, 0, 1], a.port))],
            on_relay: ticket.is_some() && a.on_relay,
            ticket,
        })
    }

    impl Discovery for LanNode {
        fn announce(&self, a: &Announcement) -> Result<(), String> {
            let mut bus = self.lan.0.lock();
            bus.records.insert(self.id, a.clone());
            for (id, sink) in &bus.browsers {
                if *id != self.id {
                    let _ = sink.try_send(seen(a));
                }
            }
            Ok(())
        }

        fn withdraw(&self) {
            let mut bus = self.lan.0.lock();
            if let Some(a) = bus.records.remove(&self.id) {
                for (id, sink) in &bus.browsers {
                    if *id != self.id {
                        let _ = sink.try_send(DiscoveryEvent::Gone { fingerprint: a.fingerprint.clone() });
                    }
                }
            }
        }

        fn browse(&self, sink: mpsc::Sender<DiscoveryEvent>) -> Result<(), String> {
            let mut bus = self.lan.0.lock();
            for (id, a) in &bus.records {
                if *id != self.id {
                    let _ = sink.try_send(seen(a));
                }
            }
            bus.browsers.insert(self.id, sink);
            Ok(())
        }

        fn stop_browsing(&self) {
            self.lan.0.lock().browsers.remove(&self.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn announcement(ticket: Option<String>) -> Announcement {
        Announcement { fingerprint: "AB12CD34EF56AB78".into(), name: "Studio".into(), platform: Platform::Macos, port: 47831, ticket, on_relay: false }
    }

    #[test]
    fn the_key_tag_is_the_fingerprint_without_its_separators() {
        let key = voltip_crypto::PublicKey([7; 32]);
        let tag = key_tag(&key);
        assert_eq!(tag.len(), 16);
        assert_eq!(tag, key.fingerprint().replace([':', ' ', '·'], ""));
    }

    #[test]
    fn a_record_round_trips_through_its_txt_properties() {
        let a = announcement(Some("voltip://pair?t=abc".into()));
        let props = txt(&a);
        let get = |k: &str| props.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
        let addrs = vec!["[fe80::1]:47831".parse().unwrap(), "192.168.1.24:47831".parse().unwrap(), "192.168.1.24:47831".parse().unwrap()];
        let s = sighting(get, addrs).unwrap();
        assert_eq!(s.fingerprint, a.fingerprint);
        assert_eq!((s.name.as_str(), s.platform), ("Studio", Platform::Macos));
        assert_eq!(s.ticket.as_deref(), Some("voltip://pair?t=abc"));
        assert!(!s.on_relay);
        let relayed = Announcement { on_relay: true, ..a.clone() };
        let props = txt(&relayed);
        let get = |k: &str| props.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
        assert!(sighting(get, Vec::new()).unwrap().on_relay, "`r=1`: the session waits on the relay");
        assert_eq!(s.addrs, vec!["192.168.1.24:47831".parse::<SocketAddr>().unwrap(), "[fe80::1]:47831".parse().unwrap()], "IPv4 first, no duplicates");
        // A ticket too long for one TXT string is left out (the QR code and the code still work).
        let long = announcement(Some("x".repeat(MAX_TXT_VALUE_BYTES + 1)));
        assert!(!txt(&long).iter().any(|(k, _)| k == "t"));
        // Another schema, or no fingerprint, is not a Voltip device we understand.
        assert!(sighting(|k| (k == "v").then(|| "2".to_owned()), Vec::new()).is_none());
        assert!(sighting(|k| (k == "v").then(|| "1".to_owned()), Vec::new()).is_none());
        // A long name is cut on a character boundary.
        let named = Announcement { name: "设".repeat(200), ..announcement(None) };
        let n = txt(&named).into_iter().find(|(k, _)| k == "n").unwrap().1;
        assert!(n.len() <= MAX_TXT_VALUE_BYTES && n.chars().all(|c| c == '设'));
    }

    /// Real multicast DNS between two daemons on this host (loopback included by default).
    /// `#[ignore]`d: it needs UDP 5353 and multicast, which CI containers may not route. Run with
    /// `cargo test -p voltip-core --lib discovery -- --ignored`.
    #[tokio::test]
    #[ignore = "needs multicast on this host"]
    async fn two_mdns_daemons_on_one_host_see_each_other() {
        let (a, b) = (MdnsDiscovery::new().unwrap(), MdnsDiscovery::new().unwrap());
        let ours = Announcement {
            fingerprint: "0123456789ABCDEF".into(),
            name: "测试机".into(),
            platform: Platform::Linux,
            port: 47999,
            ticket: Some("voltip://pair?t=x".into()),
            on_relay: true,
        };
        a.announce(&ours).unwrap();
        let (tx, mut rx) = mpsc::channel(32);
        b.browse(tx).unwrap();
        async fn next(rx: &mut mpsc::Receiver<DiscoveryEvent>, pick: impl Fn(&Sighting) -> bool) -> Sighting {
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                loop {
                    match rx.recv().await {
                        Some(DiscoveryEvent::Seen(s)) if pick(&s) => return s,
                        Some(_) => {}
                        None => panic!("browse ended"),
                    }
                }
            })
            .await
            .expect("the other daemon's record within 10 s")
        }
        let seen = next(&mut rx, |s| s.fingerprint == "0123456789ABCDEF").await;
        assert_eq!((seen.name.as_str(), seen.platform, seen.ticket.as_deref(), seen.on_relay), ("测试机", Platform::Linux, Some("voltip://pair?t=x"), true));
        assert!(seen.addrs.iter().all(|addr| addr.port() == 47999) && !seen.addrs.is_empty(), "{:?}", seen.addrs);
        // A renewed pairing (always-on, docs/pairing.md 「常开配对」) reaches the browser as a new sighting.
        a.announce(&Announcement { ticket: Some("voltip://pair?t=y".into()), ..ours.clone() }).unwrap();
        let renewed = next(&mut rx, |s| s.ticket.as_deref() == Some("voltip://pair?t=y")).await;
        assert_eq!(renewed.fingerprint, ours.fingerprint);
        a.withdraw();
        b.stop_browsing();
    }

    #[tokio::test]
    async fn the_in_memory_lan_shows_each_node_the_others_and_their_withdrawal() {
        let lan = fake::LocalLan::default();
        let (a, b) = (lan.join(), lan.join());
        a.announce(&announcement(None)).unwrap();
        let (tx, mut rx) = mpsc::channel(8);
        b.browse(tx).unwrap();
        assert!(matches!(rx.recv().await, Some(DiscoveryEvent::Seen(s)) if s.name == "Studio" && s.addrs[0].port() == 47831));
        a.announce(&announcement(Some("voltip://pair?t=x".into()))).unwrap();
        assert!(matches!(rx.recv().await, Some(DiscoveryEvent::Seen(s)) if s.ticket.is_some()));
        a.withdraw();
        assert!(matches!(rx.recv().await, Some(DiscoveryEvent::Gone { fingerprint }) if fingerprint == "AB12CD34EF56AB78"));
        let (tx, mut own) = mpsc::channel(8);
        a.browse(tx).unwrap();
        b.announce(&Announcement { fingerprint: "FFFF".into(), ..announcement(None) }).unwrap();
        assert!(matches!(own.recv().await, Some(DiscoveryEvent::Seen(s)) if s.fingerprint == "FFFF"));
        b.stop_browsing();
    }
}
