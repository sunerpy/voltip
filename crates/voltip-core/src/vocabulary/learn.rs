//! 从修改中学习 (docs/dictation.md §35): the user corrected a take in the application it went to,
//! selected the corrected text and asked Voltip to learn from it. [`match_selection`] finds the
//! takes the selection is the corrected form of and builds each one's whole corrected text;
//! [`storable`] says whether a correction holds anything to learn; [`edit_changes`] is what the
//! learning suggestions take from it: the replacements of the inserted text whose wording the
//! recogniser wrote and that neither the AI clean-up, the dictionary nor a rule touched — the
//! errors the clean-up's own changes cannot show (§16.10). Nothing here calls an AI service.

use std::collections::HashSet;

use crate::dictation::insert_check::normalize;
use crate::history::HistoryText;
use crate::mcp::candidates::{MAX_DIFF_CELLS, MAX_SCAN_CELLS, Replacement, Replay, SegmentKind, diff_segments_by, diff_tokens, replacements};

/// Recognised takes a selection is compared with, newest first.
pub const LEARN_TAKES: usize = 50;
/// The longest selection learned from, in characters.
pub const MAX_LEARN_SELECTION_CHARS: usize = 8000;
/// The least similarity at which a selection counts as the corrected form of a take.
pub const MATCH_MIN: f64 = 0.4;
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

/// The tokens of the selection's run `extra` that replace the take's run `missing` at one end of
/// the match (the error was the take's first or last words): whole tokens next to the matched part,
/// starting (at the start) or ending (at the finish) with a word, within one word of `missing`'s
/// count and half to twice its characters, the most similar character by character; on a tie the
/// one closest to `missing`'s count of words, then the shortest. The rest of `extra` is text the
/// user selected besides the take. `None` when no run fits. The answer is how many tokens of
/// `extra` are taken, and their character LCS with `missing`.
fn edge(missing: &[Tok<'_>], extra: &[Tok<'_>], end: End) -> Option<(usize, usize)> {
    let (words, chars) = (words_of(missing), chars_of(missing));
    if words == 0 || words_of(extra) == 0 || chars > INNER_MAX {
        return None;
    }
    let target = folded(missing);
    let mut best: Option<(usize, usize, usize, usize)> = None; // (taken, score, word difference, chars)
    for taken in 1..=extra.len() {
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
        let apart = run_words.abs_diff(words);
        if best.is_none_or(|(_, s, a, c)| (score, std::cmp::Reverse(apart), std::cmp::Reverse(run_chars)) > (s, std::cmp::Reverse(a), std::cmp::Reverse(c))) {
            best = Some((taken, score, apart, run_chars));
        }
    }
    best.map(|(taken, score, _, _)| (taken, score))
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

/// The alignment of `take` and `selection`, or `None` when they share no word.
fn align(take: &[Tok<'_>], selection: &[Tok<'_>]) -> Option<Alignment> {
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
    let (take_start, selection_start) = match edge(&take[..first.0], &selection[..first.1], End::Start) {
        Some((taken, score)) => {
            matched += score;
            (0, first.1 - taken)
        }
        None => (first.0, first.1),
    };
    let (take_end, selection_end) = match edge(&take[last.0 + 1..], &selection[last.1 + 1..], End::Finish) {
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
    let selection = normalize(selection);
    let sel = toks(&selection);
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
        let on_corrected = corrected.as_deref().and_then(|t| align(t, &sel));
        let on_inserted = inserted.as_deref().and_then(|t| align(t, &sel));
        let similarity = on_corrected.iter().chain(on_inserted.iter()).map(|a| a.similarity).fold(f64::NAN, f64::max);
        let Some((alignment, from_inserted)) = on_corrected.map(|a| (a, false)).or(on_inserted.map(|a| (a, true))) else { continue };
        if similarity >= MATCH_MIN {
            candidates.push(Candidate { index, alignment, on_inserted: from_inserted, similarity });
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
            let base = toks(if c.on_inserted { &take.text } else { take.corrected.as_deref().unwrap_or(&take.text) });
            let join = |toks: &[Tok<'_>]| toks.iter().map(|t| t.text).collect::<String>();
            let (a, b) = c.alignment.take;
            let (s, e) = c.alignment.selection;
            let corrected = format!("{}{}{}", join(&base[..a]), join(&sel[s..e]), join(&base[b..]));
            Matched { index: c.index, corrected, similarity: c.similarity }
        })
        .collect()
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
}
