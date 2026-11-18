//! `vocabulary_candidates` (docs/dictation.md §25): the pairs the AI clean-up changed in the recent
//! takes — what the recogniser wrote (`heard`) and what the take's final text has instead
//! (`polished`) — counted over the takes, most frequent first. These are the likely
//! misrecognitions an agent drafts dictionary entries from. The final text is the polished text
//! after the dictionary and the rules, so a take the clean-up did not polish gives no pairs, and the
//! changes a take's own corrections and rules made are left out, as on the history page
//! (`takePolishPairs` in `packages/shared/src/quick-fix.ts`, §16.9).
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
use crate::vocabulary::{BACKTEST_SNIPPET_CHARS, DictionaryEntry, MAX_TERM_CHARS, ReplacementRule, Vocabulary};

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
    diff_segments_by(raw, text, |a, b| a == b)
}

/// [`diff_segments`] with `same` deciding which tokens are equal; a `Same` segment holds `raw`'s
/// tokens.
pub(crate) fn diff_segments_by(raw: &str, text: &str, same: impl Fn(&str, &str) -> bool) -> Option<Vec<Segment>> {
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
            table[i * width + j] = if same(a[i], b[j]) { table[(i + 1) * width + j + 1] + 1 } else { table[(i + 1) * width + j].max(table[i * width + j + 1]) };
        }
    }
    let mut out: Vec<Segment> = Vec::new();
    let mut push = |kind: SegmentKind, token: &str| match out.last_mut() {
        Some(last) if last.kind == kind => last.text.push_str(token),
        _ => out.push(Segment { kind, text: token.to_owned() }),
    };
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if same(a[i], b[j]) {
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

/// A replacement inside one take: what was heard, what the final text has, where the heard run
/// starts in the recognised text, and where the two trimmed forms sit (all in characters).
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
    /// Character offset of the trimmed `heard` in `raw`.
    pub heard_at: usize,
    /// Character offset of the trimmed `polished` in `text`.
    pub polished_at: usize,
}

/// The characters [`trim_marks`] takes off the front of `text`.
fn leading_marks(text: &str) -> usize {
    text.chars().take_while(|&c| c.is_whitespace() || js_space(c) || is_mark(c)).count()
}

/// The replacements of one take: each deleted run directly followed by an added run, trimmed;
/// pairs that are empty after trimming, the same on both sides, or longer than
/// [`MAX_TERM_CHARS`] are left out. `None` when the take is too long to diff.
pub fn replacements(raw: &str, text: &str) -> Option<Vec<Replacement>> {
    let segments = diff_segments(raw, text)?;
    let mut out = Vec::new();
    let (mut at, mut to) = (0usize, 0usize);
    for (i, segment) in segments.iter().enumerate() {
        let len = segment.text.chars().count();
        if segment.kind == SegmentKind::Del
            && let Some(next) = segments.get(i + 1)
            && next.kind == SegmentKind::Add
        {
            let (heard, polished) = (trim_marks(&segment.text), trim_marks(&next.text));
            let fits = |s: &str| !s.is_empty() && s.chars().count() <= MAX_TERM_CHARS;
            if fits(heard) && fits(polished) && heard != polished {
                out.push(Replacement {
                    heard: heard.to_owned(),
                    polished: polished.to_owned(),
                    at,
                    len,
                    heard_at: at + leading_marks(&segment.text),
                    polished_at: to + leading_marks(&next.text),
                });
            }
        }
        match segment.kind {
            SegmentKind::Same => {
                at += len;
                to += len;
            }
            SegmentKind::Del => at += len,
            SegmentKind::Add => to += len,
        }
    }
    Some(out)
}

/// A take's own corrections and rules (`take.vocabulary`, in their order then, on even if turned
/// off since) replayed over its recognised text as `dictionary` and `rules` define them now
/// (docs/dictation.md §16.9): what they turn a wording into, and where they changed the
/// recognised text. [`Replay::of`] is `None` when one of them was deleted or changed after the take
/// (`updated_at_ms` past the take's `at_ms`): their changes can no longer be told apart.
pub(crate) struct Replay {
    /// The fired entries and rules; `None` when nothing fired.
    vocabulary: Option<Vocabulary>,
    /// Their replacements over the whole recognised text.
    explained: Vec<Replacement>,
}

impl Replay {
    /// The replay of `take`'s fired vocabulary, or `None` when it changed since the take.
    pub(crate) fn of(take: &HistoryText, dictionary: &[DictionaryEntry], rules: &[ReplacementRule]) -> Option<Self> {
        let fired = &take.vocabulary;
        if fired.is_empty() {
            return Some(Self { vocabulary: None, explained: Vec::new() });
        }
        let mut entries = Vec::with_capacity(fired.corrections.len());
        for hit in &fired.corrections {
            match dictionary.iter().find(|e| e.id == hit.id) {
                Some(e) if e.updated_at_ms <= take.at_ms => entries.push(DictionaryEntry { enabled: true, ..e.clone() }),
                _ => return None,
            }
        }
        let mut firing = Vec::with_capacity(fired.rules.len());
        for hit in &fired.rules {
            match rules.iter().find(|r| r.id == hit.id) {
                Some(r) if r.updated_at_ms <= take.at_ms => firing.push(ReplacementRule { enabled: true, pending: false, ..r.clone() }),
                _ => return None,
            }
        }
        let mut replay = Self { vocabulary: Some(Vocabulary::compile(&entries, &firing)), explained: Vec::new() };
        replay.explained = replacements(&take.raw_text, &replay.run(&take.raw_text)).unwrap_or_default();
        Some(replay)
    }

    /// `text` through the fired corrections, then the fired rules.
    pub(crate) fn run(&self, text: &str) -> String {
        match &self.vocabulary {
            None => text.to_owned(),
            Some(vocabulary) => vocabulary.apply_rules(&vocabulary.correct(text).text).text,
        }
    }

    /// The fired vocabulary accounts for `r`: it turns the heard wording into the polished one, or
    /// makes the same replacement at the same place.
    fn explains(&self, r: &Replacement) -> bool {
        self.run(&r.heard) == r.polished
            || self.explained.iter().any(|q| q.heard == r.heard && q.polished == r.polished && q.at < r.at + r.len && r.at < q.at + q.len)
    }

    /// One of the fired vocabulary's replacements overlaps `len` characters at `at` of the
    /// recognised text.
    pub(crate) fn touches(&self, at: usize, len: usize) -> bool {
        self.explained.iter().any(|q| q.at < at + len && at < q.at + q.len)
    }
}

/// One pair of one take before the counting: the take (an index into the history), the two forms,
/// where they sit (characters into the recognised text, and into the text the right form comes
/// from), and whether the user's own correction made it (docs/dictation.md §35) rather than the AI
/// clean-up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Found {
    pub(crate) take: usize,
    pub(crate) heard: String,
    pub(crate) right: String,
    pub(crate) heard_at: usize,
    pub(crate) right_at: usize,
    pub(crate) edit: bool,
}

/// What [`scan`] read.
#[derive(Clone, Debug, Default)]
pub(crate) struct Scan {
    /// The pairs, take by take, newest first.
    pub(crate) found: Vec<Found>,
    /// Takes looked at.
    pub(crate) scanned: u32,
    /// Of those, the takes left out as too long to compare.
    pub(crate) skipped: u32,
    /// Of those, the takes the user corrected (`HistoryText::corrected`).
    pub(crate) edited: u32,
    /// The takes compared, as indices into the history.
    pub(crate) compared: Vec<usize>,
}

/// The diff table of two texts, in cells.
fn cells_of(a: &str, b: &str) -> usize {
    (diff_tokens(a).len() + 1).saturating_mul(diff_tokens(b).len() + 1)
}

/// The pairs of `history` (newest first) at or after `since_ms`: the AI clean-up's
/// ([`Replay::explains`] leaves out what the take's own corrections and rules did) and, with
/// `edits`, the user's corrections (`crate::vocabulary::learn`). A take whose diff would pass
/// [`MAX_DIFF_CELLS`] is skipped; past [`MAX_SCAN_CELLS`] in all, older takes are not read.
pub(crate) fn scan(dictionary: &[DictionaryEntry], rules: &[ReplacementRule], history: &[HistoryText], since_ms: Option<u64>, edits: bool) -> Scan {
    let mut out = Scan::default();
    let mut cells = 0usize;
    for (index, take) in history.iter().enumerate().filter(|(_, t)| since_ms.is_none_or(|since| t.at_ms >= since)) {
        let corrected = if edits { take.corrected.as_deref() } else { None };
        if !take.refined && corrected.is_none() {
            // Not polished and not corrected: every change in it is a correction's or a rule's.
            out.scanned = out.scanned.saturating_add(1);
            continue;
        }
        let polish = take.refined.then(|| cells_of(&take.raw_text, &take.text));
        let edit = corrected.map(|c| (cells_of(&take.text, c), cells_of(&take.raw_text, &take.text)));
        if polish.is_some_and(|size| size > MAX_DIFF_CELLS) || edit.is_some_and(|(a, b)| a > MAX_DIFF_CELLS || b > MAX_DIFF_CELLS) {
            out.scanned = out.scanned.saturating_add(1);
            out.skipped = out.skipped.saturating_add(1);
            continue;
        }
        // A take whose corrections or rules fired is diffed once more for the replay (counted as
        // the polish's diff again, as before the corrections were read).
        let replay_cost = if take.vocabulary.is_empty() { 0 } else { polish.unwrap_or_else(|| cells_of(&take.raw_text, &take.raw_text)) };
        let cost = polish.unwrap_or(0).saturating_add(edit.map_or(0, |(a, b)| a.saturating_add(b))).saturating_add(replay_cost);
        if cells.saturating_add(cost) > MAX_SCAN_CELLS {
            break;
        }
        cells += cost;
        out.scanned = out.scanned.saturating_add(1);
        out.compared.push(index);
        if corrected.is_some() {
            out.edited = out.edited.saturating_add(1);
        }
        let Some(replay) = Replay::of(take, dictionary, rules) else { continue };
        if take.refined {
            let Some(found) = replacements(&take.raw_text, &take.text) else {
                out.skipped = out.skipped.saturating_add(1);
                continue;
            };
            out.found.extend(found.into_iter().filter(|r| !replay.explains(r)).map(|r| Found {
                take: index,
                heard: r.heard,
                right: r.polished,
                heard_at: r.heard_at,
                right_at: r.polished_at,
                edit: false,
            }));
        }
        if let Some(corrected) = corrected {
            out.found.extend(crate::vocabulary::learn::edit_changes(take, corrected, &replay).into_iter().map(|c| Found {
                take: index,
                heard: c.heard,
                right: c.right,
                heard_at: c.heard_at,
                right_at: c.right_at,
                edit: true,
            }));
        }
    }
    out
}

/// A character of the scripts written without spaces whose single characters the dictionary's
/// literal matcher finds inside any word (Han, kana, Hangul).
pub(crate) fn is_cjk_letter(c: char) -> bool {
    c.is_alphanumeric()
        && matches!(
            c as u32,
            0x1100..=0x11FF
                | 0x3005..=0x3007
                | 0x3040..=0x30FF
                | 0x3130..=0x318F
                | 0x31F0..=0x31FF
                | 0x3400..=0x4DBF
                | 0x4E00..=0x9FFF
                | 0xA960..=0xA97F
                | 0xAC00..=0xD7FF
                | 0xF900..=0xFAFF
                | 0xFF66..=0xFF9F
                | 0xFFA0..=0xFFDC
                | 0x20000..=0x3134F
        )
}

/// `text` is one CJK character.
fn single_cjk(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(is_cjk_letter) && chars.next().is_none()
}

/// A pair as it is counted: widened when it was a single character (docs/dictation.md §16.10).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Counted {
    pub(crate) found: Found,
    /// The pair before it was widened, when it was.
    pub(crate) single: Option<(String, String)>,
    /// Characters of the (widened) heard form.
    pub(crate) heard_len: usize,
}

/// A way to widen one pair: the forms with the neighbour, where the heard form starts.
struct Widening {
    heard: String,
    right: String,
    heard_at: usize,
    right_at: usize,
}

/// The pairs of `scan` with every pair whose heard or right form is a single CJK character
/// widened by the same unchanged neighbour on both sides (docs/dictation.md §16.10): 非 → 飞 in
/// 「非书」 becomes 非书 → 飞书. With a neighbour on both sides, the side whose widened right form
/// appears more often in the compared takes' final texts and corrections wins, the right one on a
/// tie; with none, the pair stays as it is.
pub(crate) fn widen(history: &[HistoryText], scan: &Scan) -> Vec<Counted> {
    let chars = |text: &str| text.chars().collect::<Vec<char>>();
    let mut texts: HashMap<(usize, bool), (Vec<char>, Vec<char>)> = HashMap::new();
    let mut options: Vec<Vec<Widening>> = Vec::with_capacity(scan.found.len());
    for f in &scan.found {
        if !single_cjk(&f.heard) && !single_cjk(&f.right) {
            options.push(Vec::new());
            continue;
        }
        let take = &history[f.take];
        let (raw, right) = texts.entry((f.take, f.edit)).or_insert_with(|| {
            let right = if f.edit { take.corrected.as_deref().unwrap_or(&take.text) } else { &take.text };
            (chars(&take.raw_text), chars(right))
        });
        let (heard_len, right_len) = (f.heard.chars().count(), f.right.chars().count());
        let mut found = Vec::new();
        // Right first: it wins a tie.
        if let (Some(&a), Some(&b)) = (raw.get(f.heard_at + heard_len), right.get(f.right_at + right_len))
            && a == b
            && is_cjk_letter(a)
        {
            found.push(Widening { heard: format!("{}{a}", f.heard), right: format!("{}{a}", f.right), heard_at: f.heard_at, right_at: f.right_at });
        }
        if f.heard_at > 0
            && f.right_at > 0
            && let (Some(&a), Some(&b)) = (raw.get(f.heard_at - 1), right.get(f.right_at - 1))
            && a == b
            && is_cjk_letter(a)
        {
            found.push(Widening { heard: format!("{a}{}", f.heard), right: format!("{a}{}", f.right), heard_at: f.heard_at - 1, right_at: f.right_at - 1 });
        }
        options.push(found);
    }
    let mut patterns: Vec<String> = options.iter().filter(|o| o.len() > 1).flat_map(|o| o.iter().map(|w| w.right.clone())).collect();
    patterns.sort();
    patterns.dedup();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    if !patterns.is_empty()
        && let Ok(ac) = aho_corasick::AhoCorasick::new(&patterns)
    {
        for &index in &scan.compared {
            let take = &history[index];
            for text in std::iter::once(take.text.as_str()).chain(take.corrected.as_deref()) {
                for m in ac.find_overlapping_iter(text) {
                    *counts.entry(patterns[m.pattern().as_usize()].as_str()).or_default() += 1;
                }
            }
        }
    }
    scan.found
        .iter()
        .zip(options)
        .map(|(f, found)| {
            let count = |w: &Widening| counts.get(w.right.as_str()).copied().unwrap_or(0);
            let chosen = match found.as_slice() {
                [] => None,
                [one] => Some(one),
                [right, left, ..] => Some(if count(left) > count(right) { left } else { right }),
            };
            match chosen {
                None => Counted { found: f.clone(), single: None, heard_len: f.heard.chars().count() },
                Some(w) => Counted {
                    found: Found { heard: w.heard.clone(), right: w.right.clone(), heard_at: w.heard_at, right_at: w.right_at, ..f.clone() },
                    single: Some((f.heard.clone(), f.right.clone())),
                    heard_len: w.heard.chars().count(),
                },
            }
        })
        .collect()
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
    /// `heard` is already a form an enabled dictionary entry corrects (ignoring ASCII case), or
    /// its single character before the pair was widened is.
    pub known: bool,
    /// Up to [`MAX_EXAMPLES`] snippets, newest first.
    pub examples: Vec<PairExample>,
    /// Of `count`, the times the user's own correction made it (docs/dictation.md §35); never on
    /// the MCP tool's wire, whose scan reads no corrections.
    #[serde(skip)]
    pub from_edits: u32,
    /// The pair before it was widened (docs/dictation.md §16.10), when it was.
    #[serde(skip)]
    pub single: Option<(String, String)>,
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
    out.extend(&chars[start.min(end)..end]);
    if end < chars.len() {
        out.push('…');
    }
    out
}

/// The widened pairs counted: one per form pair, with how often, how often from the user's
/// corrections, the newest take, and up to [`MAX_EXAMPLES`] snippets (one per take, newest
/// first); most frequent first, then the most recent.
pub(crate) fn tally(dictionary: &[DictionaryEntry], history: &[HistoryText], counted: Vec<Counted>) -> Vec<Pair> {
    let known: Vec<&str> = dictionary.iter().filter(|e| e.enabled).flat_map(|e| e.heard_as.iter().map(String::as_str)).collect();
    let is_known = |heard: &str| known.iter().any(|k| k.eq_ignore_ascii_case(heard));
    let mut index: HashMap<(String, String), usize> = HashMap::new();
    let mut pairs: Vec<Pair> = Vec::new();
    for Counted { found, single, heard_len } in counted {
        let take = &history[found.take];
        let key = (found.heard.clone(), found.right.clone());
        let slot = *index.entry(key).or_insert_with(|| {
            pairs.push(Pair {
                known: is_known(&found.heard) || single.as_ref().is_some_and(|(h, _)| is_known(h)),
                heard: found.heard.clone(),
                polished: found.right.clone(),
                count: 0,
                last_at_ms: take.at_ms,
                examples: Vec::new(),
                from_edits: 0,
                single: single.clone(),
            });
            pairs.len() - 1
        });
        let pair = &mut pairs[slot];
        pair.count = pair.count.saturating_add(1);
        if found.edit {
            pair.from_edits = pair.from_edits.saturating_add(1);
        }
        pair.last_at_ms = pair.last_at_ms.max(take.at_ms);
        if pair.examples.len() < MAX_EXAMPLES && !pair.examples.iter().any(|e| e.history_id == take.id) {
            pair.examples.push(PairExample { history_id: take.id, snippet: snippet(&take.raw_text, found.heard_at, heard_len) });
        }
    }
    pairs.sort_by(|a, b| b.count.cmp(&a.count).then(b.last_at_ms.cmp(&a.last_at_ms)));
    pairs
}

/// Count the pairs of `history` (newest first) at or after `since_ms`, the `limit` most frequent:
/// only those the AI clean-up made (`rules` are the current rules), single characters widened.
pub fn candidates(dictionary: &[DictionaryEntry], rules: &[ReplacementRule], history: &[HistoryText], since_ms: Option<u64>, limit: usize) -> Candidates {
    let read = scan(dictionary, rules, history, since_ms, false);
    let mut pairs = tally(dictionary, history, widen(history, &read));
    pairs.truncate(limit);
    Candidates { scanned: read.scanned, skipped: read.skipped, pairs }
}
