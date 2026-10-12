//! `vocabulary_backtest` (docs/dictation.md §16.9): what a dictionary entry or a replacement rule
//! that is not saved yet would change in the recent history. The history page's quick-fix card
//! and the MCP service show it before anything is written, with the same matchers the pipeline
//! runs: a draft entry corrects each take's recognised text, a draft rule rewrites its final text.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::matcher::LiteralMatcher;
use super::{
    DictionaryDraft, DictionaryEntry, EntrySource, ReplacementRule, RuleDraft, Vocabulary, VocabularyError, validate_dictionary_draft, validate_rule_draft,
};
use crate::history::HistoryText;

/// How many recent takes a backtest reads unless asked otherwise.
pub const BACKTEST_LIMIT: usize = 500;
/// The most recent takes one backtest may read.
pub const MAX_BACKTEST_LIMIT: usize = crate::history::MAX_RECENT_TEXTS;
/// Examples a backtest returns at most.
pub const MAX_BACKTEST_EXAMPLES: usize = 3;
/// Characters kept on each side of the first change in an example.
pub const BACKTEST_SNIPPET_CHARS: usize = 12;

/// The draft a backtest tries: a dictionary entry (`id` = the entry it would replace, `None` for a
/// new one) or a replacement rule (likewise).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BacktestDraft {
    /// A dictionary entry.
    Dictionary {
        /// The entry it stands in for.
        #[serde(default)]
        id: Option<Uuid>,
        /// The draft entry.
        entry: DictionaryDraft,
    },
    /// A replacement rule.
    Rule {
        /// The rule it stands in for.
        #[serde(default)]
        id: Option<Uuid>,
        /// The draft rule.
        rule: RuleDraft,
    },
}

/// One take the draft would change: the text around the first change, before and after.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BacktestExample {
    /// The history entry.
    pub history_id: Uuid,
    /// When it was recorded.
    pub at_ms: u64,
    /// Up to [`BACKTEST_SNIPPET_CHARS`] characters on each side of the change, before the draft
    /// (`…` marks a cut).
    pub before: String,
    /// The same stretch with the draft applied.
    pub after: String,
    /// The final text kept a form the draft replaces (dictionary drafts only).
    pub kept: bool,
}

/// What a backtest answers.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Backtest {
    /// Takes looked at (the excluded one not counted).
    pub scanned: u32,
    /// Takes the draft would change.
    pub matched: u32,
    /// Dictionary drafts: of `matched`, the takes whose final text still contains a form the draft
    /// replaces — the clean-up kept it, so it may be right there (docs/dictation.md §16.9).
    pub kept: u32,
    /// Up to [`MAX_BACKTEST_EXAMPLES`] of the changed takes, those with `kept` first, newest first.
    pub examples: Vec<BacktestExample>,
}

/// Run `draft` over `history` (newest first, as `HistoryReader::recent_texts` reads it), leaving
/// out `exclude` (the entry the card was opened on). A dictionary draft is compared with the
/// current `dictionary`; a rule draft runs alone on each final text, which the saved rules have
/// already rewritten. The draft is validated on its own; the checks against the other entries are
/// the save's. A text over [`super::MAX_TEXT_BYTES`] is passed through, as the pipeline would.
pub fn backtest(dictionary: &[DictionaryEntry], history: &[HistoryText], draft: &BacktestDraft, exclude: Option<Uuid>) -> Result<Backtest, VocabularyError> {
    let takes = history.iter().filter(|t| Some(t.id) != exclude);
    match draft {
        BacktestDraft::Dictionary { id, entry } => {
            let entry = validate_dictionary_draft(entry)?;
            let base = Vocabulary::compile(dictionary, &[]);
            let next = Vocabulary::compile(&with_entry(dictionary, *id, &entry), &[]);
            let forms: Vec<(String, usize)> = entry.heard_as.iter().map(|h| (h.clone(), 0)).collect();
            let kept_matcher = if forms.is_empty() { None } else { LiteralMatcher::new(&forms, true).ok() };
            Ok(collect(takes, |take| {
                let before = base.correct(&take.raw_text).text;
                let after = next.correct(&take.raw_text).text;
                (before != after).then(|| {
                    let kept = kept_matcher.as_ref().is_some_and(|m| m.find_all(&take.text).is_ok_and(|found| !found.is_empty()));
                    (before, after, kept)
                })
            }))
        }
        BacktestDraft::Rule { id, rule } => {
            let rule = validate_rule_draft(rule)?;
            let draft_rule = ReplacementRule {
                id: id.unwrap_or_else(Uuid::nil),
                name: rule.name.clone(),
                kind: rule.kind,
                pattern: rule.pattern.clone(),
                replacement: rule.replacement.clone(),
                case_sensitive: rule.case_sensitive,
                enabled: true,
                created_at_ms: 0,
                updated_at_ms: 0,
            };
            let only = Vocabulary::compile(&[], std::slice::from_ref(&draft_rule));
            Ok(collect(takes, |take| {
                let after = only.apply_rules(&take.text).text;
                (after != take.text).then(|| (take.text.clone(), after, false))
            }))
        }
    }
}

/// The dictionary as if `entry` were saved: in place of entry `id`, or appended (enabled as drafted).
fn with_entry(dictionary: &[DictionaryEntry], id: Option<Uuid>, entry: &DictionaryDraft) -> Vec<DictionaryEntry> {
    let mut list = dictionary.to_vec();
    let as_entry = |id: Uuid, source: EntrySource| DictionaryEntry {
        id,
        term: entry.term.clone(),
        heard_as: entry.heard_as.clone(),
        enabled: entry.enabled,
        source,
        created_at_ms: 0,
        updated_at_ms: 0,
    };
    match id.and_then(|id| list.iter().position(|e| e.id == id)) {
        Some(i) => list[i] = as_entry(list[i].id, list[i].source.clone()),
        None => list.push(as_entry(id.unwrap_or_else(Uuid::nil), EntrySource::Manual)),
    }
    list
}

/// Count the takes `change` reports as changed — `(before, after, kept)` — and keep the examples.
fn collect<'a>(takes: impl Iterator<Item = &'a HistoryText>, mut change: impl FnMut(&HistoryText) -> Option<(String, String, bool)>) -> Backtest {
    let mut out = Backtest::default();
    let mut kept_examples = Vec::new();
    let mut other_examples = Vec::new();
    for take in takes {
        out.scanned = out.scanned.saturating_add(1);
        let Some((before, after, kept)) = change(take) else { continue };
        out.matched = out.matched.saturating_add(1);
        if kept {
            out.kept = out.kept.saturating_add(1);
        }
        let bucket = if kept { &mut kept_examples } else { &mut other_examples };
        if bucket.len() < MAX_BACKTEST_EXAMPLES {
            let (before, after) = snippets(&before, &after);
            bucket.push(BacktestExample { history_id: take.id, at_ms: take.at_ms, before, after, kept });
        }
    }
    out.examples = kept_examples.into_iter().chain(other_examples).take(MAX_BACKTEST_EXAMPLES).collect();
    out
}

/// The stretch of `before` and `after` around their first difference: the differing middle plus up
/// to [`BACKTEST_SNIPPET_CHARS`] characters on each side, `…` where the text goes on.
fn snippets(before: &str, after: &str) -> (String, String) {
    let b: Vec<char> = before.chars().collect();
    let a: Vec<char> = after.chars().collect();
    let prefix = b.iter().zip(&a).take_while(|(x, y)| x == y).count();
    let suffix = b[prefix..].iter().rev().zip(a[prefix..].iter().rev()).take_while(|(x, y)| x == y).count();
    let start = prefix.saturating_sub(BACKTEST_SNIPPET_CHARS);
    let cut = |chars: &[char]| {
        let end = (chars.len() - suffix + BACKTEST_SNIPPET_CHARS).min(chars.len());
        let mut s = String::new();
        if start > 0 {
            s.push('…');
        }
        s.extend(&chars[start..end]);
        if end < chars.len() {
            s.push('…');
        }
        s
    };
    (cut(&b), cut(&a))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vocabulary::RuleKind;

    fn take(raw: &str, text: &str, at_ms: u64) -> HistoryText {
        HistoryText { id: Uuid::new_v4(), at_ms, raw_text: raw.into(), text: text.into() }
    }

    fn entry(term: &str, heard: &[&str]) -> DictionaryEntry {
        DictionaryEntry {
            id: Uuid::new_v4(),
            term: term.into(),
            heard_as: heard.iter().map(|h| (*h).to_owned()).collect(),
            enabled: true,
            source: EntrySource::Manual,
            created_at_ms: 0,
            updated_at_ms: 0,
        }
    }

    fn dict_draft(term: &str, heard: &[&str]) -> BacktestDraft {
        BacktestDraft::Dictionary {
            id: None,
            entry: DictionaryDraft { term: term.into(), heard_as: heard.iter().map(|h| (*h).to_owned()).collect(), enabled: true },
        }
    }

    fn rule_draft(pattern: &str, replacement: &str) -> BacktestDraft {
        BacktestDraft::Rule {
            id: None,
            rule: RuleDraft {
                name: format!("{pattern} → {replacement}"),
                kind: RuleKind::Literal,
                pattern: pattern.into(),
                replacement: replacement.into(),
                case_sensitive: false,
                enabled: true,
            },
        }
    }

    /// A draft entry counts the takes whose recognised text it would correct, leaves out the take
    /// the card was opened on, and flags the takes whose final text kept the form (§16.9).
    #[test]
    fn a_dictionary_draft_counts_corrections_and_kept_forms() {
        let here = take("用维他命测了一下", "用 Voltip 测了一下。", 9);
        let history = vec![
            here.clone(),
            take("维他命的安装包发给我", "维他命的安装包发给我。", 8),
            take("今天用维他命跑了一遍回归", "今天用 Voltip 跑了一遍回归。", 7),
            take("与此无关", "与此无关。", 6),
        ];
        let out = backtest(&[], &history, &dict_draft("Voltip", &["维他命"]), Some(here.id)).unwrap();
        assert_eq!((out.scanned, out.matched, out.kept), (3, 2, 1));
        assert_eq!(out.examples.len(), 2);
        assert!(out.examples[0].kept, "the kept take comes first");
        assert_eq!(out.examples[0].before, "维他命的安装包发给我");
        assert_eq!(out.examples[0].after, "Voltip的安装包发给我");
        assert_eq!((out.examples[1].before.as_str(), out.examples[1].after.as_str()), ("今天用维他命跑了一遍回归", "今天用Voltip跑了一遍回归"));
    }

    /// Only what the draft changes counts: a form the dictionary already corrects is no match, and
    /// a draft standing in for an existing entry is compared with that entry's current behaviour.
    #[test]
    fn the_baseline_is_the_current_dictionary() {
        let voltip = entry("Voltip", &["维他命"]);
        let history = vec![take("维他命和沃尔提普", "Voltip 和沃尔提普。", 2)];
        let out = backtest(std::slice::from_ref(&voltip), &history, &dict_draft("Voltip2", &["维他命"]), None).unwrap();
        assert_eq!(out.matched, 0, "a form an earlier entry already owns changes nothing (and the save refuses the draft)");
        let replace = BacktestDraft::Dictionary {
            id: Some(voltip.id),
            entry: DictionaryDraft { term: "Voltip".into(), heard_as: vec!["维他命".into(), "沃尔提普".into()], enabled: true },
        };
        let out = backtest(std::slice::from_ref(&voltip), &history, &replace, None).unwrap();
        assert_eq!(out.matched, 1);
        assert_eq!(out.examples[0].before, "Voltip和沃尔提普");
        assert_eq!(out.examples[0].after, "Voltip和Voltip");
        let same = BacktestDraft::Dictionary {
            id: Some(voltip.id),
            entry: DictionaryDraft { term: "Voltip".into(), heard_as: vec!["维他命".into()], enabled: true },
        };
        assert_eq!(backtest(&[voltip], &history, &same, None).unwrap().matched, 0, "no change, no match");
    }

    /// A draft rule rewrites the final text on its own; `kept` stays 0 for rules.
    #[test]
    fn a_rule_draft_counts_rewritten_final_texts() {
        let history = vec![take("发到非书群", "结果发到非书群了。", 3), take("飞书已经对了", "飞书已经对了。", 2), take("非书", "非书。", 1)];
        let out = backtest(&[], &history, &rule_draft("非书", "飞书"), None).unwrap();
        assert_eq!((out.scanned, out.matched, out.kept), (3, 2, 0));
        assert_eq!(out.examples[0].before, "结果发到非书群了。");
        assert_eq!(out.examples[0].after, "结果发到飞书群了。");
        assert!(out.examples.iter().all(|e| !e.kept));
    }

    /// Long texts are cut around the change with `…`; at most three examples come back.
    #[test]
    fn examples_are_short_and_few() {
        let raw = format!("{}维他命{}", "前".repeat(30), "后".repeat(30));
        let history: Vec<HistoryText> = (0..5).map(|i| take(&raw, "无关。", 10 - i)).collect();
        let out = backtest(&[], &history, &dict_draft("Voltip", &["维他命"]), None).unwrap();
        assert_eq!((out.matched, out.examples.len()), (5, MAX_BACKTEST_EXAMPLES));
        let e = &out.examples[0];
        assert_eq!(e.before, format!("…{}维他命{}…", "前".repeat(12), "后".repeat(12)));
        assert_eq!(e.after, format!("…{}Voltip{}…", "前".repeat(12), "后".repeat(12)));
        assert_eq!(out.examples.iter().map(|e| e.at_ms).collect::<Vec<_>>(), vec![10, 9, 8], "newest first");
    }

    #[test]
    fn drafts_are_validated_and_disabled_or_empty_drafts_match_nothing() {
        let history = vec![take("维他命", "维他命。", 1)];
        assert!(backtest(&[], &history, &dict_draft("", &["维他命"]), None).is_err());
        assert!(backtest(&[], &history, &rule_draft("", "x"), None).is_err());
        let off = BacktestDraft::Dictionary { id: None, entry: DictionaryDraft { term: "Voltip".into(), heard_as: vec!["维他命".into()], enabled: false } };
        assert_eq!(backtest(&[], &history, &off, None).unwrap().matched, 0);
        assert_eq!(backtest(&[], &history, &dict_draft("Voltip", &[]), None).unwrap(), Backtest { scanned: 1, ..Backtest::default() });
        assert_eq!(backtest(&[], &[], &dict_draft("Voltip", &["维他命"]), None).unwrap(), Backtest::default());
    }

    #[test]
    fn the_wire_shape_is_tagged_by_kind() {
        let json = serde_json::to_value(dict_draft("Voltip", &["维他命"])).unwrap();
        assert_eq!(json, serde_json::json!({ "kind": "dictionary", "id": null, "entry": { "term": "Voltip", "heard_as": ["维他命"], "enabled": true } }));
        let back: BacktestDraft = serde_json::from_value(serde_json::json!({ "kind": "rule", "rule": {
            "name": "非书 → 飞书", "kind": "literal", "pattern": "非书", "replacement": "飞书", "case_sensitive": false, "enabled": true } }))
        .unwrap();
        assert!(matches!(back, BacktestDraft::Rule { id: None, .. }));
    }

    #[test]
    fn snippets_handle_edges() {
        assert_eq!(snippets("abc", "abc"), ("abc".to_owned(), "abc".to_owned()));
        assert_eq!(snippets("ab", "abc"), ("ab".to_owned(), "abc".to_owned()));
        assert_eq!(snippets("", "x"), (String::new(), "x".to_owned()));
    }
}
