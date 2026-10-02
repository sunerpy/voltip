//! The computer's history and settings on its phones, and the phones' own records on the
//! computer (docs/dictation.md §20.8).
//!
//! - Computer → phone: the phone pulls. It asks for the changes after the one it applied
//!   (`MirrorRequest`); the computer answers with its settings, when their tag differs, and one
//!   batch of changes, both as bulk bodies on one path. A `MirrorChanged` tells a phone that asked
//!   during this connection that something changed; at most one goes unanswered per phone.
//! - Phone → computer: the phone's records no computer has confirmed are its outbox. One batch at a
//!   time goes to the computer that came online first; its `PhoneRecordsAck` marks them uploaded.
//! - The computer's switch per phone and its generation decide what a phone accepts; a
//!   `MirrorRevoke` deletes the phone's copy.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use uuid::Uuid;
use voltip_crypto::PublicKey;
use voltip_identity::{MAX_SYNC_PEERS, SyncChange, TrustedDevice};
use voltip_protocol::ProtocolVersion;
use voltip_protocol::app::AppMessage;
use voltip_protocol::relay::RelayFrame;

use super::Runtime;
use crate::history::{EntryOrigin, HistoryEntry, OriginKind};
use crate::peer::{LinkId, PeerPhase};
use crate::sync::{BATCH_BUDGET_BYTES, BulkBody, BulkKind, MAX_UPLOAD_RECORDS, MirrorStore, MirrorSyncState, MirrorView, Profile, SyncRole};
use crate::{CoreError, CoreEvent, now_ms};

/// The computer's view of one phone that asked during this connection.
#[derive(Default)]
struct PhoneSync {
    /// The request to answer.
    pending: Option<Pending>,
    /// A `MirrorChanged` went out since the phone's last request.
    told: bool,
}

struct Pending {
    req: u32,
    epoch: Option<Uuid>,
    since: u64,
    profile: Option<[u8; 32]>,
    at: Instant,
}

/// The phone's view of one computer that is online.
struct ComputerSync {
    /// The device info since it came online: whether it said it syncs.
    info: Option<bool>,
    /// When it came online.
    online_since: Instant,
    /// The request out, and when it or a part of its answer was last seen.
    inflight: Option<(u32, Instant)>,
    /// Wait before asking again; doubles while requests go unanswered.
    wait: Duration,
    /// A change was announced while a request was out.
    dirty: bool,
}

/// What the phone keeps about a computer while it runs (reset when a pairing completes).
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
struct Switch {
    seen_gen: u32,
    revoked: bool,
    next_req: u32,
}

impl Switch {
    /// The switch generation rule (docs/dictation.md §20.8): a message of an older generation is
    /// dropped; a revoke marks the computer revoked; the other messages lift it only with a newer
    /// generation (in one generation the computer's switch does not change).
    fn accept(&mut self, generation: u32, revoke: bool) -> bool {
        if generation < self.seen_gen {
            return false;
        }
        if revoke {
            self.seen_gen = generation;
            self.revoked = true;
            return true;
        }
        if self.revoked && generation == self.seen_gen {
            return false;
        }
        self.seen_gen = generation;
        self.revoked = false;
        true
    }
}

/// The phone's batch on its way to a computer.
struct Upload {
    computer: PublicKey,
    body: Vec<u8>,
    ids: Vec<Uuid>,
    tag: u64,
    /// When its last part went out.
    sent_at: Option<Instant>,
    resent: bool,
}

/// The runtime's sync state.
#[derive(Default)]
pub(super) struct SyncState {
    next_tag: u64,
    /// The peers that had a secure path at the last look.
    online: HashSet<PublicKey>,
    // ---- computer ----
    phones: HashMap<PublicKey, PhoneSync>,
    /// The head and settings tag the phones were last told about.
    told: Option<(u64, [u8; 32])>,
    /// The settings now, rebuilt when they change.
    profile: Option<(Profile, [u8; 32])>,
    // ---- phone ----
    computers: HashMap<PublicKey, ComputerSync>,
    switches: HashMap<PublicKey, Switch>,
    stores: HashMap<PublicKey, MirrorStore>,
    upload: Option<Upload>,
    /// The outbox was empty at the last look and nothing was written since.
    outbox_idle: bool,
    too_large: Vec<Uuid>,
}

impl Runtime {
    fn sync_role(&self) -> SyncRole {
        self.config.sync_role
    }

    /// A paired computer (the phone syncs with computers only).
    fn is_computer(&self, key: &PublicKey) -> bool {
        self.trusted.get_by_key(key).is_some_and(|d| !d.is_phone())
    }

    /// The computers this phone syncs with: the [`MAX_SYNC_PEERS`] paired first.
    fn sync_computers(&self) -> Vec<(TrustedDevice, bool)> {
        let mut computers: Vec<TrustedDevice> = self.trusted.list().into_iter().filter(|d| !d.is_phone()).collect();
        computers.sort_by(|a, b| a.trusted_at.cmp(&b.trusted_at).then(a.public_key.0.cmp(&b.public_key.0)));
        computers.into_iter().enumerate().map(|(i, d)| (d, i < MAX_SYNC_PEERS)).collect()
    }

    fn eligible(&self, key: &PublicKey) -> bool {
        self.sync_computers().iter().any(|(d, ok)| *ok && &d.public_key == key)
    }

    // ---------------- both ----------------

    /// After every event: who came and went, then answer, upload and send what the windows allow.
    pub(super) async fn sync_step(&mut self) {
        if self.sync_role() == SyncRole::Off {
            return;
        }
        self.sync_presence();
        match self.sync_role() {
            SyncRole::Computer => self.answer_requests(),
            SyncRole::Phone => self.upload_next(),
            SyncRole::Off => {}
        }
        self.pump_bulk().await;
    }

    /// Once per tick.
    pub(super) async fn sync_tick(&mut self) {
        match self.sync_role() {
            SyncRole::Computer => self.tell_changes().await,
            SyncRole::Phone => {
                self.retry_requests().await;
                self.check_upload();
            }
            SyncRole::Off => {}
        }
    }

    fn sync_presence(&mut self) {
        let online: HashSet<PublicKey> = self.peers.iter_mut().filter_map(|(k, st)| st.best_secure_path().map(|_| *k)).collect();
        if online == self.sync.online {
            return;
        }
        let went: Vec<PublicKey> = self.sync.online.difference(&online).copied().collect();
        let came: Vec<PublicKey> = online.difference(&self.sync.online).copied().collect();
        self.sync.online = online;
        for key in went {
            self.sync.phones.remove(&key);
            if self.sync.computers.remove(&key).is_some() {
                // Every connection decides anew whether the computer syncs.
                if let Some(sw) = self.sync.switches.get_mut(&key) {
                    sw.revoked = false;
                }
            }
            if self.sync.upload.as_ref().is_some_and(|u| u.computer == key) {
                self.sync.upload = None;
                self.sync.outbox_idle = false;
            }
        }
        if self.sync_role() == SyncRole::Phone {
            let wait = self.config.sync_request_timeout;
            for key in came {
                if self.is_computer(&key) {
                    self.sync.computers.entry(key).or_insert_with(|| ComputerSync {
                        info: None,
                        online_since: Instant::now(),
                        inflight: None,
                        wait,
                        dirty: false,
                    });
                }
            }
            self.emit_mirrors();
        }
    }

    /// Queue an encoded body for `to` on its best secure path; its tag, or `None` when it is
    /// offline.
    fn push_bulk(&mut self, to: PublicKey, kind: BulkKind, body: Vec<u8>) -> Option<u64> {
        let tag = self.sync.next_tag;
        self.sync.next_tag += 1;
        let path = self.peers.get_mut(&to)?.best_secure_path()?;
        let PeerPhase::Secure(session) = &mut path.phase else { return None };
        session.bulk.push(kind, tag, body);
        Some(tag)
    }

    fn send_bulk(&mut self, to: PublicKey, kind: BulkKind, body: &BulkBody) -> Option<u64> {
        match body.encode() {
            Ok(bytes) => self.push_bulk(to, kind, bytes),
            Err(e) => {
                tracing::error!(error = %e, "sync body not sent");
                None
            }
        }
    }

    /// Drop `key`'s queued and half-sent bodies of `kind` on every path.
    fn cancel_bulk(&mut self, key: &PublicKey, kind: BulkKind) {
        for p in self.peers.get_mut(key).map(|st| st.paths.iter_mut()).into_iter().flatten() {
            if let PeerPhase::Secure(session) = &mut p.phase {
                session.bulk.cancel(kind);
            }
        }
    }

    /// The body `tag` is still queued or being sent to `key`.
    fn bulk_pending(&self, key: &PublicKey, tag: u64) -> bool {
        self.peers.get(key).is_some_and(|st| st.paths.iter().any(|p| matches!(&p.phase, PeerPhase::Secure(s) if s.bulk.pending(tag))))
    }

    /// Seal and send the parts every path's window and link allow (docs/dictation.md §20.8).
    async fn pump_bulk(&mut self) {
        let links: HashSet<LinkId> = self
            .peers
            .values()
            .flat_map(|st| st.paths.iter())
            .filter(|p| matches!(&p.phase, PeerPhase::Secure(s) if s.bulk.has_work()))
            .map(|p| p.link)
            .collect();
        if links.is_empty() {
            return;
        }
        let mut free: HashMap<LinkId, usize> = links.into_iter().map(|id| (id, self.link(id).map_or(0, |l| l.free_slots()))).collect();
        let mut frames = Vec::new();
        let mut most = 0;
        for st in self.peers.values_mut() {
            for p in &mut st.paths {
                let (PeerPhase::Secure(session), Some(sid)) = (&mut p.phase, p.session_id) else { continue };
                let Some(room) = free.get_mut(&p.link) else { continue };
                while let Some(part) = session.bulk.next_part(*room) {
                    match session.channel.seal(&part) {
                        Ok(bytes) => {
                            frames.push((p.link, RelayFrame::forward(sid, bytes)));
                            *room = room.saturating_sub(1);
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "bulk part not sealed");
                            break;
                        }
                    }
                }
                most = most.max(session.bulk.in_flight());
            }
        }
        self.config.sync_stats.max_in_flight.fetch_max(most, Ordering::Relaxed);
        for (link, frame) in frames {
            if let Err(e) = self.send_on(link, frame).await {
                tracing::debug!(error = %e, "bulk part not sent");
            }
        }
    }

    /// A whole body arrived from `key`.
    pub(super) async fn on_bulk_body(&mut self, key: PublicKey, bytes: &[u8]) {
        let body = match BulkBody::decode(bytes) {
            Ok(body) => body,
            Err(e) => {
                tracing::warn!(peer = %key.fingerprint(), error = %e, "sync body unreadable; dropped");
                return;
            }
        };
        match (self.sync_role(), body) {
            (SyncRole::Computer, BulkBody::PhoneRecords { records }) => self.on_records(key, records).await,
            (SyncRole::Phone, BulkBody::MirrorProfile { req, generation, tag, profile }) => self.on_profile(key, req, generation, &tag, &profile),
            (SyncRole::Phone, BulkBody::MirrorBatch { req, generation, epoch, reset, to, upserts, deletes, shortened, more, .. }) => {
                self.on_batch(key, req, generation, Batch { epoch, reset, to, upserts, deletes, shortened, more }).await;
            }
            _ => tracing::debug!(peer = %key.fingerprint(), "sync body meant for the other side; dropped"),
        }
    }

    /// A part from `key` arrived: the request it answers is alive.
    pub(super) fn on_bulk_progress(&mut self, key: PublicKey) {
        if let Some((_, at)) = self.sync.computers.get_mut(&key).and_then(|c| c.inflight.as_mut()) {
            *at = Instant::now();
        }
    }

    /// A pairing with `key` completed: a phone starts over with that computer (its generation
    /// from the new record on, no copy from before).
    pub(super) async fn on_sync_paired(&mut self, key: PublicKey) {
        if self.sync_role() != SyncRole::Phone {
            return;
        }
        let next_req = self.sync.switches.get(&key).map_or(0, |sw| sw.next_req);
        self.sync.switches.insert(key, Switch { seen_gen: 0, revoked: false, next_req });
        self.delete_mirror(key).await;
        // A re-pair keeps the secure channel (no new device info comes): ask from 0 now.
        if self.sync.computers.get(&key).is_some_and(|c| c.info == Some(true)) && self.eligible(&key) {
            self.send_request(key).await;
            self.emit_mirrors();
        }
    }

    /// `key` was forgotten: its copy goes (phone), or it stops being a subscriber (computer).
    pub(super) async fn on_sync_forgotten(&mut self, key: PublicKey) {
        self.sync.phones.remove(&key);
        self.sync.computers.remove(&key);
        self.sync.switches.remove(&key);
        if self.sync.upload.as_ref().is_some_and(|u| u.computer == key) {
            self.sync.upload = None;
        }
        if self.sync_role() == SyncRole::Phone {
            self.delete_mirror(key).await;
        }
    }

    // ---------------- computer ----------------

    pub(super) async fn on_mirror_request(&mut self, key: PublicKey, req: u32, epoch: Option<Uuid>, since: u64, profile: Option<&[u8]>) {
        if self.sync_role() != SyncRole::Computer {
            return;
        }
        let Some(record) = self.trusted.get_by_key(&key) else { return };
        if !record.sync {
            self.send_revoke(&record).await;
            return;
        }
        self.cancel_bulk(&key, BulkKind::Reply);
        let phone = self.sync.phones.entry(key).or_default();
        phone.pending = Some(Pending { req, epoch, since, profile: profile.and_then(|t| <[u8; 32]>::try_from(t).ok()), at: Instant::now() });
        phone.told = false;
    }

    /// The settings now and their tag.
    fn current_profile(&mut self) -> (Profile, [u8; 32]) {
        if self.profile_dirty.swap(false, Ordering::Relaxed) || self.sync.profile.is_none() {
            let engines = self.resolved_engines().status();
            let profile = Profile::new(&self.settings, &engines, self.presets.presets(), self.dictionary.entries(), self.rules.rules(), self.scenes.scenes());
            let tag = profile.tag();
            self.sync.profile = Some((profile, tag));
        }
        match &self.sync.profile {
            Some((profile, tag)) => (profile.clone(), *tag),
            None => unreachable!("set above"),
        }
    }

    /// Answer the phones' requests: the settings when the phone's differ, then one batch.
    fn answer_requests(&mut self) {
        let delay = self.config.test_hooks.answer_delay;
        let due: Vec<(PublicKey, Pending)> =
            self.sync.phones.iter_mut().filter_map(|(k, p)| p.pending.take_if(|r| r.at.elapsed() >= delay).map(|r| (*k, r))).collect();
        if due.is_empty() {
            return;
        }
        let (profile, tag) = self.current_profile();
        for (key, req) in due {
            let Some(record) = self.trusted.get_by_key(&key) else { continue };
            let generation = record.sync_gen;
            if req.profile != Some(tag) {
                let mut body =
                    BulkBody::MirrorProfile { req: req.req, generation, tag: serde_bytes::ByteBuf::from(tag.to_vec()), profile: Box::new(profile.clone()) };
                if body.encode().is_err() {
                    tracing::error!("the settings do not fit a body; sent without the lists");
                    body = BulkBody::MirrorProfile {
                        req: req.req,
                        generation,
                        tag: serde_bytes::ByteBuf::from(tag.to_vec()),
                        profile: Box::new(profile.without_lists()),
                    };
                }
                self.send_bulk(key, BulkKind::Reply, &body);
            }
            match self.history.changes_since(req.epoch, req.since, BATCH_BUDGET_BYTES) {
                Ok(batch) => {
                    self.send_bulk(key, BulkKind::Reply, &BulkBody::batch(req.req, generation, batch));
                }
                Err(e) => tracing::warn!(error = %e, "history changes unavailable; the phone asks again"),
            }
        }
    }

    /// Tell the phones that asked during this connection that something changed, once per
    /// request (docs/dictation.md §20.8).
    async fn tell_changes(&mut self) {
        let history = self.history_dirty.swap(false, Ordering::Relaxed);
        if !history && !self.profile_dirty.load(Ordering::Relaxed) && self.sync.told.is_some() {
            return;
        }
        let head = self.history.head();
        let (_, tag) = self.current_profile();
        if self.sync.told == Some((head, tag)) {
            return;
        }
        self.sync.told = Some((head, tag));
        let phones: Vec<PublicKey> = self.sync.phones.iter().filter(|(_, p)| !p.told && p.pending.is_none()).map(|(k, _)| *k).collect();
        for key in phones {
            let Some(record) = self.trusted.get_by_key(&key).filter(|r| r.sync) else { continue };
            let msg = AppMessage::MirrorChanged { version: ProtocolVersion::CURRENT, head, generation: record.sync_gen };
            if self.send_app(key, &msg).await.is_ok()
                && let Some(p) = self.sync.phones.get_mut(&key)
            {
                p.told = true;
            }
        }
    }

    /// Records a phone uploaded: written once each, then confirmed, all of them.
    async fn on_records(&mut self, key: PublicKey, records: Vec<HistoryEntry>) {
        let Some(record) = self.trusted.get_by_key(&key) else { return };
        if !record.sync {
            self.send_revoke(&record).await;
            return;
        }
        let mut ids = Vec::with_capacity(records.len());
        let mut wrote = false;
        for mut entry in records {
            ids.push(entry.id);
            // A computer that keeps no history confirms without writing.
            if !self.settings.history.enabled {
                continue;
            }
            entry.origin = Some(EntryOrigin { device: record.name.clone(), kind: OriginKind::Standalone });
            entry.segments = None;
            match self.history.insert_received(entry, self.settings.history.keep as usize, now_ms()) {
                Ok(written) => wrote |= written,
                Err(e) => {
                    // Not confirmed: the phone sends the batch again.
                    tracing::warn!(error = %e, "a phone's records could not be written");
                    return;
                }
            }
        }
        if wrote {
            self.emit_history();
        }
        if let Err(e) = self.send_app(key, &AppMessage::PhoneRecordsAck { version: ProtocolVersion::CURRENT, ids }).await {
            tracing::debug!(error = %e, "records confirmation not sent; the phone sends them again");
        }
    }

    async fn send_revoke(&mut self, record: &TrustedDevice) {
        let key = record.public_key;
        // Its requests and its notice limit start over with the next switch message.
        self.sync.phones.remove(&key);
        self.cancel_bulk(&key, BulkKind::Reply);
        let msg = AppMessage::MirrorRevoke { version: ProtocolVersion::CURRENT, generation: record.sync_gen };
        if let Err(e) = self.send_app(key, &msg).await {
            tracing::debug!(error = %e, "revoke not sent (the phone is offline; its next request gets one)");
        }
    }

    /// Switch syncing with a phone on or off (docs/dictation.md §20.8).
    pub(super) async fn set_device_sync(&mut self, key: PublicKey, on: bool) -> Result<(), CoreError> {
        match self.trusted.set_sync(&key, on)? {
            SyncChange::Changed(record) => {
                if on {
                    let msg = AppMessage::MirrorChanged { version: ProtocolVersion::CURRENT, head: self.history.head(), generation: record.sync_gen };
                    if self.send_app(key, &msg).await.is_ok() {
                        self.sync.phones.entry(key).or_default().told = true;
                    }
                } else {
                    self.send_revoke(&record).await;
                }
                self.emit_devices();
                Ok(())
            }
            SyncChange::Unchanged => Ok(()),
            SyncChange::Limit => Err(CoreError::Invalid(format!("最多与 {MAX_SYNC_PEERS} 部手机同步"))),
            SyncChange::Unknown => Err(CoreError::Invalid("未知设备".into())),
        }
    }

    // ---------------- phone ----------------

    /// A device info from `key` (every new secure path sends one): a computer that syncs gets a
    /// request, and a batch still unconfirmed goes once more on the new path.
    pub(super) async fn on_sync_announce(&mut self, key: PublicKey, mirror: bool) {
        if self.sync_role() != SyncRole::Phone || !self.is_computer(&key) {
            return;
        }
        let wait = self.config.sync_request_timeout;
        let computer =
            self.sync.computers.entry(key).or_insert_with(|| ComputerSync { info: None, online_since: Instant::now(), inflight: None, wait, dirty: false });
        computer.info = Some(mirror);
        let revoked = self.sync.switches.get(&key).is_some_and(|sw| sw.revoked);
        if mirror && !revoked && self.eligible(&key) {
            self.send_request(key).await;
        }
        let resend = match &self.sync.upload {
            Some(u) if u.computer == key && !u.resent => Some(u.body.clone()),
            _ => None,
        };
        if let Some(body) = resend
            && let Some(tag) = self.push_bulk(key, BulkKind::Upload, body)
            && let Some(u) = self.sync.upload.as_mut()
        {
            u.tag = tag;
            u.sent_at = None;
            u.resent = true;
        }
        self.emit_mirrors();
    }

    async fn send_request(&mut self, key: PublicKey) {
        let state =
            self.open_mirror(key).and_then(|store| store.state().inspect_err(|e| tracing::warn!(error = %e, "copy unreadable")).ok()).unwrap_or_default();
        let switch = self.sync.switches.entry(key).or_default();
        let req = switch.next_req;
        switch.next_req = switch.next_req.wrapping_add(1);
        if let Some(c) = self.sync.computers.get_mut(&key) {
            c.inflight = Some((req, Instant::now()));
            c.dirty = false;
        }
        let msg = AppMessage::MirrorRequest {
            version: ProtocolVersion::CURRENT,
            req,
            epoch: state.epoch,
            since: state.applied,
            profile: state.profile_tag.map(|t| serde_bytes::ByteBuf::from(t.to_vec())),
        };
        if let Err(e) = self.send_app(key, &msg).await {
            tracing::debug!(error = %e, "sync request not sent; asked again later");
        }
    }

    /// The switch generation rule (docs/dictation.md §20.8): older messages are dropped, a revoke
    /// marks the computer revoked, and only a newer generation lifts it.
    fn accept(&mut self, key: &PublicKey, generation: u32, revoke: bool) -> bool {
        self.sync.switches.entry(*key).or_default().accept(generation, revoke)
    }

    fn is_inflight(&self, key: &PublicKey, req: u32) -> bool {
        self.sync.computers.get(key).and_then(|c| c.inflight).is_some_and(|(r, _)| r == req)
    }

    pub(super) async fn on_mirror_changed(&mut self, key: PublicKey, generation: u32) {
        if self.sync_role() != SyncRole::Phone || !self.accept(&key, generation, false) || !self.eligible(&key) {
            return;
        }
        let Some(c) = self.sync.computers.get_mut(&key) else { return };
        if c.inflight.is_some() {
            c.dirty = true;
        } else {
            self.send_request(key).await;
        }
        self.emit_mirrors();
    }

    pub(super) async fn on_mirror_revoke(&mut self, key: PublicKey, generation: u32) {
        if self.sync_role() != SyncRole::Phone || !self.accept(&key, generation, true) {
            return;
        }
        if let Some(c) = self.sync.computers.get_mut(&key) {
            c.inflight = None;
            c.dirty = false;
        }
        if self.sync.upload.as_ref().is_some_and(|u| u.computer == key) {
            self.cancel_bulk(&key, BulkKind::Upload);
            self.sync.upload = None;
            self.sync.outbox_idle = false;
        }
        tracing::info!(computer = %key.fingerprint(), "the computer stopped syncing; its copy is deleted");
        self.delete_mirror(key).await;
    }

    fn on_profile(&mut self, key: PublicKey, req: u32, generation: u32, tag: &[u8], profile: &Profile) {
        if !self.accept(&key, generation, false) || !self.is_inflight(&key, req) {
            return;
        }
        let Ok(tag) = <[u8; 32]>::try_from(tag) else { return };
        if let Some(store) = self.open_mirror(key)
            && let Err(e) = store.set_profile(tag, profile, now_ms())
        {
            tracing::warn!(error = %e, "copied settings not kept");
        }
        self.emit_mirrors();
    }

    async fn on_batch(&mut self, key: PublicKey, req: u32, generation: u32, batch: Batch) {
        if !self.accept(&key, generation, false) || !self.is_inflight(&key, req) {
            return;
        }
        if !self.config.sync_apply_delay.is_zero() {
            // Tests only: a phone that applies slowly.
            tokio::time::sleep(self.config.sync_apply_delay).await;
        }
        let applied = match self.open_mirror(key) {
            Some(store) => store.apply(batch.epoch, batch.reset, batch.to, &batch.upserts, &batch.deletes, &batch.shortened, now_ms()),
            None => Err(CoreError::History("copy unavailable".into())),
        };
        if let Err(e) = applied {
            // The request goes unanswered and is asked again.
            tracing::warn!(error = %e, "sync batch not applied");
            return;
        }
        let wait = self.config.sync_request_timeout;
        let Some(c) = self.sync.computers.get_mut(&key) else { return };
        c.wait = wait;
        if batch.more || c.dirty {
            self.send_request(key).await;
        } else {
            c.inflight = None;
        }
        self.emit_mirrors();
    }

    /// Ask again where a request has gone unanswered for too long (the wait doubles up to
    /// `sync_request_timeout_max`).
    async fn retry_requests(&mut self) {
        let now = Instant::now();
        let max = self.config.sync_request_timeout_max;
        let late: Vec<PublicKey> = self
            .sync
            .computers
            .iter_mut()
            .filter_map(|(k, c)| match c.inflight {
                Some((_, at)) if now.duration_since(at) >= c.wait => {
                    c.wait = (c.wait * 2).min(max);
                    Some(*k)
                }
                _ => None,
            })
            .collect();
        for key in late {
            tracing::info!(computer = %key.fingerprint(), "sync request unanswered; asking again");
            self.send_request(key).await;
        }
    }

    /// The computer to upload to: one that syncs, online, the one online the longest.
    fn upload_target(&self) -> Option<PublicKey> {
        let eligible: HashSet<PublicKey> = self.sync_computers().into_iter().filter(|(_, ok)| *ok).map(|(d, _)| d.public_key).collect();
        self.sync
            .computers
            .iter()
            .filter(|(k, c)| c.info == Some(true) && eligible.contains(k) && !self.sync.switches.get(k).is_some_and(|sw| sw.revoked))
            .min_by_key(|(_, c)| c.online_since)
            .map(|(k, _)| *k)
    }

    /// Send the next batch of the phone's own records, when none is on its way.
    fn upload_next(&mut self) {
        if self.history_dirty.swap(false, Ordering::Relaxed) {
            self.sync.outbox_idle = false;
        }
        if self.sync.upload.is_some() || self.sync.outbox_idle {
            return;
        }
        let Some(computer) = self.upload_target() else { return };
        let outbox = match self.history.outbox(BATCH_BUDGET_BYTES, MAX_UPLOAD_RECORDS, self.config.sync_max_entry_bytes) {
            Ok(outbox) => outbox,
            Err(e) => {
                tracing::warn!(error = %e, "outbox unreadable");
                return;
            }
        };
        if outbox.too_large != self.sync.too_large {
            self.sync.too_large = outbox.too_large.clone();
            self.emit(CoreEvent::PhoneOutbox { too_large: outbox.too_large });
        }
        if outbox.records.is_empty() {
            self.sync.outbox_idle = true;
            return;
        }
        let ids: Vec<Uuid> = outbox.records.iter().map(|e| e.id).collect();
        let body = match (BulkBody::PhoneRecords { records: outbox.records }).encode() {
            Ok(body) => body,
            Err(e) => {
                tracing::error!(error = %e, "upload batch not encoded");
                return;
            }
        };
        if let Some(tag) = self.push_bulk(computer, BulkKind::Upload, body.clone()) {
            tracing::info!(records = ids.len(), computer = %computer.fingerprint(), "uploading the phone's records");
            self.sync.upload = Some(Upload { computer, body, ids, tag, sent_at: None, resent: false });
        }
    }

    /// The batch's wait for a confirmation starts when its last part went out; past it, the batch
    /// is chosen again.
    fn check_upload(&mut self) {
        let Some(upload) = &self.sync.upload else { return };
        let pending = self.bulk_pending(&upload.computer, upload.tag);
        let timeout = self.config.sync_upload_timeout;
        let Some(upload) = self.sync.upload.as_mut() else { return };
        match upload.sent_at {
            None if !pending => upload.sent_at = Some(Instant::now()),
            Some(at) if at.elapsed() >= timeout => {
                tracing::info!(computer = %upload.computer.fingerprint(), "upload not confirmed; sent again");
                self.sync.upload = None;
                self.sync.outbox_idle = false;
            }
            _ => {}
        }
    }

    pub(super) fn on_records_ack(&mut self, key: PublicKey, ids: &[Uuid]) {
        if self.sync_role() != SyncRole::Phone {
            return;
        }
        if let Err(e) = self.history.mark_uploaded(ids, &key.to_hex(), now_ms()) {
            tracing::warn!(error = %e, "upload confirmation not recorded; the records go again");
        }
        if self.sync.upload.as_ref().is_some_and(|u| u.computer == key && u.ids.iter().all(|id| ids.contains(id))) {
            self.sync.upload = None;
        }
        self.sync.outbox_idle = false;
    }

    /// `key`'s copy, opened (and created) on first use.
    fn open_mirror(&mut self, key: PublicKey) -> Option<&mut MirrorStore> {
        if !self.sync.stores.contains_key(&key) {
            match MirrorStore::open(&self.config.mirror_files, &key.to_hex()) {
                Ok(store) => {
                    self.sync.stores.insert(key, store);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "copy of the computer's history unavailable");
                    return None;
                }
            }
        }
        self.sync.stores.get_mut(&key)
    }

    /// Delete `key`'s copy (waiting for the queries in progress), then tell the interface.
    async fn delete_mirror(&mut self, key: PublicKey) {
        let store = self.sync.stores.remove(&key);
        let files = self.config.mirror_files.clone();
        let computer = key.to_hex();
        match tokio::task::spawn_blocking(move || files.delete(&computer, store)).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::warn!(error = %e, "copy of a computer's history not deleted"),
            Err(e) => tracing::warn!(error = %e, "copy deletion did not run"),
        }
        self.emit_mirrors();
    }

    /// `UiState.mirrors`: every computer this phone is paired with.
    pub(super) fn emit_mirrors(&mut self) {
        if self.sync_role() != SyncRole::Phone {
            return;
        }
        let mut views = Vec::new();
        for (device, eligible) in self.sync_computers() {
            let key = device.public_key;
            let switch = self.sync.switches.get(&key).copied().unwrap_or_default();
            let state = match self.sync.computers.get(&key) {
                _ if !eligible => MirrorSyncState::Limit,
                _ if switch.revoked => MirrorSyncState::Revoked,
                None => MirrorSyncState::Offline,
                Some(c) if c.info == Some(false) => MirrorSyncState::NeedsUpgrade,
                Some(c) if c.info.is_none() || c.inflight.is_some() => MirrorSyncState::Syncing,
                Some(_) => MirrorSyncState::UpToDate,
            };
            let exists = self.config.mirror_files.path(&key.to_hex()).exists();
            let (state_now, profile) = match (eligible && !switch.revoked && exists).then(|| self.open_mirror(key)).flatten() {
                Some(store) => (store.state().unwrap_or_default(), store.profile().ok().flatten()),
                None => (Default::default(), None),
            };
            views.push(MirrorView {
                desktop: key.to_hex(),
                name: device.name,
                state,
                entries: state_now.entries,
                synced_at_ms: state_now.synced_at_ms,
                profile: profile.map(|p| p.without_lists()),
            });
        }
        self.emit(CoreEvent::Mirrors(views));
    }
}

/// A batch as [`Runtime::on_batch`] applies it.
struct Batch {
    epoch: Uuid,
    reset: bool,
    to: u64,
    upserts: Vec<HistoryEntry>,
    deletes: Vec<Uuid>,
    shortened: Vec<Uuid>,
    more: bool,
}

#[cfg(test)]
mod tests {
    use super::Switch;

    /// regression (plan gate, M7 design): the switch messages travel on whichever path is best at
    /// the time and may arrive out of order. Whatever the order, the phone ends in the state of the
    /// newest generation.
    #[test]
    fn regression_switch_messages_out_of_order_end_in_the_newest_state() {
        // Off (gen 1) then on again (gen 2); the revoke arrives last.
        let mut sw = Switch::default();
        assert!(sw.accept(0, false), "changes of the first generation");
        assert!(sw.accept(2, false), "on again");
        assert!(!sw.accept(1, true), "the late revoke is older");
        assert!(!sw.revoked);
        // In order: the revoke, then an old change, then the new one.
        let mut sw = Switch::default();
        assert!(sw.accept(1, true));
        assert!(sw.revoked);
        assert!(!sw.accept(1, false), "a change of the revoked generation is dropped");
        assert!(!sw.accept(0, false), "and an older one");
        assert!(sw.revoked);
        assert!(sw.accept(2, false), "a newer generation lifts it");
        assert!(!sw.revoked);
        assert_eq!(sw.seen_gen, 2);
        // A revoke repeated in the same generation (an answer to a request) stays revoked.
        let mut sw = Switch { seen_gen: 3, revoked: true, next_req: 0 };
        assert!(sw.accept(3, true));
        assert!(sw.revoked);
    }
}
