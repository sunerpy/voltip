//! `vocabulary_candidates` (docs/dictation.md §25): the pairs the AI clean-up changed in the recent
//! takes — what the recogniser wrote (`heard`) and what the take's final text has instead
//! (`polished`) — counted over the takes, most frequent first. These are the likely
//! misrecognitions an agent drafts dictionary entries from.
//!
//! The diff is the history page's: [`diff_tokens`] and [`diff_segments`] are the same
//! tokenisation and the same token-level LCS as `packages/shared/src/refine.ts` (`diffTokens`,
//! `diffSegments`), checked against each other through `packages/shared/src/fixtures/diff.json`.
//! (JavaScript's regular expression without the `u` flag splits a character outside the Basic
//! Multilingual Plane into two code units; here it stays one token. Texts of the BMP diff alike.)
//! A pair is a deleted run directly followed by an added one, with white space and punctuation
//! trimmed from both ends; a pair that is only punctuation, or longer than a dictionary form may
//! be, is left out.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::history::HistoryText;
use crate::vocabulary::{BACKTEST_SNIPPET_CHARS, DictionaryEntry, MAX_TERM_CHARS};

/// Most pairs one call returns.
pub const MAX_PAIRS: usize = 200;
/// Pairs a call returns unless asked otherwise.
pub const DEFAULT_PAIRS: usize = 50;
/// Snippets kept per pair.
pub const MAX_EXAMPLES: usize = 3;
/// A take whose diff table would be larger than this many cells (tokens × tokens) is left out
/// (a long recording; `history_search` reads it whole).
pub const MAX_DIFF_CELLS: usize = 1_000_000;
/// One call stops scanning past this many cells in all.
pub const MAX_SCAN_CELLS: usize = 50_000_000;

/// What one diff segment is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SegmentKind {
    /// In both texts.
    Same,
    /// Only in the final text.
    Add,
    /// Only in the recognised text.
    Del,
}

/// A run of tokens of one kind.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    /// Same, added or deleted.
    pub kind: SegmentKind,
    /// The tokens, joined.
    pub text: String,
}

/// The characters `refine.ts` keeps together as one word: ASCII letters and digits and
/// `_<>@#$%&*+=/\.:-`.
fn word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "_<>@#$%&*+=/\\.:-".contains(c)
}

/// JavaScript's `\s`: white space and line terminators (not U+0085, which Rust counts).
fn js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

/// `diffTokens`: a run of word characters with the white space after it, a run of white space, or
/// any other single character.
pub fn diff_tokens(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((start, c)) = chars.next() {
        let mut end = start + c.len_utf8();
        let mut extend = |keep: &dyn Fn(char) -> bool, end: &mut usize| {
            while let Some(&(i, next)) = chars.peek() {
                if !keep(next) {
                    break;
                }
                *end = i + next.len_utf8();
                chars.next();
            }
        };
        if word_char(c) {
            extend(&word_char, &mut end);
            extend(&js_space, &mut end);
        } else if js_space(c) {
            extend(&js_space, &mut end);
        }
        out.push(&text[start..end]);
    }
    out
}

/// `diffSegments`: the token-level LCS of `raw` → `text`, runs of one kind merged; ties go to the
/// deletion, as in `refine.ts`. `None` when the table would pass [`MAX_DIFF_CELLS`].
pub fn diff_segments(raw: &str, text: &str) -> Option<Vec<Segment>> {
    let a = diff_tokens(raw);
    let b = diff_tokens(text);
    let width = b.len() + 1;
    if (a.len() + 1).saturating_mul(width) > MAX_DIFF_CELLS {
        return None;
    }
    // table[i][j] = LCS length of a[i..] and b[j..].
    let mut table = vec![0u32; (a.len() + 1) * width];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            table[i * width + j] = if a[i] == b[j] { table[(i + 1) * width + j + 1] + 1 } else { table[(i + 1) * width + j].max(table[i * width + j + 1]) };
        }
    }
    let mut out: Vec<Segment> = Vec::new();
    let mut push = |kind: SegmentKind, token: &str| match out.last_mut() {
        Some(last) if last.kind == kind => last.text.push_str(token),
        _ => out.push(Segment { kind, text: token.to_owned() }),
    };
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            push(SegmentKind::Same, a[i]);
            i += 1;
            j += 1;
        } else if table[(i + 1) * width + j] >= table[i * width + j + 1] {
            push(SegmentKind::Del, a[i]);
            i += 1;
        } else {
            push(SegmentKind::Add, b[j]);
            j += 1;
        }
    }
    for token in &a[i..] {
        push(SegmentKind::Del, token);
    }
    for token in &b[j..] {
        push(SegmentKind::Add, token);
    }
    Some(out)
}

/// Punctuation and symbols a pair is trimmed of: ASCII punctuation, Latin-1's, general
/// punctuation, CJK symbols and punctuation, and the full-width and small forms.
pub fn is_mark(c: char) -> bool {
    c.is_ascii_punctuation()
        || matches!(
            c,
            '\u{a1}'..='\u{bf}'
                | '\u{d7}'
                | '\u{f7}'
                | '\u{2010}'..='\u{2027}'
                | '\u{2030}'..='\u{205e}'
                | '\u{3000}'..='\u{303f}'
                | '\u{30fb}'
                | '\u{fe10}'..='\u{fe19}'
                | '\u{fe30}'..='\u{fe6f}'
                | '\u{ff01}'..='\u{ff0f}'
                | '\u{ff1a}'..='\u{ff20}'
                | '\u{ff3b}'..='\u{ff40}'
                | '\u{ff5b}'..='\u{ff65}'
        )
}

/// `text` without white space and punctuation at either end.
fn trim_marks(text: &str) -> &str {
    text.trim_matches(|c: char| c.is_whitespace() || js_space(c) || is_mark(c))
}

/// A replacement inside one take: what was heard, what the final text has, and where the heard
/// run starts in the recognised text (in characters).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Replacement {
    /// Trimmed recognised run.
    pub heard: String,
    /// Trimmed final run.
    pub polished: String,
    /// Character offset of the (untrimmed) deleted run in `raw`.
    pub at: usize,
    /// Its length in characters.
    pub len: usize,
}

/// The replacements of one take: each deleted run directly followed by an added run, trimmed;
/// pairs that are empty after trimming, the same on both sides, or longer than
/// [`MAX_TERM_CHARS`] are left out. `None` when the take is too long to diff.
pub fn replacements(raw: &str, text: &str) -> Option<Vec<Replacement>> {
    let segments = diff_segments(raw, text)?;
    let mut out = Vec::new();
    let mut at = 0usize;
    for (i, segment) in segments.iter().enumerate() {
        let len = segment.text.chars().count();
        if segment.kind == SegmentKind::Del
            && let Some(next) = segments.get(i + 1)
            && next.kind == SegmentKind::Add
        {
            let (heard, polished) = (trim_marks(&segment.text), trim_marks(&next.text));
            let fits = |s: &str| !s.is_empty() && s.chars().count() <= MAX_TERM_CHARS;
            if fits(heard) && fits(polished) && heard != polished {
                out.push(Replacement { heard: heard.to_owned(), polished: polished.to_owned(), at, len });
            }
        }
        if segment.kind != SegmentKind::Add {
            at += len;
        }
    }
    Some(out)
}

/// One snippet of a pair: the recognised text around the heard run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairExample {
    /// The history entry.
    pub history_id: Uuid,
    /// Up to [`BACKTEST_SNIPPET_CHARS`] characters on each side of the run (`…` marks a cut).
    pub snippet: String,
}

/// One pair, counted over the takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pair {
    /// What the recogniser wrote.
    pub heard: String,
    /// What the final text has instead.
    pub polished: String,
    /// How often, in all takes scanned.
    pub count: u32,
    /// The newest take it happened in.
    pub last_at_ms: u64,
    /// `heard` is already a form an enabled dictionary entry corrects (ignoring ASCII case).
    pub known: bool,
    /// Up to [`MAX_EXAMPLES`] snippets, newest first.
    pub examples: Vec<PairExample>,
}

/// What `vocabulary_candidates` answers.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidates {
    /// Takes looked at.
    pub scanned: u32,
    /// Takes left out as too long to diff.
    pub skipped: u32,
    /// The pairs, most frequent first (then the most recent).
    pub pairs: Vec<Pair>,
}

/// The characters of `raw` around `at..at + len`, [`BACKTEST_SNIPPET_CHARS`] on each side.
fn snippet(raw: &str, at: usize, len: usize) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let start = at.saturating_sub(BACKTEST_SNIPPET_CHARS);
    let end = (at + len + BACKTEST_SNIPPET_CHARS).min(chars.len());
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.extend(&chars[start..end]);
    if end < chars.len() {
        out.push('…');
    }
    out
}

/// Count the pairs of `history` (newest first) at or after `since_ms`, the `limit` most frequent.
pub fn candidates(dictionary: &[DictionaryEntry], history: &[HistoryText], since_ms: Option<u64>, limit: usize) -> Candidates {
    let known: Vec<&str> = dictionary.iter().filter(|e| e.enabled).flat_map(|e| e.heard_as.iter().map(String::as_str)).collect();
    let mut out = Candidates::default();
    let mut index: HashMap<(String, String), usize> = HashMap::new();
    let mut pairs: Vec<Pair> = Vec::new();
    let mut cells = 0usize;
    for take in history.iter().filter(|t| since_ms.is_none_or(|since| t.at_ms >= since)) {
        let size = (diff_tokens(&take.raw_text).len() + 1).saturating_mul(diff_tokens(&take.text).len() + 1);
        if size > MAX_DIFF_CELLS {
            out.scanned = out.scanned.saturating_add(1);
            out.skipped = out.skipped.saturating_add(1);
            continue;
        }
        if cells.saturating_add(size) > MAX_SCAN_CELLS {
            break;
        }
        cells += size;
        out.scanned = out.scanned.saturating_add(1);
        let Some(found) = replacements(&take.raw_text, &take.text) else {
            out.skipped = out.skipped.saturating_add(1);
            continue;
        };
        for r in found {
            let key = (r.heard.clone(), r.polished.clone());
            let slot = *index.entry(key).or_insert_with(|| {
                pairs.push(Pair {
                    known: known.iter().any(|k| k.eq_ignore_ascii_case(&r.heard)),
                    heard: r.heard.clone(),
                    polished: r.polished.clone(),
                    count: 0,
                    last_at_ms: take.at_ms,
                    examples: Vec::new(),
                });
                pairs.len() - 1
            });
            let pair = &mut pairs[slot];
            pair.count = pair.count.saturating_add(1);
            pair.last_at_ms = pair.last_at_ms.max(take.at_ms);
            if pair.examples.len() < MAX_EXAMPLES && !pair.examples.iter().any(|e| e.history_id == take.id) {
                pair.examples.push(PairExample { history_id: take.id, snippet: snippet(&take.raw_text, r.at, r.len) });
            }
        }
    }
    pairs.sort_by(|a, b| b.count.cmp(&a.count).then(b.last_at_ms.cmp(&a.last_at_ms)));
    pairs.truncate(limit);
    out.pairs = pairs;
    out
}
