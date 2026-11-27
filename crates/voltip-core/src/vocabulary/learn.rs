//! 从修改中学习 (docs/dictation.md §35): the user corrected a take in the application it went to,
//! selected the corrected text and asked Voltip to learn from it. [`match_selection`] finds the
//! takes the selection is the corrected form of and builds each one's whole corrected text;
//! [`storable`] says whether a correction holds anything to learn; [`edit_changes`] is what the
//! learning suggestions take from it: the replacements of the inserted text whose wording the
//! recogniser wrote and that neither the AI clean-up, the dictionary nor a rule touched — the
//! errors the clean-up's own changes cannot show (§16.10). Nothing here calls an AI service.

use std::collections::HashSet;

use crate::dictation::CONTEXT_CHARS;
use crate::dictation::insert_check::normalize;
use crate::history::HistoryText;
use crate::mcp::candidates::{MAX_DIFF_CELLS, MAX_SCAN_CELLS, Replacement, Replay, SegmentKind, diff_segments_by, diff_tokens, is_cjk_letter, replacements};

/// Recognised takes a selection is compared with, newest first.
pub const LEARN_TAKES: usize = 50;
/// The longest selection learned from, in characters.
pub const MAX_LEARN_SELECTION_CHARS: usize = 8000;
/// The least similarity at which a selection counts as the corrected form of a take.
pub const MATCH_MIN: f64 = 0.4;
/// The share of its words a followed take's text must keep (自动跟踪输入框里的修改,
/// docs/dictation.md §35.10): below it, the field holds other text and the take is gone (another
/// chat's draft in the same input box, everything selected and typed over, 2026-11-28). Counted in
/// words, not characters, so a word replaced by a shorter or longer spelling counts once
/// (「打开微信」 → 「打开WeChat」 keeps 2 of 4 words; 「one thousand」 → 「1,000」).
pub const FOLLOW_MATCH_MIN: f64 = 0.3;
/// Takes this close in similarity to the best one count as tied; the newer one wins.
pub const MATCH_TIE: f64 = 0.05;
/// Most takes one selection corrects (it may cover a few takes dictated one after another).
pub const MAX_TAKES_PER_SELECTION: usize = 5;
/// The longest run, in characters other than white space, compared character by character inside
/// one replacement; a longer one adds nothing to the similarity.
const INNER_MAX: usize = 64;

/// One replacement the user made that the learning suggestions learn from: the recogniser's
/// wording and the user's, where the first sits in the recognised text and the second in the
/// corrected text (characters).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EditChange {
    pub(crate) heard: String,
    pub(crate) right: String,
    pub(crate) heard_at: usize,
    pub(crate) right_at: usize,
}

/// The replacements of `take.text` → `corrected` whose inserted wording is the recogniser's: it
/// lies where `raw_text` → `text` keeps the tokens (ASCII case aside, so a clean-up that only
/// changed the case still counts), with the recognised wording and its offset in `raw_text`.
fn recognised_changes(take: &HistoryText, corrected: &str) -> Vec<(Replacement, usize)> {
    let Some(found) = replacements(&take.text, corrected) else { return Vec::new() };
    if found.is_empty() {
        return Vec::new();
    }
    let Some(segments) = diff_segments_by(&take.raw_text, &take.text, |a, b| a.eq_ignore_ascii_case(b)) else { return Vec::new() };
    // For each character of `text`, the character of `raw_text` it is, where the two keep it.
    let mut to_raw: Vec<Option<usize>> = Vec::new();
    let mut at = 0usize;
    for segment in &segments {
        let n = segment.text.chars().count();
        match segment.kind {
            SegmentKind::Same => {
                to_raw.extend((at..at + n).map(Some));
                at += n;
            }
            SegmentKind::Del => at += n,
            SegmentKind::Add => to_raw.extend(std::iter::repeat_n(None, n)),
        }
    }
    let raw: Vec<char> = take.raw_text.chars().collect();
    found
        .into_iter()
        .filter_map(|change| {
            let n = change.heard.chars().count();
            let start = to_raw.get(change.heard_at).copied().flatten()?;
            if !(0..n).all(|k| to_raw.get(change.heard_at + k).copied().flatten() == Some(start + k)) {
                return None;
            }
            let heard: String = raw.get(start..start + n)?.iter().collect();
            (heard != change.polished).then_some((Replacement { heard, ..change }, start))
        })
        .collect()
}

/// `corrected` holds a change worth keeping: it differs from what was inserted, and at least one
/// of its replacements is in the recogniser's own wording ([`recognised_changes`]). Whether the
/// take's dictionary corrections or rules explain it is left to the suggestions, which replay them.
pub fn storable(take: &HistoryText, corrected: &str) -> bool {
    corrected != take.text && !recognised_changes(take, corrected).is_empty()
}

/// The replacements of `corrected` the learning suggestions take: [`recognised_changes`] without
/// those the take's own corrections and rules made (`replay`: it overlaps one of their changes, or
/// turns the recognised wording into the user's).
pub(crate) fn edit_changes(take: &HistoryText, corrected: &str, replay: &Replay) -> Vec<EditChange> {
    recognised_changes(take, corrected)
        .into_iter()
        .filter(|(change, at)| !replay.touches(*at, change.heard.chars().count()) && replay.run(&change.heard) != change.polished)
        .map(|(change, at)| EditChange { heard: change.heard, right: change.polished, heard_at: at, right_at: change.polished_at })
        .collect()
}

/// One token of a text being matched: the token, the token without its trailing white space (what
/// two tokens are compared by: an application may end a line or the field differently), its
/// characters other than white space, and whether it is a word (letters, digits, one CJK character).
struct Tok<'a> {
    text: &'a str,
    key: &'a str,
    chars: usize,
    word: bool,
}

fn toks(text: &str) -> Vec<Tok<'_>> {
    diff_tokens(text)
        .into_iter()
        .map(|t| Tok {
            text: t,
            key: t.trim_end(),
            chars: t.chars().filter(|c| !c.is_whitespace()).count(),
            word: t.chars().next().is_some_and(char::is_alphanumeric),
        })
        .collect()
}

fn chars_of(toks: &[Tok<'_>]) -> usize {
    toks.iter().map(|t| t.chars).sum()
}

fn words_of(toks: &[Tok<'_>]) -> usize {
    toks.iter().filter(|t| t.word).count()
}

/// The characters of `toks` other than white space, ASCII letters in lower case.
fn folded(toks: &[Tok<'_>]) -> Vec<char> {
    toks.iter().flat_map(|t| t.text.chars()).filter(|c| !c.is_whitespace()).map(|c| c.to_ascii_lowercase()).collect()
}

/// The length of the longest common subsequence of two short character runs.
fn lcs_len(a: &[char], b: &[char]) -> usize {
    let mut row = vec![0usize; b.len() + 1];
    for &x in a {
        let mut diagonal = 0;
        for (j, &y) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = if x == y { diagonal + 1 } else { above.max(row[j]) };
            diagonal = above;
        }
    }
    row[b.len()]
}

/// The matched token pairs of the token-level LCS of `a` and `b`, ties going to `a`'s side as in
/// [`crate::mcp::candidates::diff_segments`]; `None` past [`MAX_DIFF_CELLS`].
fn lcs_pairs(a: &[Tok<'_>], b: &[Tok<'_>]) -> Option<Vec<(usize, usize)>> {
    let width = b.len() + 1;
    if (a.len() + 1).saturating_mul(width) > MAX_DIFF_CELLS {
        return None;
    }
    let mut table = vec![0u32; (a.len() + 1) * width];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            table[i * width + j] =
                if a[i].key == b[j].key { table[(i + 1) * width + j + 1] + 1 } else { table[(i + 1) * width + j].max(table[i * width + j + 1]) };
        }
    }
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].key == b[j].key {
            out.push((i, j));
            i += 1;
            j += 1;
        } else if table[(i + 1) * width + j] >= table[i * width + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    Some(out)
}

/// Which end of the take an unmatched run is at.
#[derive(Clone, Copy, PartialEq, Eq)]
enum End {
    Start,
    Finish,
}

/// How [`edge`] ranks a run: its character LCS with the missing run, whether it meets the text
/// around it at a word's edge, then fewer words apart from the missing run, then fewer characters.
type EdgeRank = (usize, bool, std::cmp::Reverse<usize>, std::cmp::Reverse<usize>);

/// The tokens of the selection's run `extra` that replace the take's run `missing` at one end of
/// the match (the error was the take's first or last words): whole tokens next to the matched part,
/// starting (at the start) or ending (at the finish) with a word, within one word of `missing`'s
/// count and half to twice its characters, the most similar character by character; on a tie the
/// one that meets the rest of `extra` at a word's edge ([`word_edge`]), then the one closest to
/// `missing`'s count of words, then the shortest. The rest of `extra` is text the user selected
/// besides the take. At most `limit` tokens are taken: with a paragraph read around the selection,
/// the change at a take's end lies in the selection, not in the text around it (the review of
/// 2026-11-25: 「我们讨论一下千问四Max…」 with 千问四 corrected took 「下」 of the text glued before
/// the take into the change). `None` when no run fits. The answer is how many tokens of `extra` are
/// taken, and their character LCS with `missing`.
fn edge(missing: &[Tok<'_>], extra: &[Tok<'_>], end: End, limit: usize) -> Option<(usize, usize)> {
    let (words, chars) = (words_of(missing), chars_of(missing));
    if words == 0 || words_of(extra) == 0 || chars > INNER_MAX {
        return None;
    }
    let target = folded(missing);
    let mut best: Option<(usize, EdgeRank)> = None;
    for taken in 1..=extra.len().min(limit) {
        let run = match end {
            End::Start => &extra[extra.len() - taken..],
            End::Finish => &extra[..taken],
        };
        let (run_words, run_chars) = (words_of(run), chars_of(run));
        if run_chars > 2 * chars {
            break;
        }
        let bounded = match end {
            End::Start => run[0].word,
            End::Finish => run[run.len() - 1].word,
        };
        if !bounded || run_words.abs_diff(words) > 1 || 2 * run_chars < chars {
            continue;
        }
        let score = lcs_len(&folded(run), &target);
        let rank = (score, word_edge(extra, taken, end), std::cmp::Reverse(run_words.abs_diff(words)), std::cmp::Reverse(run_chars));
        if best.as_ref().is_none_or(|(_, r)| rank > *r) {
            best = Some((taken, rank));
        }
    }
    best.map(|(taken, (score, ..))| (taken, score))
}

/// Whether the run of the last (at the start) or first (at the finish) `taken` tokens of `extra`
/// meets the rest of `extra` where one word ends and another begins: at the edge of the text, at
/// white space or punctuation, or where CJK text meets another script. With nothing selected and
/// the caret in 「我们讨论一下Qwen 4 Max的推理能力」 (千问四 corrected), 「Qwen 4」 begins at such a
/// place and 「下Qwen 4」 inside 「讨论一下」, the text glued before the take (2026-11-26: the
/// paragraph read around the caret learned 「千问四 → 下Qwen 4」).
fn word_edge(extra: &[Tok<'_>], taken: usize, end: End) -> bool {
    let (left, right) = match end {
        End::Start => match extra.len().checked_sub(taken + 1) {
            Some(i) => (&extra[i], &extra[i + 1]),
            None => return true,
        },
        End::Finish => match extra.get(taken) {
            Some(next) => (&extra[taken - 1], next),
            None => return true,
        },
    };
    if left.text.len() != left.key.len() || !left.word || !right.word {
        return true;
    }
    matches!((left.key.chars().next_back(), right.text.chars().next()), (Some(a), Some(b)) if is_cjk_letter(a) != is_cjk_letter(b))
}

/// Where a take and a selection meet, and how alike they are there.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Alignment {
    /// The take's tokens the selection covers.
    take: (usize, usize),
    /// The selection's tokens that are the take's.
    selection: (usize, usize),
    /// (Characters in the same tokens + the character LCS inside each replacement) ÷ the larger
    /// side's characters, white space not counted.
    similarity: f64,
}

/// Words on one side of a gap between two matched tokens past the other side's, from which the
/// match is taken to be two places rather than one replacement: the LCS also pairs a stray word of
/// another take the selection holds, or of a part of the take the user did not select.
const GAP_BREAK: usize = 3;

/// The run of `pairs` with the most matched characters, cut where a gap between two matched tokens
/// has more than [`GAP_BREAK`] words more on one side than on the other.
fn densest(take: &[Tok<'_>], selection: &[Tok<'_>], pairs: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut best: (usize, &[(usize, usize)]) = (0, &[]);
    let mut start = 0;
    for end in 1..=pairs.len() {
        let cut = end == pairs.len() || {
            let (p, q) = (pairs[end - 1], pairs[end]);
            words_of(&take[p.0 + 1..q.0]).abs_diff(words_of(&selection[p.1 + 1..q.1])) > GAP_BREAK
        };
        if cut {
            let run = &pairs[start..end];
            let matched = run.iter().map(|&(i, _)| take[i].chars).sum::<usize>();
            if matched > best.0 || best.1.is_empty() {
                best = (matched, run);
            }
            start = end;
        }
    }
    best.1.to_vec()
}

/// The alignment of `take` and `selection`, or `None` when they share no word. `inside`: the tokens
/// of `selection` the user selected in a paragraph read around it; a change at an end of the take
/// is taken from them only ([`edge`]).
fn align(take: &[Tok<'_>], selection: &[Tok<'_>], inside: Option<(usize, usize)>) -> Option<Alignment> {
    let pairs = densest(take, selection, &lcs_pairs(take, selection)?);
    if !pairs.iter().any(|&(i, _)| take[i].word) {
        return None;
    }
    let (first, last) = (pairs[0], pairs[pairs.len() - 1]);
    let mut matched: usize = pairs.iter().map(|&(i, _)| take[i].chars).sum();
    for window in pairs.windows(2) {
        let (p, q) = (window[0], window[1]);
        let (gap_take, gap_selection) = (&take[p.0 + 1..q.0], &selection[p.1 + 1..q.1]);
        if chars_of(gap_take) <= INNER_MAX && chars_of(gap_selection) <= INNER_MAX {
            matched += lcs_len(&folded(gap_take), &folded(gap_selection));
        }
    }
    let (lo, hi) = inside.unwrap_or((0, selection.len()));
    let (take_start, selection_start) = match edge(&take[..first.0], &selection[..first.1], End::Start, first.1.saturating_sub(lo)) {
        Some((taken, score)) => {
            matched += score;
            (0, first.1 - taken)
        }
        None => (first.0, first.1),
    };
    let (take_end, selection_end) = match edge(&take[last.0 + 1..], &selection[last.1 + 1..], End::Finish, hi.saturating_sub(last.1 + 1)) {
        Some((taken, score)) => {
            matched += score;
            (take.len(), last.1 + 1 + taken)
        }
        None => (last.0 + 1, last.1 + 1),
    };
    let size = chars_of(&take[take_start..take_end]).max(chars_of(&selection[selection_start..selection_end]));
    #[allow(clippy::cast_precision_loss)] // texts of at most a few thousand characters
    (size > 0).then(|| Alignment { take: (take_start, take_end), selection: (selection_start, selection_end), similarity: matched as f64 / size as f64 })
}

/// `base` with `range` replaced by `picked`. A selection that ends just before the take's next word
/// leaves out the space the take had there: the replaced part's trailing white space stays.
fn splice(base: &[Tok<'_>], range: (usize, usize), picked: &[Tok<'_>]) -> String {
    fn trailing<'a>(toks: &[Tok<'a>]) -> &'a str {
        toks.last().map_or("", |t| &t.text[t.key.len()..])
    }
    let join = |toks: &[Tok<'_>]| toks.iter().map(|t| t.text).collect::<String>();
    let (a, b) = range;
    let kept = if trailing(picked).is_empty() { trailing(&base[a..b]) } else { "" };
    format!("{}{}{kept}{}", join(&base[..a]), join(picked), join(&base[b..]))
}

/// The tokens of `corrected` that stand for `range` of `inserted` (the take's text and its earlier
/// correction): from the token `range.0` became, or right after the last one kept before it, to the
/// token `range.1 - 1` became, or up to the next one kept after it. A replacement the earlier
/// correction made across an end of `range` is inside; one wholly outside stays as corrected.
/// `None` past [`MAX_DIFF_CELLS`].
fn map_range(inserted: &[Tok<'_>], corrected: &[Tok<'_>], range: (usize, usize)) -> Option<(usize, usize)> {
    let pairs = lcs_pairs(inserted, corrected)?;
    let (a, b) = range;
    let start = match pairs.iter().find(|(i, _)| *i == a) {
        Some(&(_, j)) => j,
        None => pairs.iter().rev().find(|(i, _)| *i < a).map_or(0, |&(_, j)| j + 1),
    };
    let end = match pairs.iter().find(|(i, _)| *i + 1 == b) {
        Some(&(_, j)) => j + 1,
        None => pairs.iter().find(|(i, _)| *i >= b).map_or(corrected.len(), |&(_, j)| j),
    };
    Some((start, end.max(start)))
}

/// A take the selection corrects: which (an index into the takes given), its whole corrected text,
/// and how alike they were.
#[derive(Clone, Debug, PartialEq)]
pub struct Matched {
    /// Index into the takes [`match_selection`] was given.
    pub index: usize,
    /// The take's text with the selection's wording where the selection covers it: on top of the
    /// earlier correction when there is one, so a second correction keeps the first.
    pub corrected: String,
    /// See [`MATCH_MIN`].
    pub similarity: f64,
    /// The characters of the selection (as [`normalize`]d) the take covers, `[start, end)`.
    pub span: (usize, usize),
}

/// A candidate before the choice: `alignment` is with the corrected text, or with the inserted
/// text when the corrected one shares no word with the selection (`on_inserted`).
struct Candidate {
    index: usize,
    alignment: Alignment,
    on_inserted: bool,
    similarity: f64,
}

fn overlaps(a: (usize, usize), b: (usize, usize)) -> bool {
    a.0 < b.1 && b.0 < a.1
}

/// The takes (newest first) that `selection` is the corrected form of, newest first: each the most
/// alike take of a part of the selection (similarity at least [`MATCH_MIN`], a word in common; a
/// newer take wins within [`MATCH_TIE`]), at most [`MAX_TAKES_PER_SELECTION`], over parts that do
/// not overlap. A take already corrected is compared with both its corrected and its inserted
/// text, the higher similarity counting; its new corrected text builds on the earlier correction,
/// or on the inserted text when the corrected one has no word in common with the selection (the
/// original wording selected again to take a correction back).
pub fn match_selection(selection: &str, takes: &[HistoryText]) -> Vec<Matched> {
    match_at(selection, takes, None)
}

/// [`match_selection`], with only the takes whose part meets `focus` (characters of the normalised
/// selection: a range that is not empty must overlap the take's part, an empty one must lie in it
/// or at its edge) taking part in the choice, so takes elsewhere cannot take the places.
fn match_at(selection: &str, takes: &[HistoryText], focus: Option<(usize, usize)>) -> Vec<Matched> {
    let selection = normalize(selection);
    let sel = toks(&selection);
    // Characters before each token of the selection, white space included: a token's text is its
    // part of the selection, so the tokens join back into it.
    let offsets: Vec<usize> = std::iter::once(0)
        .chain(sel.iter().scan(0usize, |at, t| {
            *at += t.text.chars().count();
            Some(*at)
        }))
        .collect();
    let span = |a: &Alignment| (offsets[a.selection.0], offsets[a.selection.1]);
    // The tokens a selection inside the paragraph covers: from the one it starts in to the one it
    // ends in.
    let inside = focus.filter(|(at, to)| to > at).map(|(at, to)| {
        let first = offsets.iter().rposition(|&o| o <= at).unwrap_or(0).min(sel.len());
        let last = offsets.iter().position(|&o| o >= to).unwrap_or(sel.len()).max(first);
        (first, last)
    });
    let meets = |a: &Alignment| {
        let (start, end) = span(a);
        focus.is_none_or(|(at, to)| if to > at { start < to && at < end } else { start <= at && at <= end })
    };
    let words: HashSet<&str> = sel.iter().filter(|t| t.word).map(|t| t.key).collect();
    let shares = |toks: &[Tok<'_>]| toks.iter().any(|t| t.word && words.contains(t.key));
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut cells = 0usize;
    for (index, take) in takes.iter().enumerate() {
        let corrected = take.corrected.as_deref().map(toks).filter(|t| shares(t));
        let inserted = Some(toks(&take.text)).filter(|t| (take.corrected.is_some() || corrected.is_none()) && shares(t));
        if corrected.is_none() && inserted.is_none() {
            continue;
        }
        let cost =
            [&corrected, &inserted].iter().filter_map(|t| t.as_ref()).fold(0usize, |sum, t| sum.saturating_add((t.len() + 1).saturating_mul(sel.len() + 1)));
        if cells.saturating_add(cost) > MAX_SCAN_CELLS {
            break;
        }
        cells += cost;
        let on_corrected = corrected.as_deref().and_then(|t| align(t, &sel, inside));
        let on_inserted = inserted.as_deref().and_then(|t| align(t, &sel, inside));
        // The alignment that wins decides the rebuild: the more alike, then the one covering more
        // of the selection; the corrected text on a full tie.
        let covered = |a: &Alignment| chars_of(&sel[a.selection.0..a.selection.1]);
        let (alignment, from_inserted) = match (on_corrected, on_inserted) {
            (Some(c), Some(i)) if i.similarity > c.similarity + 1e-9 || ((i.similarity - c.similarity).abs() <= 1e-9 && covered(&i) > covered(&c)) => (i, true),
            (Some(c), _) => (c, false),
            (None, Some(i)) => (i, true),
            (None, None) => continue,
        };
        if alignment.similarity >= MATCH_MIN && meets(&alignment) {
            candidates.push(Candidate { index, alignment, on_inserted: from_inserted, similarity: alignment.similarity });
        }
    }
    let mut chosen: Vec<Candidate> = Vec::new();
    while chosen.len() < MAX_TAKES_PER_SELECTION {
        let Some(best) = candidates.iter().max_by(|a, b| a.similarity.total_cmp(&b.similarity).then(b.index.cmp(&a.index))) else { break };
        let (best_selection, best_similarity) = (best.alignment.selection, best.similarity);
        let Some(pick) = candidates
            .iter()
            .enumerate()
            .filter(|(_, c)| overlaps(c.alignment.selection, best_selection) && c.similarity >= best_similarity - MATCH_TIE)
            .min_by_key(|(_, c)| c.index)
            .map(|(i, _)| i)
        else {
            break;
        };
        let pick = candidates.swap_remove(pick);
        candidates.retain(|c| !overlaps(c.alignment.selection, pick.alignment.selection));
        chosen.push(pick);
    }
    chosen.sort_by_key(|c| c.index);
    chosen
        .into_iter()
        .map(|c| {
            let take = &takes[c.index];
            let picked = &sel[c.alignment.selection.0..c.alignment.selection.1];
            let corrected = match (c.on_inserted, take.corrected.as_deref()) {
                // On the inserted text, of a take corrected before: the selection replaces the part of
                // the corrected text that stands for the aligned part, the rest keeps its correction.
                (true, Some(earlier)) => {
                    let inserted = toks(&take.text);
                    let earlier = toks(earlier);
                    match map_range(&inserted, &earlier, c.alignment.take) {
                        Some(range) => splice(&earlier, range, picked),
                        None => splice(&inserted, c.alignment.take, picked),
                    }
                }
                (true | false, None) => splice(&toks(&take.text), c.alignment.take, picked),
                (false, Some(earlier)) => splice(&toks(earlier), c.alignment.take, picked),
            };
            Matched { index: c.index, corrected, similarity: c.similarity, span: span(&c.alignment) }
        })
        .collect()
}

/// The takes (newest first) a paragraph read around the selection corrects (docs/dictation.md §35,
/// the focused control's [`crate::dictation::FocusContext`]): [`match_selection`] over `before`,
/// `selected` and `after` together, where only the takes whose part overlaps the selection, or,
/// with nothing selected, holds or touches the caret between `before` and `after`, take part in
/// the choice. A take elsewhere in the paragraph is left alone, and cannot take the places of the
/// one the user pointed at (the review of 2026-11-25).
pub fn match_around(before: &str, selected: &str, after: &str, takes: &[HistoryText]) -> Vec<Matched> {
    let (before, selected, after) = (normalize(before), normalize(selected), normalize(after));
    let (at, len) = (before.chars().count(), selected.chars().count());
    match_at(&format!("{before}{selected}{after}"), takes, Some((at, at + len)))
}

/// What a token of a followed paragraph is (自动跟踪输入框里的修改, docs/dictation.md §35.10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Part {
    /// Text before the take that was there at its paste.
    Before,
    /// Text typed between that and the take since.
    Head,
    /// The take's tokens, and what replaced them in the field.
    Take,
    /// Text typed into the take between two of its tokens: the take's, but no replacement of a
    /// word of it (2026-11-28: 「最新的」 typed before the last word 千问四, then the word typed
    /// again, was taken for its replacement).
    Added,
    /// Text typed right after the take since its paste.
    Tail,
    /// Text after the take that was there at its paste.
    After,
}

/// A followed take in a paragraph read from the field it went to: the paragraph as [`normalize`]d,
/// what each of its tokens is, and the take's characters as read, `[start, end)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FollowSpot {
    pub(crate) paragraph: String,
    parts: Vec<Part>,
    read: (usize, usize),
}

impl FollowSpot {
    /// The field's text of the take.
    pub(crate) fn text(&self) -> String {
        self.paragraph.chars().skip(self.read.0).take(self.read.1.saturating_sub(self.read.0)).collect()
    }

    /// The take is what was read: the follow wrote it. Text read as the replacement of its first
    /// or last word is the take's from now on, and what is typed next to it later is not (2026-11-28:
    /// 「通义」 stored for 千问四 became 「通义模」 when 「模型」 was typed after it).
    pub(crate) fn fix(&mut self) {
        let offsets = char_offsets(&toks(&self.paragraph));
        for (k, w) in offsets.windows(2).enumerate() {
            if w[0] >= self.read.0 && w[1] <= self.read.1 && w[1] > w[0] {
                self.parts[k] = Part::Take;
            }
        }
    }
}

/// Whether a token is the take's: its own, or text typed into it.
fn of_take(part: Part) -> bool {
    matches!(part, Part::Take | Part::Added)
}

/// What a later reading says of a followed take ([`follow_next`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FollowNext {
    /// The take as the field shows it now.
    Read(FollowSpot),
    /// The take is there, but a word at its start or end is being typed and cannot be told yet
    /// (「Q」 for 千问四): the place is carried on, nothing is read.
    Typing(FollowSpot),
    /// The reading's window cuts the take at its start or end: it cannot tell what the user left
    /// of it.
    Cut,
    /// No token of the take is left: sent, deleted or undone.
    Gone,
    /// Past [`MAX_DIFF_CELLS`]: this reading is not compared.
    TooLong,
}

/// The take in the first reading after its paste, where the caret is at it or next to it: its
/// place in the paragraph `before` `selected` `after`, `None` when the paragraph holds nothing like
/// it. The insert check has just read the take right before the caret: when it is still there as
/// inserted, that is its place; otherwise (the application rewrote it, the user went on) the take
/// is matched as [`match_around`] does, compared as inserted (its `corrected` is not looked at).
/// The match alone took text before the take that ends as the take starts for part of it
/// (2026-11-28: 「好的，」 before a take starting with 「的」 gave 「的，的服务…」).
pub(crate) fn follow_first(before: &str, selected: &str, after: &str, take: &HistoryText) -> Option<FollowSpot> {
    let (before, selected, after) = (normalize(before), normalize(selected), normalize(after));
    let (at, len) = (before.chars().count(), selected.chars().count());
    let paragraph = format!("{before}{selected}{after}");
    let offsets = char_offsets(&toks(&paragraph));
    let inserted = normalize(&take.text);
    let anchored = (selected.is_empty() && !inserted.is_empty() && before.ends_with(inserted.as_str()))
        .then(|| (at - inserted.chars().count(), at))
        .filter(|(from, to)| offsets.contains(from) && offsets.contains(to));
    let span = match anchored {
        Some(span) => span,
        None => {
            let take = HistoryText { corrected: None, ..take.clone() };
            match_at(&paragraph, std::slice::from_ref(&take), Some((at, at + len))).into_iter().next()?.span
        }
    };
    let parts = offsets
        .windows(2)
        .map(|w| {
            if w[1] <= span.0 {
                Part::Before
            } else if w[0] >= span.1 {
                Part::After
            } else {
                Part::Take
            }
        })
        .collect();
    Some(FollowSpot { paragraph, parts, read: span })
}

/// Characters of other text a reading must show between its window's full edge and the take, for
/// the take's end there to be in view: a character or two before the take can match the window's
/// first characters by chance (2026-11-28: 「的」 of 「我说的」 matched the 「的」 the window started
/// at, inside the take).
pub(crate) const CUT_MARGIN: usize = 8;

/// The take in a later reading (docs/dictation.md §35.10). Each token of the paragraph carries
/// what it was in the previous reading over the two readings' diff ([`carry`]): a kept token stays
/// what it was; a run that replaced tokens of the take is the take's; one typed right before or
/// after the take is text typed next to it ([`Part::Head`], [`Part::Tail`]). `reference` is the
/// field's text of the take at the paste, or when the follow last wrote: where its first or last
/// words are gone, what was typed there may be their replacement ([`pick`]); where they are still
/// there, the take starts or ends with them, and text joined to it there is not the take's. A
/// reading whose take keeps no word of `reference`, or is less alike to it than
/// [`FOLLOW_MATCH_MIN`], has lost it ([`FollowNext::Gone`]).
pub(crate) fn follow_next(previous: &FollowSpot, before: &str, selected: &str, after: &str, reference: &str) -> FollowNext {
    let (before, selected, after) = (normalize(before), normalize(selected), normalize(after));
    let paragraph = format!("{before}{selected}{after}");
    let (old, new) = (toks(&previous.paragraph), toks(&paragraph));
    let offsets = char_offsets(&new);
    let caret = offsets.iter().position(|&o| o >= before.chars().count()).unwrap_or(new.len()).min(new.len());
    let Some((mut parts, _, stayed)) = carry(&old, &previous.parts, &new, caret) else { return FollowNext::TooLong };
    let (Some(lo), Some(last)) = (parts.iter().position(|&p| of_take(p)), parts.iter().rposition(|&p| of_take(p))) else {
        return FollowNext::Gone;
    };
    // With the whole paragraph in view, half or more of the take's words gone in one reading, in
    // two places or more, is other text in the field, not an edit (another chat's draft in the
    // same input box; 2026-11-28: one sharing some of the take's words was stored as its
    // correction). One edit changes one place, even when letters typed next to a word join it
    // (「Gour」 for 「get hub our」 while 「GitHub」 is typed).
    if before.chars().count() < CONTEXT_CHARS && after.chars().count() < CONTEXT_CHARS {
        let words: Vec<bool> = old.iter().zip(&previous.parts).zip(&stayed).filter(|&((t, &p), _)| t.word && of_take(p)).map(|(_, &k)| k).collect();
        let kept = words.iter().filter(|&&k| k).count();
        let places = words.iter().enumerate().filter(|&(i, &k)| !k && (i == 0 || words[i - 1])).count();
        if 2 * kept <= words.len() && places >= 2 {
            return FollowNext::Gone;
        }
    }
    let hi = last + 1;
    for part in &mut parts[lo..hi] {
        if !of_take(*part) {
            *part = Part::Added;
        }
    }
    let reference = toks(reference);
    // A word of the take the user replaced can match the same character typed in its place, or
    // after it in the same reading (「Qwen 4，我问了四…」 for 千问四): such short matches by chance
    // do not keep the take's end ([`clean`]).
    let Some(pairs) = anchored_pairs(&reference, &new[lo..hi], caret.clamp(lo, hi) - lo).map(|p| clean(&reference, &new[lo..hi], p, |_| false, 2)) else {
        return FollowNext::TooLong;
    };
    let (Some(&(first, at_first)), Some(&(last, at_last))) = (pairs.first(), pairs.last()) else { return FollowNext::Gone };
    let (mut read_lo, mut read_hi) = (lo + at_first, lo + at_last + 1);
    let mut typing = false;
    if first > 0 {
        // Text typed into the take right after where its first words were stays the take's; what
        // replaced them is before it.
        let kept = (lo..lo + at_first).rev().take_while(|&k| parts[k] == Part::Added).last().unwrap_or(lo + at_first);
        let from = (0..lo).rev().take_while(|&k| parts[k] == Part::Head).last().unwrap_or(lo);
        match pick(&reference[..first], &new, (from, kept), End::Start, kept - lo) {
            Some(taken) => read_lo = kept - taken,
            None => typing = true,
        }
    }
    if last + 1 < reference.len() {
        let kept = (lo + at_last + 1..hi).take_while(|&k| parts[k] == Part::Added).last().map_or(lo + at_last + 1, |k| k + 1);
        let to = (hi..new.len()).take_while(|&k| parts[k] == Part::Tail).last().map_or(hi, |k| k + 1);
        match pick(&reference[last + 1..], &new, (kept, to), End::Finish, hi - kept) {
            Some(taken) => read_hi = kept + taken,
            None => typing = true,
        }
    }
    let (read_lo, read_hi) = if typing { (lo, hi) } else { (read_lo, read_hi) };
    // The window's edge, where that side is full, may lie inside the take, even inside one of its
    // words or in a word typed in place of one (「etes集群…」 of 「Kubernetes集群…」): what is read
    // must have other text before (after) it in view (2026-11-28: the margin measured to the
    // take's kept words let 「ubernetes…」 through).
    let total = offsets[new.len()];
    if (before.chars().count() >= CONTEXT_CHARS && offsets[read_lo] < CUT_MARGIN)
        || (after.chars().count() >= CONTEXT_CHARS && total - offsets[read_hi] < CUT_MARGIN)
    {
        return FollowNext::Cut;
    }
    // Less of the take's words left than FOLLOW_MATCH_MIN of them: the field holds other text.
    let words = words_of(&reference);
    let kept = pairs.iter().filter(|&&(i, _)| reference[i].word).count();
    #[allow(clippy::cast_precision_loss)] // a few hundred words at most
    if words == 0 || (kept as f64) < FOLLOW_MATCH_MIN * words as f64 {
        return FollowNext::Gone;
    }
    if typing {
        return FollowNext::Typing(FollowSpot { paragraph, parts, read: (offsets[lo], offsets[hi]) });
    }
    // What is left of the take's run outside what was read is text next to it. Text read as the
    // replacement of a first or last word stays what it was typed as until the follow writes it
    // ([`FollowSpot::fix`]): the next reading decides again.
    parts[lo..read_lo.max(lo)].fill(Part::Head);
    parts[read_hi.min(hi)..hi].fill(Part::Tail);
    // A full stop typed right after the take's last word joins it into one token (「model.」):
    // it is what follows the take, unless the word was replaced by one that ends with its own
    // (「U.S.」 for 「us」).
    let mut end = offsets[read_hi];
    if let (Some(word), Some(was)) = (new[read_lo..read_hi].last(), reference.last()) {
        let bare = word.key.trim_end_matches('.');
        if bare.len() < word.key.len() && !bare.is_empty() && bare == was.key {
            end = offsets[read_hi - 1] + bare.chars().count();
        }
    }
    FollowNext::Read(FollowSpot { paragraph, parts, read: (offsets[read_lo], end) })
}

/// What each token of `new` is, from `old`'s `parts` over the diff of the two readings
/// ([`follow_next`]), which tokens of `new` were in `old`, and which tokens of `old` are still in
/// `new`. The diff is anchored at the caret
/// ([`anchored_pairs`]): the user types, deletes and replaces before it, so 「我」 typed before a
/// take starting with 「我们」 is the new token, not the take's (2026-11-28: the LCS from the start
/// matched the take's 「我」 to it). Matches by chance in what
/// is left are dropped ([`clean`]), never in the text around the take, which was there at its
/// paste (2026-11-28: a 「，」 before the take dropped while the window's start moved over
/// 「好好好…」 made the text before it part of the take). `None` past [`MAX_DIFF_CELLS`].
fn carry(old: &[Tok<'_>], parts: &[Part], new: &[Tok<'_>], caret: usize) -> Option<(Vec<Part>, Vec<bool>, Vec<bool>)> {
    let pairs = clean(old, new, anchored_pairs(old, new, caret)?, |i| matches!(parts[i], Part::Before | Part::After), 2);
    let mut out = vec![Part::Take; new.len()];
    let mut kept = vec![false; new.len()];
    let mut stayed = vec![false; old.len()];
    for &(i, _) in &pairs {
        stayed[i] = true;
    }
    let (mut i0, mut j0) = (0usize, 0usize);
    for &(i, j) in pairs.iter().chain(std::iter::once(&(old.len(), new.len()))) {
        if j > j0 {
            let deleted = &parts[i0..i];
            let part = if deleted.contains(&Part::Take) {
                Part::Take
            } else if deleted.contains(&Part::Added) {
                Part::Added
            } else if deleted.contains(&Part::Head) {
                Part::Head
            } else if deleted.contains(&Part::Tail) {
                Part::Tail
            } else if let Some(&part) = deleted.first() {
                part
            } else {
                inserted(i0.checked_sub(1).map(|k| parts[k]), parts.get(i).copied())
            };
            out[j0..j].fill(part);
        }
        if j < new.len() {
            out[j] = parts[i];
            kept[j] = true;
        }
        (i0, j0) = (i + 1, j + 1);
    }
    slide_to_edges(new, &mut out, &mut kept);
    Some((out, kept, stayed))
}

/// Text inserted into the take right after its first token, that the same diff could as well put
/// before it, goes before it (and the same at its last token): an insertion read with the caret
/// already elsewhere (2026-11-28: 「我觉得」 typed before 「我们…」, read with the caret at the end,
/// matched the take's 「我」 to the typed one and became part of the take).
fn slide_to_edges(new: &[Tok<'_>], parts: &mut [Part], kept: &mut [bool]) {
    let (Some(lo), Some(last)) = (parts.iter().position(|&p| of_take(p)), parts.iter().rposition(|&p| of_take(p))) else { return };
    // An inserted run right after the first token: [lo] kept, then lo + 1 .. end inserted.
    let end = (lo + 1..=last).take_while(|&k| parts[k] == Part::Added && !kept[k]).last().map_or(lo + 1, |k| k + 1);
    if end > lo + 1 && kept[lo] && end <= last && new[lo].key == new[end - 1].key {
        parts[lo..end - 1].fill(Part::Head);
        (kept[lo], kept[end - 1]) = (false, true);
        parts[end - 1] = Part::Take;
        return;
    }
    // An inserted run right before the last token.
    let start = (lo..last).rev().take_while(|&k| parts[k] == Part::Added && !kept[k]).last().unwrap_or(last);
    if start < last && kept[last] && start > lo && new[last].key == new[start].key {
        parts[start + 1..=last].fill(Part::Tail);
        (kept[last], kept[start]) = (false, true);
        parts[start] = Part::Take;
    }
}

/// The kept tokens of `old` → `new` (pairs of indices) where the edit between them was made at the
/// caret, `caret` a token of `new`: the tokens from the caret on are matched from the end first,
/// as far as they are the same, then the tokens from the start, and only what is left between is
/// compared by token LCS. Where the LCS alone has two equal matches it takes the first, which pairs
/// a word of the take with the same word typed before it, or with a word inside the one the user
/// changed (「its it it」 → 「it's it it」 paired the second 「it」 with the 「it」 of 「it's」).
/// `None` past [`MAX_DIFF_CELLS`].
fn anchored_pairs(old: &[Tok<'_>], new: &[Tok<'_>], caret: usize) -> Option<Vec<(usize, usize)>> {
    let suffix = (0..new.len().saturating_sub(caret).min(old.len())).take_while(|&k| old[old.len() - 1 - k].key == new[new.len() - 1 - k].key).count();
    let prefix = (0..(old.len() - suffix).min(new.len() - suffix)).take_while(|&k| old[k].key == new[k].key).count();
    let middle = lcs_pairs(&old[prefix..old.len() - suffix], &new[prefix..new.len() - suffix])?;
    Some(
        (0..prefix)
            .map(|k| (k, k))
            .chain(middle.into_iter().map(|(i, j)| (i + prefix, j + prefix)))
            .chain((0..suffix).rev().map(|k| (old.len() - 1 - k, new.len() - 1 - k)))
            .collect(),
    )
}

/// What text inserted between a token that was `before` and one that was `after` is (`None` at the
/// paragraph's edge): typed into the take, next to it, or into the text around it.
fn inserted(before: Option<Part>, after: Option<Part>) -> Part {
    use Part::{Added, Head, Tail, Take};
    match (before, after) {
        (Some(a), Some(b)) if of_take(a) && of_take(b) => Added,
        (Some(a), Some(b)) if a == b => a,
        (Some(Take | Added), _) => Tail,
        (_, Some(Take | Added)) => Head,
        (None, Some(b)) => b,
        (Some(a), None) => a,
        (Some(a), Some(b)) if a == Head || b == Head => Head,
        (Some(a), Some(b)) if a == Tail || b == Tail => Tail,
        (Some(a), Some(_)) => a,
        (None, None) => Take,
    }
}

/// `pairs` without each run of kept tokens that has larger changes on both sides of it, as
/// diff-match-patch's semantic clean-up does: there they are a match by chance, not text kept (a
/// word of the take the user replaced, matched in what was typed in its place). A run holding a
/// token of `a` that `fixed` names stays, so does one of more than `largest` characters, and the
/// last run left (2026-11-28: both ends of a take replaced at once dropped its kept middle).
fn clean(a: &[Tok<'_>], b: &[Tok<'_>], mut pairs: Vec<(usize, usize)>, fixed: impl Fn(usize) -> bool, largest: usize) -> Vec<(usize, usize)> {
    loop {
        // Runs of pairs one after the other on both sides, as [start, end) into `pairs`.
        let mut runs: Vec<(usize, usize)> = Vec::new();
        for (n, &(i, j)) in pairs.iter().enumerate() {
            match runs.last_mut() {
                Some(run) if pairs[run.1 - 1] == (i.wrapping_sub(1), j.wrapping_sub(1)) => run.1 = n + 1,
                _ => runs.push((n, n + 1)),
            }
        }
        let changed = |from: (usize, usize), to: (usize, usize)| chars_of(&a[from.0..to.0]).max(chars_of(&b[from.1..to.1]));
        let drop = runs.iter().enumerate().position(|(r, &(start, end))| {
            let ((i, j), (k, l)) = (pairs[start], pairs[end - 1]);
            let before = changed(if r == 0 { (0, 0) } else { (pairs[runs[r - 1].1 - 1].0 + 1, pairs[runs[r - 1].1 - 1].1 + 1) }, (i, j));
            let after = changed((k + 1, l + 1), runs.get(r + 1).map_or((a.len(), b.len()), |next| pairs[next.0]));
            let size = chars_of(&a[i..=k]);
            runs.len() > 1 && size <= largest && !pairs[start..end].iter().any(|&(i, _)| fixed(i)) && before > 0 && after > 0 && size <= before && size <= after
        });
        let Some(r) = drop else { return pairs };
        pairs.drain(runs[r].0..runs[r].1);
    }
}

/// The size [`pick`] compares: a CJK letter counts two, any other character but white space one
/// (「千问四」 is 6, 「Qwen 4」 5).
fn units(toks: &[Tok<'_>]) -> usize {
    toks.iter().flat_map(|t| t.text.chars()).filter(|c| !c.is_whitespace()).map(|c| if is_cjk_letter(c) { 2 } else { 1 }).sum()
}

/// A token that is neither a word nor white space.
fn is_punctuation(t: &Tok<'_>) -> bool {
    !t.word && !t.key.is_empty()
}

/// [`folded`], with Chinese numerals as digits: 「千问四」 and 「Qwen 4」 share the 4.
fn spelled(toks: &[Tok<'_>]) -> Vec<char> {
    const NUMERALS: [(char, char); 12] = [
        ('零', '0'),
        ('〇', '0'),
        ('一', '1'),
        ('二', '2'),
        ('两', '2'),
        ('三', '3'),
        ('四', '4'),
        ('五', '5'),
        ('六', '6'),
        ('七', '7'),
        ('八', '8'),
        ('九', '9'),
    ];
    folded(toks).into_iter().map(|c| NUMERALS.iter().find(|(n, _)| *n == c).map_or(c, |&(_, d)| d)).collect()
}

/// The edit distance of two short character runs.
fn distance(a: &[char], b: &[char]) -> usize {
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, &x) in a.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, &y) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (above + 1).min(row[j] + 1).min(diagonal + usize::from(x != y));
            diagonal = above;
        }
    }
    row[b.len()]
}

/// Whether `toks` mix CJK letters with other letters or digits.
fn mixed(toks: &[Tok<'_>]) -> bool {
    let letters = || toks.iter().flat_map(|t| t.text.chars()).filter(|c| c.is_alphanumeric());
    letters().any(is_cjk_letter) && letters().any(|c| !is_cjk_letter(c))
}

/// Marks that go with the word they touch: quotes and brackets (「《三体》」).
const QUOTES: &str = "《》〈〉「」『』（）()[]{}【】\"'‘’“”";

/// Opening marks and the closing ones that go with them.
const PAIRED: [(char, char); 10] =
    [('《', '》'), ('〈', '〉'), ('「', '」'), ('『', '』'), ('（', '）'), ('(', ')'), ('[', ']'), ('{', '}'), ('【', '】'), ('“', '”')];

/// Whether the opening and closing marks in `toks` pair up, so a run does not end inside a title
/// or a quote (「《三」 or 「体》」 of 「《三体》」, closer in spelling to 「三提」).
fn balanced(toks: &[Tok<'_>]) -> bool {
    PAIRED.iter().all(|&(open, close)| {
        let chars = || toks.iter().flat_map(|t| t.key.chars());
        chars().filter(|&c| c == open).count() == chars().filter(|&c| c == close).count()
    })
}

/// Whether the mark `new[g]` belongs to the word next to it rather than ending it ([`pick`]): a
/// comma between digits (「1,000」), or a quote, apostrophe or bracket touching a word (「don't」,
/// 「boys'」, 「《三体》」; 2026-11-28: the replacement was cut at the mark).
fn joins(new: &[Tok<'_>], g: usize) -> bool {
    let mark = &new[g];
    let left = g.checked_sub(1).map(|p| &new[p]).filter(|p| p.word && p.text.len() == p.key.len());
    let right = new.get(g + 1).filter(|n| n.word && mark.text.len() == mark.key.len());
    let (last, next) = (left.and_then(|p| p.key.chars().next_back()), right.and_then(|n| n.key.chars().next()));
    match mark.key {
        "," => last.is_some_and(|c| c.is_ascii_digit()) && next.is_some_and(|c| c.is_ascii_digit()),
        key => key.chars().all(|c| QUOTES.contains(c)) && (left.is_some() || right.is_some()),
    }
}

/// How many tokens of `new[typed.0..typed.1]` replace the take's `missing` words at one end of it
/// ([`follow_next`]), counted from the side next to the take's kept words (the last ones at the
/// start, the first ones at the finish). The `fixed` tokens on that side replaced the take's words
/// in the field itself and are kept, up to a mark. A run stops at a mark that does not belong to a
/// word ([`joins`]), ends in a word (or a quote touching one), has at least two characters (one
/// when `missing` has one), and at most twice `missing`'s size, or six [`units`] more than it; the
/// closest in spelling wins ([`spelled`], [`distance`]), after one that is not mixed in script
/// where `missing` is not, then one that meets the rest at a word's edge ([`word_edge`]), then the
/// closest in size, then the shortest. Marks of the take that `missing` ends (starts) with, typed
/// again next to the run, are the take's (「。」 after 「见面」). `missing` with no word (the take's
/// final 「。」 deleted) is replaced only by marks typed in its place. `Some(0)` when nothing
/// replaces it (nothing typed there, or a mark first: text typed on after the take); `None` when
/// what is there cannot be told yet (「Q」 for 千问四).
fn pick(missing: &[Tok<'_>], new: &[Tok<'_>], typed: (usize, usize), end: End, fixed: usize) -> Option<usize> {
    let n = typed.1 - typed.0;
    // The index in `new` of the `k`th token from the take's side.
    let at = |k: usize| match end {
        End::Start => typed.1 - k,
        End::Finish => typed.0 + k - 1,
    };
    let run = |k: usize| match end {
        End::Start => &new[typed.1 - k..typed.1],
        End::Finish => &new[typed.0..typed.0 + k],
    };
    let mark = |k: usize| is_punctuation(&new[at(k)]) && !joins(new, at(k));
    if words_of(missing) == 0 {
        return Some((1..=n).take_while(|&k| is_punctuation(&new[at(k)])).count());
    }
    // The take's own marks at this end (its final 「。」) are not part of the words a run replaces
    // (2026-11-28: counted in, a one-character replacement of 「会。」 was too short, and the mark
    // drew a Latin replacement's spelling away).
    let lead = missing.iter().take_while(|t| is_punctuation(t)).count();
    let trail = missing[lead..].iter().rev().take_while(|t| is_punctuation(t)).count();
    let words = &missing[lead..missing.len() - trail];
    let reach = (1..=n).find(|&k| mark(k)).map_or(n, |k| k - 1);
    let fixed = fixed.min(reach);
    let size = units(words);
    let shortest = chars_of(words).min(2);
    let target = spelled(words);
    // A run ends where its last word does: not on or before a mark that joins it to the word
    // beyond (「1,」 or 「1」 of 「1,000」, 「I'」 of 「I'm」), nor with an opening quote (「《」).
    let whole = |k: usize| {
        let far = &new[at(k)];
        let beyond = (k < n).then(|| &new[at(k + 1)]);
        let glued_on = match end {
            End::Finish => beyond.is_some_and(|b| b.word && far.text.len() == far.key.len()),
            End::Start => beyond.is_some_and(|b| b.word && b.text.len() == b.key.len()),
        };
        let far_ends = if is_punctuation(far) { joins(new, at(k)) && !glued_on } else { far.word };
        far_ends && !(k < n && is_punctuation(&new[at(k + 1)]) && joins(new, at(k + 1))) && balanced(run(k))
    };
    type Rank = (bool, std::cmp::Reverse<usize>, bool, std::cmp::Reverse<usize>, std::cmp::Reverse<usize>);
    let mut best: Option<(usize, Rank)> = None;
    for k in fixed.max(1)..=reach {
        let run = run(k);
        let units = units(run);
        if units > (2 * size).max(size + 6) {
            break;
        }
        if !whole(k) || chars_of(run) < shortest {
            continue;
        }
        let rank = (
            !mixed(run) || mixed(missing),
            std::cmp::Reverse(distance(&spelled(run), &target)),
            word_edge(&new[typed.0..typed.1], k, end),
            std::cmp::Reverse(units.abs_diff(size)),
            std::cmp::Reverse(units),
        );
        if best.as_ref().is_none_or(|(_, r)| rank > *r) {
            best = Some((k, rank));
        }
    }
    let taken = match best {
        Some((k, _)) => k,
        None if fixed > 0 => fixed,
        None if n == 0 || mark(1) => return Some(0),
        None => return None,
    };
    // The take's own marks at that end, typed again right after the run, in the order they are
    // met going away from the take. A full stop at the end of a Latin last word is in its token
    // (「boy.」).
    let marks: Vec<&str> = match end {
        End::Start => missing[..lead].iter().rev().map(|t| t.key).collect(),
        End::Finish if trail == 0 => words.last().map(|t| &t.key[t.key.trim_end_matches('.').len()..]).filter(|dots| !dots.is_empty()).into_iter().collect(),
        End::Finish => missing[missing.len() - trail..].iter().map(|t| t.key).collect(),
    };
    let again = marks.iter().enumerate().take_while(|&(m, key)| taken + m < n && new[at(taken + m + 1)].key == *key).count();
    Some(taken + again)
}

/// The share of `take`'s words that `corrected` does not keep (token LCS): 0 for the same text, 1
/// when no word is left. A follow that changes more than half of a take is a rewrite, not a
/// correction (docs/dictation.md §35.10). `None` past [`MAX_DIFF_CELLS`].
pub(crate) fn changed_share(take: &str, corrected: &str) -> Option<f64> {
    let (take, corrected) = (normalize(take), normalize(corrected));
    let (a, c) = (toks(&take), toks(&corrected));
    let words = words_of(&a);
    if words == 0 {
        return Some(0.0);
    }
    let kept = lcs_pairs(&a, &c)?.iter().filter(|&&(i, _)| a[i].word).count();
    #[allow(clippy::cast_precision_loss)] // a few hundred words at most
    Some((words - kept) as f64 / words as f64)
}

/// Characters before each token, white space included, and after the last one.
fn char_offsets(toks: &[Tok<'_>]) -> Vec<usize> {
    std::iter::once(0)
        .chain(toks.iter().scan(0usize, |at, t| {
            *at += t.text.chars().count();
            Some(*at)
        }))
        .collect()
}

/// The take's text with only the user's changes, for 自动跟踪输入框里的修改 (docs/dictation.md
/// §35.10): `take` as inserted (A), `baseline` the field's text of it in the first reading after
/// the paste (B0, with the application's own rewrites in it, such as Word's capital at the start of
/// a sentence), `current` its text now (Bt).
///
/// The application's rewrites are the runs where A and B0 differ, the user's changes the runs where
/// B0 and Bt differ (token LCS, case counts). A change that shares a token with a rewrite, or is an
/// insertion strictly inside one, is widened over it and replaces the rewrite's part of A as a
/// whole, so the user's wording wins there. A rewrite the user did not touch, even one right next
/// to a change, keeps A's wording and is not learned; an insertion right after (before) a rewrite
/// goes after (before) the rewrite's part of A. `None` past [`MAX_DIFF_CELLS`].
pub(crate) fn follow_corrected(take: &str, baseline: &str, current: &str) -> Option<String> {
    let (take, baseline, current) = (normalize(take), normalize(baseline), normalize(current));
    let (a, b0, bt) = (toks(&take), toks(&baseline), toks(&current));
    // Where each token of B0 stands in A: its own token, or the whole A part of its rewrite.
    let mut place: Vec<(usize, usize)> = vec![(0, 0); b0.len()];
    // Rewrites, as (A range, B0 range); `rewrite[j]` the one B0's token `j` is in.
    let mut rewrites: Vec<((usize, usize), (usize, usize))> = Vec::new();
    let mut rewrite: Vec<Option<usize>> = vec![None; b0.len()];
    // Whether B0's token is one of A's (not inside a rewrite).
    let mut own = vec![false; b0.len()];
    let (mut ia, mut jb) = (0usize, 0usize);
    for &(i, j) in lcs_pairs(&a, &b0)?.iter().chain(std::iter::once(&(a.len(), b0.len()))) {
        if i > ia || j > jb {
            for k in jb..j {
                place[k] = (ia, i);
                rewrite[k] = Some(rewrites.len());
            }
            rewrites.push(((ia, i), (jb, j)));
        }
        if j < b0.len() {
            place[j] = (i, i + 1);
            own[j] = true;
        }
        (ia, jb) = (i + 1, j + 1);
    }
    // The user's changes, as (B0 range, Bt range), in order.
    let mut changes: Vec<((usize, usize), (usize, usize))> = Vec::new();
    let mut kept: Vec<Option<usize>> = vec![None; b0.len()];
    let (mut jb, mut jt) = (0usize, 0usize);
    for &(j, t) in lcs_pairs(&b0, &bt)?.iter().chain(std::iter::once(&(b0.len(), bt.len()))) {
        if j > jb || t > jt {
            changes.push(((jb, j), (jt, t)));
        }
        if j < b0.len() {
            kept[j] = Some(t);
        }
        (jb, jt) = (j + 1, t + 1);
    }
    // Widen each change over the rewrites it reaches into, then merge what overlaps: groups of
    // changes over one B0 range each.
    let mut items: Vec<(usize, usize, usize)> = changes
        .iter()
        .enumerate()
        .map(|(c, &((lo, hi), _))| {
            let (mut wlo, mut whi) = (lo, hi);
            for &(_, (rlo, rhi)) in &rewrites {
                let shares = lo < hi && lo < rhi && rlo < hi;
                let inside = lo == hi && rlo < lo && lo < rhi;
                if shares || inside {
                    (wlo, whi) = (wlo.min(rlo), whi.max(rhi));
                }
            }
            (wlo, whi, c)
        })
        .collect();
    items.sort_unstable();
    let mut groups: Vec<((usize, usize), Vec<usize>)> = Vec::new();
    for (lo, hi, c) in items {
        match groups.last_mut() {
            // An insertion at a group's very start stays apart: it goes before that group.
            Some((range, members)) if lo < range.1 && (lo > range.0 || hi > lo) => {
                range.1 = range.1.max(hi);
                members.push(c);
            }
            _ => groups.push(((lo, hi), vec![c])),
        }
    }
    // A's tokens are copied outside the changes; where the user changed the white space after one
    // of them, it is the user's (the review of 2026-11-27: 「测试GPT五的能力」 corrected to
    // 「测试GPT 5的能力」 was stored as 「测试GPT5的能力」). Not after Bt's last token: a space there
    // belongs to what follows the take.
    let mut spaced: Vec<Option<String>> = vec![None; a.len()];
    for (j, t) in kept.iter().enumerate() {
        if let Some(t) = *t
            && own[j]
            && t + 1 < bt.len()
            && b0[j].text[b0[j].key.len()..] != bt[t].text[bt[t].key.len()..]
        {
            let i = place[j].0;
            spaced[i] = Some(format!("{}{}", a[i].key, &bt[t].text[bt[t].key.len()..]));
        }
    }
    let join_a = |from: usize, to: usize| (from..to).map(|i| spaced[i].as_deref().unwrap_or(a[i].text)).collect::<String>();
    let join = |toks: &[Tok<'_>]| toks.iter().map(|t| t.text).collect::<String>();
    let mut out = String::new();
    let mut at = 0usize;
    for ((lo, hi), members) in groups {
        let (alo, ahi) = if lo < hi {
            (place[lo].0, place[hi - 1].1)
        } else {
            let p = lo.checked_sub(1).map_or(0, |p| place[p].1);
            (p, p)
        };
        // The group's part of Bt: its changes' runs and the tokens the user kept inside it.
        let mut tlo = usize::MAX;
        let mut thi = 0usize;
        for &c in &members {
            let (_, (s, e)) = changes[c];
            (tlo, thi) = (tlo.min(s), thi.max(e));
        }
        for t in kept[lo..hi].iter().flatten() {
            (tlo, thi) = (tlo.min(*t), thi.max(t + 1));
        }
        if tlo > thi {
            continue;
        }
        out.push_str(&join_a(at.min(alo), alo));
        out.push_str(&join(&bt[tlo..thi]));
        at = at.max(ahi);
    }
    out.push_str(&join_a(at.min(a.len()), a.len()));
    Some(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::vocabulary::{DictionaryEntry, EntrySource};
    use crate::vocabulary::{VocabularyHit, VocabularyHits};

    /// A take whose recognised text was inserted as it was (no clean-up).
    fn plain(text: &str, at_ms: u64) -> HistoryText {
        HistoryText {
            id: Uuid::new_v4(),
            at_ms,
            raw_text: text.into(),
            text: text.into(),
            refined: false,
            vocabulary: VocabularyHits::default(),
            corrected: None,
            corrected_via: None,
        }
    }

    fn polished(raw: &str, text: &str, at_ms: u64) -> HistoryText {
        HistoryText { raw_text: raw.into(), refined: true, ..plain(text, at_ms) }
    }

    fn pairs(take: &HistoryText, corrected: &str) -> Vec<(String, String)> {
        let replay = Replay::of(take, &[], &[]).unwrap();
        edit_changes(take, corrected, &replay).into_iter().map(|c| (c.heard, c.right)).collect()
    }

    fn one(selection: &str, takes: &[HistoryText]) -> Matched {
        let found = match_selection(selection, takes);
        assert_eq!(found.len(), 1, "{found:?}");
        found.into_iter().next().unwrap()
    }

    #[test]
    fn a_corrected_sentence_finds_its_take_and_the_users_wording() {
        let take = plain("我在用 Quinn for max 做测试", 10);
        let found = one("我在用 Qwen 4 max 做测试", std::slice::from_ref(&take));
        assert_eq!(found.corrected, "我在用 Qwen 4 max 做测试");
        assert!((found.similarity - 11.0 / 17.0).abs() < 1e-9, "{}", found.similarity);
        assert_eq!(pairs(&take, &found.corrected), [("Quinn for".into(), "Qwen 4".into())]);
        assert!(storable(&take, &found.corrected));
    }

    /// The tester's case (2026-11-18): the whole take was the wrong words but one.
    #[test]
    fn a_take_that_is_mostly_the_error_still_matches() {
        let take = plain("Quinn for max", 10);
        let found = one("Qwen 4 max", std::slice::from_ref(&take));
        assert!((found.similarity - 5.0 / 11.0).abs() < 1e-9, "{}", found.similarity);
        assert_eq!(found.corrected, "Qwen 4 max");
        assert_eq!(pairs(&take, &found.corrected), [("Quinn for".into(), "Qwen 4".into())]);
    }

    #[test]
    fn text_selected_around_the_take_is_left_out() {
        let take = plain("Quinn for max is great", 10);
        assert_eq!(one("Hi, Qwen 4 max is great", std::slice::from_ref(&take)).corrected, "Qwen 4 max is great");
        let take = plain("我们用非书开会", 10);
        let found = one("大家好，我们用飞书开会", std::slice::from_ref(&take));
        assert_eq!(found.corrected, "我们用飞书开会");
        assert_eq!(pairs(&take, &found.corrected), [("非".into(), "飞".into())], "widened by the suggestions, not here");
        let take = plain("先说一句。第二句 Quinn for max。最后一句。", 10);
        assert_eq!(
            one("第二句 Qwen 4 max。", std::slice::from_ref(&take)).corrected,
            "先说一句。第二句 Qwen 4 max。最后一句。",
            "only the selected sentence changes"
        );
    }

    /// Regression (scripts/smoke-desktop-linux.sh, 2026-11-18: history.sqlite3 kept
    /// `corrected.text` 「Voltip 冒烟测」 for the selection 「Voltip 冒烟测试」): at the end of the
    /// take no run of the selection has a character in common with 「注入」, and the shortest run won
    /// the tie, so 「试」 counted as text selected besides the take. A run as many words long as the
    /// take's wins such a tie.
    #[test]
    fn regression_an_end_replacement_with_nothing_in_common_keeps_every_character() {
        let take = plain("Voltip 冒烟注入", 10);
        let found = one("Voltip 冒烟测试", std::slice::from_ref(&take));
        assert_eq!(found.corrected, "Voltip 冒烟测试");
        assert_eq!(pairs(&take, &found.corrected), [("注入".into(), "测试".into())]);
        let take = plain("注入成功", 10);
        assert_eq!(one("测试成功", std::slice::from_ref(&take)).corrected, "测试成功", "at the start too");
    }

    /// Regression (goal review of PR #260, 2026-11-18): a take corrected before is compared with
    /// its corrected text and its inserted text, the higher similarity counting (§35.4). Only the
    /// corrected text was searched, so selecting the original wording again to take a correction
    /// back found nothing when the correction had replaced every word.
    #[test]
    fn regression_the_original_wording_takes_back_a_correction_that_shares_no_word_with_it() {
        let take = HistoryText { corrected: Some("Qwen 4".into()), ..plain("Quinn for", 10) };
        let found = one("Quinn for", std::slice::from_ref(&take));
        assert_eq!(found.corrected, "Quinn for");
        assert!((found.similarity - 1.0).abs() < 1e-9, "{}", found.similarity);
        assert!(!storable(&take, &found.corrected), "nothing left to keep: the correction is cleared");
        // Sharing a word with the corrected text, the correction stays the base (§35.4).
        let take = HistoryText { corrected: Some("我在用 Qwen 4 max 做测试".into()), ..plain("我在用 Quinn for max 做测试", 10) };
        assert_eq!(one("用 Qwen 4 max 做实验", std::slice::from_ref(&take)).corrected, "我在用 Qwen 4 max 做实验");
    }

    /// Regression (goal review of PR #260, round 2): both texts of a corrected take align, the
    /// inserted one better — the original wording selected again — and the corrected text was
    /// rebuilt on the corrected alignment all the same: 「red apple」 matched the corrected text's
    /// first 「red」 only, 「apple」 counted as selected besides the take, and the earlier
    /// 「grape」 stayed. The winning alignment decides now, mapped onto the corrected text, so an
    /// earlier correction outside the selection (「plum」) stays too.
    #[test]
    fn regression_the_alignment_that_wins_rebuilds_the_take() {
        let take = HistoryText { corrected: Some("red grape red pear".into()), ..plain("red apple red pear", 10) };
        assert_eq!(one("red apple", std::slice::from_ref(&take)).corrected, "red apple red pear", "taken back");
        let take = HistoryText { corrected: Some("red grape red plum".into()), ..plain("red apple red pear", 10) };
        assert_eq!(one("red apple", std::slice::from_ref(&take)).corrected, "red apple red plum", "the other correction stays");
        let take = HistoryText { corrected: Some("我们下午四点在飞书开会".into()), ..plain("我们下午三点在飞书开会", 10) };
        assert_eq!(one("下午五点在飞书", std::slice::from_ref(&take)).corrected, "我们下午五点在飞书开会");
    }

    /// Regression (found while fixing the round-2 item, 2026-11-18): a selection that stops right
    /// before the take's next word has no trailing space, and the take's space there was lost with
    /// the replaced words (「Qwen 4 maxis great」).
    #[test]
    fn regression_a_selection_that_stops_before_the_next_word_keeps_the_space() {
        let take = plain("Quinn for max is great", 10);
        assert_eq!(one("Qwen 4 max", std::slice::from_ref(&take)).corrected, "Qwen 4 max is great");
    }

    /// docs/dictation.md §35. Regression, 2026-11-25 (the user on windows-local): 「今天测试一下千问四Max的推理能力。」
    /// corrected to 「…Qwen 4 Max…」; with only 「Qwen 4」 selected the pill said 「最近50次听写里没有找到这段文字」:
    /// the selection shares no word with the take. Read with its paragraph it finds the take, also
    /// with the change at the start of the selection (which alone matched but learned nothing) and
    /// with nothing selected, the caret in the sentence or at its end.
    #[test]
    fn regression_a_corrected_word_selected_alone_is_learned_from_its_paragraph() {
        let inserted = "今天测试一下千问四Max的推理能力。";
        let fixed = "今天测试一下Qwen 4 Max的推理能力。";
        for take in [plain(inserted, 10), polished("今天测试一下千问四 max 的推理能力", inserted, 10)] {
            let takes = std::slice::from_ref(&take);
            assert!(match_selection("Qwen 4", takes).is_empty(), "alone it is not found");
            for (before, selected, after) in [
                ("今天测试一下", "Qwen 4", " Max的推理能力。"),
                ("今天测试一下", "Qwen 4 Max", "的推理能力。"),
                (fixed, "", ""),
                ("今天测试一下Qwen", "", " 4 Max的推理能力。"),
            ] {
                let found = match_around(before, selected, after, takes);
                assert_eq!(found.len(), 1, "{before:?} {selected:?} {after:?}: {found:?}");
                assert_eq!(found[0].corrected, fixed);
                assert_eq!(pairs(&take, &found[0].corrected), [("千问四".to_owned(), "Qwen 4".to_owned())], "{selected:?}");
            }
        }
    }

    /// Two takes in one paragraph: only the one at the selection, or at the caret, is learned.
    #[test]
    fn only_the_take_at_the_selection_or_the_caret_is_learned_from_a_paragraph() {
        let takes = [plain("我们下午三点在飞书开会", 20), plain("今天测试一下千问四Max的推理能力。", 10)];
        let found = match_around("今天测试一下", "Qwen 4", " Max的推理能力。我们下午四点在飞书开会", &takes);
        let got: Vec<(usize, &str)> = found.iter().map(|m| (m.index, m.corrected.as_str())).collect();
        assert_eq!(got, [(1, "今天测试一下Qwen 4 Max的推理能力。")]);
        let found = match_around("今天测试一下Qwen 4 Max的推理能力。我们下午", "", "四点在飞书开会", &takes);
        let got: Vec<(usize, &str)> = found.iter().map(|m| (m.index, m.corrected.as_str())).collect();
        assert_eq!(got, [(0, "我们下午四点在飞书开会")], "the caret is in the second take");
    }

    /// The review of 2026-11-25: the takes were chosen over the whole paragraph, at most five, and
    /// only then kept to the selection, so unchanged takes in the paragraph (alike 1.0) took the five
    /// places and the corrected one at the cursor was not found.
    #[test]
    fn regression_unchanged_takes_around_do_not_crowd_out_the_one_at_the_cursor() {
        let lines = ["春眠不觉晓。", "处处闻啼鸟。", "夜来风雨声。", "今天测试一下千问四Max的推理能力。", "花落知多少。", "白日依山尽。"];
        let takes: Vec<HistoryText> = lines.iter().enumerate().map(|(i, t)| plain(t, 100 - i as u64)).collect();
        let found = match_around("春眠不觉晓。处处闻啼鸟。夜来风雨声。今天测试一下", "Qwen 4", " Max的推理能力。花落知多少。白日依山尽。", &takes);
        let got: Vec<(usize, &str)> = found.iter().map(|m| (m.index, m.corrected.as_str())).collect();
        assert_eq!(got, [(3, "今天测试一下Qwen 4 Max的推理能力。")]);
        let caret = match_around("春眠不觉晓。处处闻啼鸟。夜来风雨声。今天测试一下Qwen 4", "", " Max的推理能力。花落知多少。白日依山尽。", &takes);
        assert_eq!(caret.iter().map(|m| m.index).collect::<Vec<_>>(), [3], "the caret alone");
        // A selection over three of seven takes, two of them corrected: both are learned.
        let seven = ["一。", "二。", "第三句写的是 GitHab 上的项目。", "第四句没有改。", "第五句说 Quinn for max 很好用。", "六。", "七。"];
        let takes: Vec<HistoryText> = seven.iter().enumerate().map(|(i, t)| plain(t, 100 - i as u64)).collect();
        let found = match_around("一。二。", "第三句写的是 GitHub 上的项目。第四句没有改。第五句说 Qwen 4 max 很好用。", "六。七。", &takes);
        let got: Vec<(usize, &str)> = found.iter().filter(|m| m.corrected != takes[m.index].text).map(|m| (m.index, m.corrected.as_str())).collect();
        assert_eq!(got, [(2, "第三句写的是 GitHub 上的项目。"), (4, "第五句说 Qwen 4 max 很好用。")]);
    }

    /// The review of 2026-11-25: a take whose first words are the change, glued to text before it
    /// with no space or mark, took a character of that text into the change (「千问四 → 下Qwen 4」):
    /// with a selection, the change comes from the selection only.
    #[test]
    fn regression_text_glued_before_the_take_is_not_taken_into_the_change() {
        let take = plain("千问四Max的推理能力", 10);
        let takes = std::slice::from_ref(&take);
        for (selected, after) in [("Qwen 4", " Max的推理能力"), ("Qwen 4 Max的推理能力", "")] {
            let found = match_around("我们讨论一下", selected, after, takes);
            assert_eq!(found.len(), 1, "{selected:?}: {found:?}");
            assert_eq!(found[0].corrected, "Qwen 4 Max的推理能力", "{selected:?}");
            assert_eq!(pairs(&take, &found[0].corrected), [("千问四".to_owned(), "Qwen 4".to_owned())], "{selected:?}");
        }
    }

    /// The same field with nothing selected: the caret after the corrected words, or at the end.
    #[test]
    fn regression_text_glued_before_the_take_with_only_the_caret_is_not_taken_into_the_change() {
        let take = plain("千问四Max的推理能力", 10);
        let takes = std::slice::from_ref(&take);
        for (before, after) in [("我们讨论一下Qwen 4", " Max的推理能力"), ("我们讨论一下Qwen 4 Max的推理能力", "")] {
            let found = match_around(before, "", after, takes);
            assert_eq!(found.len(), 1, "{before:?}: {found:?}");
            assert_eq!(found[0].corrected, "Qwen 4 Max的推理能力", "{before:?}");
            assert_eq!(pairs(&take, &found[0].corrected), [("千问四".to_owned(), "Qwen 4".to_owned())], "{before:?}");
        }
    }

    /// A match tells which characters of the selection its take covers.
    #[test]
    fn a_match_names_the_characters_of_the_selection_its_take_covers() {
        let takes = [plain("第二次说的是 GitHab 上的项目", 20), plain("第一次说 Quinn for max 很好用", 10)];
        let selection = "第一次说 Qwen 4 max 很好用第二次说的是 GitHub 上的项目";
        let found = match_selection(selection, &takes);
        let chars: Vec<char> = selection.chars().collect();
        let covered: Vec<(usize, String)> = found.iter().map(|m| (m.index, chars[m.span.0..m.span.1].iter().collect())).collect();
        assert_eq!(covered, [(0, "第二次说的是 GitHub 上的项目".to_owned()), (1, "第一次说 Qwen 4 max 很好用".to_owned())]);
    }

    #[test]
    fn a_selection_over_two_takes_corrects_each() {
        let takes = [plain("第二次说的是 GitHab 上的项目", 20), plain("第一次说 Quinn for max 很好用", 10)];
        let found = match_selection("第一次说 Qwen 4 max 很好用第二次说的是 GitHub 上的项目", &takes);
        let got: Vec<(usize, &str)> = found.iter().map(|m| (m.index, m.corrected.as_str())).collect();
        assert_eq!(got, [(0, "第二次说的是 GitHub 上的项目"), (1, "第一次说 Qwen 4 max 很好用")]);
    }

    #[test]
    fn nothing_alike_matches_nothing_and_the_same_text_is_nothing_to_keep() {
        let take = plain("Quinn for max", 10);
        assert!(match_selection("今天天气不错", std::slice::from_ref(&take)).is_empty());
        let same = one("Quinn for max", std::slice::from_ref(&take));
        assert_eq!(same.corrected, "Quinn for max");
        assert!(!storable(&take, &same.corrected));
    }

    #[test]
    fn the_newer_take_wins_a_tie() {
        let takes = [plain("Quinn for max", 20), plain("Quinn for max", 10)];
        assert_eq!(one("Qwen 4 max", &takes).index, 0);
    }

    #[test]
    fn a_change_the_clean_up_made_is_not_learned_but_a_case_change_is_no_obstacle() {
        let cased = polished("quinn for max", "Quinn for max", 10);
        assert_eq!(pairs(&cased, "Qwen 4 max"), [("quinn for".into(), "Qwen 4".into())], "the recogniser's wording");
        let polished_word = polished("我们用维他命开会", "我们用 Voltip 开会。", 10);
        assert!(pairs(&polished_word, "我们用 VoltIP 开会。").is_empty(), "the clean-up wrote Voltip");
        assert!(!storable(&polished_word, "我们用 VoltIP 开会。"));
    }

    #[test]
    fn a_change_the_dictionary_made_is_not_learned() {
        let entry = DictionaryEntry {
            id: Uuid::new_v4(),
            term: "GitHub".into(),
            heard_as: vec!["github".into()],
            enabled: true,
            source: EntrySource::Manual,
            created_at_ms: 1,
            updated_at_ms: 1,
        };
        let take = HistoryText {
            raw_text: "代码在 github 上".into(),
            text: "代码在 GitHub 上".into(),
            vocabulary: VocabularyHits { corrections: vec![VocabularyHit { id: entry.id, count: 1 }], rules: Vec::new() },
            ..plain("", 10)
        };
        let replay = Replay::of(&take, std::slice::from_ref(&entry), &[]).unwrap();
        assert!(edit_changes(&take, "代码在 GitLab 上", &replay).is_empty(), "the dictionary wrote GitHub there");
        assert!(storable(&take, "代码在 GitLab 上"), "kept: whether it is learned is the suggestions' call");
        assert!(Replay::of(&take, &[], &[]).is_none(), "the entry deleted since: nothing to tell apart");
    }

    #[test]
    fn a_second_correction_keeps_the_first_and_a_revert_leaves_nothing() {
        let mut take = plain("Quinn for max 和 kubernetes 都要装", 10);
        let first = one("Qwen 4 max 和 kubernetes 都要装", std::slice::from_ref(&take));
        take.corrected = Some(first.corrected.clone());
        let second = one("kubernetes 都要装", std::slice::from_ref(&take));
        assert_eq!(second.corrected, "Qwen 4 max 和 kubernetes 都要装", "the part not selected keeps the first correction");
        let third = one("Qwen 4 max 和 Kubernetes 都要装", std::slice::from_ref(&take));
        assert_eq!(third.corrected, "Qwen 4 max 和 Kubernetes 都要装");
        assert_eq!(pairs(&take, &third.corrected), [("Quinn for".into(), "Qwen 4".into()), ("kubernetes".into(), "Kubernetes".into())], "both corrections");
        let reverted = one("Quinn for max 和 kubernetes 都要装", std::slice::from_ref(&take));
        assert_eq!(reverted.corrected, take.text);
        assert!(!storable(&take, &reverted.corrected));
    }

    #[test]
    fn white_space_the_application_adds_at_the_end_does_not_matter() {
        let take = plain("Quinn for max", 10);
        assert_eq!(one("Qwen 4 max \n", std::slice::from_ref(&take)).corrected, "Qwen 4 max \n");
    }

    #[test]
    fn line_endings_of_the_application_do_not_matter() {
        let take = plain("第一行 Quinn for max\n第二行", 10);
        assert_eq!(one("第一行 Qwen 4 max\r\n第二行", std::slice::from_ref(&take)).corrected, "第一行 Qwen 4 max\n第二行");
    }

    /// docs/dictation.md §35.10: the user's change is kept, the field's text of the take as is.
    #[test]
    fn a_followed_change_with_no_rewrite_of_the_application_is_the_field_text() {
        let a = "今天测试一下千问四Max的推理能力";
        assert_eq!(follow_corrected(a, a, "今天测试一下Qwen 4 Max的推理能力").as_deref(), Some("今天测试一下Qwen 4 Max的推理能力"));
        assert_eq!(follow_corrected(a, a, a).as_deref(), Some(a), "nothing changed");
    }

    /// A rewrite of the application the user changed again: the user's wording wins over the whole
    /// rewrite. Regression of the plan's review (2026-11-26): Word's capital at the start, then the
    /// user's correction of that word.
    #[test]
    fn regression_a_change_over_a_rewrite_of_the_application_takes_the_users_wording() {
        assert_eq!(follow_corrected("quinn for max", "Quinn for max", "Qwen 4 max").as_deref(), Some("Qwen 4 max"));
        let take = plain("quinn for max", 10);
        assert_eq!(pairs(&take, "Qwen 4 max"), [("quinn for".to_owned(), "Qwen 4".to_owned())]);
    }

    /// A rewrite the user only changed next to keeps the take's wording, and is not learned.
    /// Regression of the plan's review (round 2, 2026-11-26): widening over adjacent rewrites
    /// copied Word's capital into the correction.
    #[test]
    fn regression_a_rewrite_next_to_a_change_keeps_the_takes_wording() {
        assert_eq!(follow_corrected("quinn foo", "Quinn foo", "Quinn bar").as_deref(), Some("quinn bar"));
        assert_eq!(follow_corrected("quinn for max", "Quinn for max", "Quinn for Max").as_deref(), Some("quinn for Max"));
    }

    /// Regression (review of #271, 2026-11-27): the merge copied the take's own tokens outside the
    /// user's changes, so a space typed after a kept Latin word was lost: the field read
    /// 「测试GPT 5的能力」, 「测试GPT5的能力」 was stored and 「GPT五 → GPT5」 suggested.
    #[test]
    fn regression_a_space_typed_after_a_kept_latin_word_is_kept() {
        for (take, field) in
            [("测试GPT五的能力", "测试GPT 5的能力"), ("测试一下Max的能力", "测试一下Max Pro的能力"), ("新买的iPhone很好用", "新买的iPhone 16很好用")]
        {
            assert_eq!(follow_corrected(take, take, field).as_deref(), Some(field), "{take}");
        }
        assert_eq!(pairs(&plain("测试GPT五的能力", 10), "测试GPT 5的能力"), [("GPT五".to_owned(), "GPT 5".to_owned())]);
        // A space the application added is its own: the take's spacing stays.
        assert_eq!(follow_corrected("测试GPT五的能力", "测试GPT 五的能力", "测试GPT 5的能力").as_deref(), Some("测试GPT5的能力"));
    }

    #[test]
    fn an_insertion_next_to_a_rewrite_goes_beside_the_takes_part_of_it() {
        assert_eq!(follow_corrected("quinn foo", "Quinn foo", "Quinn new foo").as_deref(), Some("quinn new foo"));
        assert_eq!(follow_corrected("foo quinn", "foo Quinn", "foo new Quinn").as_deref(), Some("foo new quinn"));
        // Strictly inside a rewrite of two words: the user's wording of the whole rewrite.
        assert_eq!(follow_corrected("quinn for max", "Quinn For max", "Quinn new For max").as_deref(), Some("Quinn new For max"));
    }

    fn read(next: FollowNext) -> FollowSpot {
        match next {
            FollowNext::Read(spot) => spot,
            other => panic!("not read: {other:?}"),
        }
    }

    /// The take in the first reading after its paste, then carried through the user's edits: the
    /// change at the take's start is taken from the take's span, not from the text glued before it.
    #[test]
    fn a_followed_take_is_found_after_its_paste_and_carried_through_the_edits() {
        let take = plain("千问四Max的推理能力", 10);
        let first = follow_first("我们讨论一下千问四Max的推理能力", "", "", &take).expect("found");
        let baseline = first.text();
        assert_eq!(baseline, "千问四Max的推理能力");
        // Corrected in place, the caret after the corrected words.
        let next = read(follow_next(&first, "我们讨论一下Qwen 4", "", " Max的推理能力", &baseline));
        assert_eq!(next.text(), "Qwen 4 Max的推理能力");
        assert_eq!(follow_corrected(&take.text, &baseline, &next.text()).as_deref(), Some("Qwen 4 Max的推理能力"));
        assert_eq!(pairs(&take, "Qwen 4 Max的推理能力"), [("千问四".to_owned(), "Qwen 4".to_owned())]);
        // Words typed after the take are not the take's.
        let typed = read(follow_next(&next, "我们讨论一下Qwen 4 Max的推理能力，你觉得呢", "", "", &baseline));
        assert_eq!(typed.text(), "Qwen 4 Max的推理能力");
        // Sent: the field is empty.
        assert_eq!(follow_next(&typed, "", "", "", &baseline), FollowNext::Gone);
    }

    /// The first word deleted, then typed again key by key: the half-typed reading is not read
    /// (the take's place is carried on), and the typed word replaces the deleted one; the text
    /// before the take that was there at its paste is never taken for it (「我们讨论一下」).
    #[test]
    fn a_deleted_first_word_typed_again_is_read_once_it_can_be_told() {
        let take = plain("千问四的推理能力", 10);
        let first = follow_first("我们讨论一下千问四的推理能力", "", "", &take).expect("found");
        let baseline = first.text();
        assert_eq!(baseline, "千问四的推理能力");
        let deleted = read(follow_next(&first, "我们讨论一下", "", "的推理能力", &baseline));
        assert_eq!(deleted.text(), "的推理能力");
        let FollowNext::Typing(half) = follow_next(&deleted, "我们讨论一下Q", "", "的推理能力", &baseline) else {
            panic!("a word half typed is not read")
        };
        let again = read(follow_next(&half, "我们讨论一下Qwen 4", "", "的推理能力", &baseline));
        assert_eq!(again.text(), "Qwen 4的推理能力");
    }

    /// The first reading takes the take where the insert check read it, right before the caret:
    /// text before it that ends as the take starts is not part of it (2026-11-28: 「好的，」 before
    /// a take starting with 「的」 was matched as 「的，的服务…」).
    #[test]
    fn regression_the_first_reading_does_not_take_text_before_the_take_that_ends_as_it_starts() {
        let take = plain("的服务部署代码模型飞书我们测试", 10);
        let first = follow_first("好的，的服务部署代码模型飞书我们测试", "", "，谢谢", &take).expect("found");
        assert_eq!(first.text(), "的服务部署代码模型飞书我们测试");
        // Rewritten by the application (Word's capital): matched as before, the rewrite in it.
        let first = follow_first("好的，Quinn for max", "", "", &plain("quinn for max", 10)).expect("found");
        assert_eq!(first.text(), "Quinn for max");
    }

    #[test]
    fn a_followed_take_that_is_not_in_the_first_reading_is_not_found() {
        let take = plain("千问四Max的推理能力", 10);
        assert_eq!(follow_first("完全不同的一句话", "", "", &take), None);
    }
}
