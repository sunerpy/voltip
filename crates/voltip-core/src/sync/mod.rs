//! A computer's history and settings on its phones, and the phones' own records on the computer
//! (docs/dictation.md §20.8). The history's numbered changes live with the store
//! (`history::ChangeBatch`); this module holds the limits both sides agree on.

use serde::Serialize;

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
