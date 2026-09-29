//! What the database keeps beside each entry for the queries and the statistics
//! (docs/dictation.md §4.5): whether the entry counts, how long it was spoken, its transcribed and
//! corrected characters, its latency, and the text a search looks in. All of it is computed from
//! the entry when it is written; the entry's JSON stays the record.

use super::{HistoryEntry, OriginKind, Outcome};
use crate::dictation::TakeKind;

/// Longest run of characters compared as one piece when counting corrections. Longer texts are cut
/// into pieces of about this size at the same proportions on both sides, which keeps the count
/// linear in the length (an approximation for texts that long).
pub const CORRECTION_PIECE_CHARS: usize = 2_000;

/// Separator between the fields of [`search_text`]: a control character no query contains, so a
/// match never spans two fields.
const FIELD_SEPARATOR: char = '\u{1f}';

/// Whether an entry counts toward the statistics: dictations only. A voice edit does not, nor a
/// text or the clipboard a phone sent; a phone's take (its audio recognised here) does.
pub fn counts_for_stats(entry: &HistoryEntry) -> bool {
    entry.kind == TakeKind::Dictation && !matches!(entry.origin.as_ref().map(|o| o.kind), Some(OriginKind::Typed | OriginKind::Clipboard))
}

/// The characters the transcript has (Unicode scalar values).
pub(super) fn raw_chars(entry: &HistoryEntry) -> u64 {
    u64::try_from(entry.raw_text.chars().count()).unwrap_or(u64::MAX)
}

/// The characters between the transcript and the text delivered: their Levenshtein distance in
/// Unicode scalar values, white space left out of both. Texts longer than
/// [`CORRECTION_PIECE_CHARS`] are compared piece by piece.
pub fn corrected_chars(raw: &str, text: &str) -> u64 {
    let a: Vec<char> = raw.chars().filter(|c| !c.is_whitespace()).collect();
    let b: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    let pieces = a.len().max(b.len()).div_ceil(CORRECTION_PIECE_CHARS).max(1);
    (0..pieces)
        .map(|i| {
            let piece = |chars: &[char]| chars[chars.len() * i / pieces..chars.len() * (i + 1) / pieces].to_vec();
            u64::try_from(strsim::generic_levenshtein(&piece(&a), &piece(&b))).unwrap_or(u64::MAX)
        })
        .sum()
}

/// Recognition plus clean-up, as the home page's latency adds them up.
pub(super) fn latency_ms(entry: &HistoryEntry) -> u64 {
    entry.asr_ms.saturating_add(entry.refine_ms.unwrap_or(0))
}

/// The entry's kind as its `kind` column stores it.
pub(super) const fn kind_name(kind: TakeKind) -> &'static str {
    match kind {
        TakeKind::Dictation => "dictation",
        TakeKind::Edit => "edit",
    }
}

/// How the entry ended, as its `outcome` column stores it (the wire's `outcome.kind`).
pub(super) const fn outcome_name(outcome: &Outcome) -> &'static str {
    match outcome {
        Outcome::Inserted { .. } => "inserted",
        Outcome::Clipboard { .. } => "clipboard",
        Outcome::Failed { .. } => "failed",
    }
}

/// The text `history_query` searches: the fields the history page searched when it filtered in the
/// webview — a built-in scene's name in both languages, the text, the transcript, both models, the
/// application's name and id, the scene's name, the voice edit's instruction and selection — each
/// lowercased on its own and joined by [`FIELD_SEPARATOR`].
pub(super) fn search_text(entry: &HistoryEntry) -> String {
    let builtin = entry.scene.as_ref().and_then(|s| s.builtin);
    let fields = [
        builtin.map(|b| b.display_name()),
        builtin.map(|b| b.english_name()),
        Some(entry.text.as_str()),
        Some(entry.raw_text.as_str()),
        Some(entry.asr_model.as_str()),
        entry.refine_model.as_deref(),
        entry.app.as_ref().map(|a| a.name.as_str()),
        entry.app.as_ref().map(|a| a.id.as_str()),
        entry.scene.as_ref().map(|s| s.name.as_str()),
        entry.edit.as_ref().map(|e| e.instruction.as_str()),
        entry.edit.as_ref().map(|e| e.selection.as_str()),
    ];
    let mut out = String::new();
    for field in fields.into_iter().flatten().filter(|f| !f.is_empty()) {
        if !out.is_empty() {
            out.push(FIELD_SEPARATOR);
        }
        out.push_str(&field.to_lowercase());
    }
    out
}

/// A query as [`search_text`] is matched against: trimmed and lowercased, `None` when empty.
pub(super) fn search_needle(query: &str) -> Option<String> {
    let q = query.trim();
    (!q.is_empty()).then(|| q.to_lowercase())
}
