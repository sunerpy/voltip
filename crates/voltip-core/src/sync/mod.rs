//! A computer's history and settings on its phones, and the phones' own records on the computer
//! (docs/dictation.md §20.8). The history's numbered changes live with the store
//! (`history::ChangeBatch`); this module holds the limits both sides agree on.

use serde::Serialize;

pub mod bulk;
pub mod mirror;
pub mod wire;

pub use bulk::{BULK_ACK_EVERY, BULK_WINDOW, BulkKind, BulkPath, LINK_RESERVE};
pub use mirror::{MirrorFiles, MirrorState, MirrorStore, read_copied_profile, read_entry, valid_computer};
pub use wire::{BulkBody, Profile};

/// Largest body put back together from [`voltip_protocol::app::AppMessage::Bulk`] parts.
pub const MAX_BULK_BYTES: usize = 16 * 1024 * 1024;
/// What a batch of changes or of uploaded records aims at: the encoded entries add up to this,
/// except that a batch always takes its first entry.
pub const BATCH_BUDGET_BYTES: usize = 1024 * 1024;
/// Most entries in one batch of changes.
pub const MAX_BATCH_UPSERTS: usize = 2_000;
/// Most deletions in one batch of changes.
pub const MAX_BATCH_DELETES: usize = 20_000;
/// Most records in one upload from a phone (one `PhoneRecordsAck`).
pub const MAX_UPLOAD_RECORDS: usize = voltip_protocol::app::MAX_PHONE_RECORDS_ACK;
/// An entry whose encoding is larger than this goes out as its bounded projection (history) or
/// stays on the phone (uploads), so a batch always fits in [`MAX_BULK_BYTES`].
pub const MAX_ENTRY_BYTES: usize = MAX_BULK_BYTES - 64 * 1024;

/// The CBOR size of `value`, as it travels.
pub fn cbor_len<T: Serialize>(value: &T) -> usize {
    struct Count(usize);
    impl std::io::Write for Count {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0 += buf.len();
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count(0);
    match ciborium::into_writer(value, &mut count) {
        Ok(()) => count.0,
        Err(_) => usize::MAX,
    }
}

/// Which side of the sync a core plays (`CoreConfig::sync_role`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SyncRole {
    /// Neither (tests, tools).
    #[default]
    Off,
    /// The desktop: offers its history and settings, takes the phones' records.
    Computer,
    /// The phone: keeps copies of its computers', uploads its own records.
    Phone,
}

/// Where a phone's copy of one computer stands (`UiState.mirrors`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MirrorSyncState {
    /// Changes are on their way.
    Syncing,
    /// The copy has every change the computer reported.
    UpToDate,
    /// The computer is not connected; the copy is as of `synced_at_ms`.
    Offline,
    /// The computer stopped syncing with this phone; there is no copy.
    Revoked,
    /// The Voltip on the computer does not sync yet.
    NeedsUpgrade,
    /// The phone already syncs with [`voltip_identity::MAX_SYNC_PEERS`] computers paired earlier.
    Limit,
}

/// One computer as the phone's 记录 and 设置 › 电脑 show it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct MirrorView {
    /// The computer's key in hex: what the copy queries name.
    pub desktop: String,
    /// The computer's name.
    pub name: String,
    /// Where the copy stands.
    pub state: MirrorSyncState,
    /// Entries in the copy.
    pub entries: u32,
    /// When a batch or the settings last arrived.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synced_at_ms: Option<u64>,
    /// The computer's settings without the four lists (`mirror_profile` has them).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<Profile>,
}

/// What a core's bulk traffic reached (`CoreConfig::sync_stats`): the end-to-end tests read it to
/// check the windows on a real relay.
#[derive(Debug, Default)]
pub struct SyncStats {
    /// The most parts one of this core's paths had out unconfirmed at once.
    pub max_in_flight: std::sync::atomic::AtomicU64,
}
