//! The dictionary's learning suggestions (docs/dictation.md §16.10): the pairs the AI clean-up
//! changed in the recent takes — what the recogniser wrote (`heard`) and what the polished text has
//! instead (`polished`) — that the dictionary does not correct yet and the user has not ignored,
//! most frequent first. The pairs are [`candidates`]'s, the word-by-word comparison of the MCP tool
//! `vocabulary_candidates` (§25) and of the history page (§16.9); nothing here calls an AI service.
//!
//! [`accept`] turns a suggestion into a dictionary entry, or adds its `heard` to the entry that
//! already has the right spelling, after checking that no rule rewrites that spelling and no
//! entry is in the way; [`undo`] puts the entry back exactly as it was. The undo records live in
//! memory only ([`UndoLog`]): the page offers 撤销 while it stays open.

use std::collections::VecDeque;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{
    DictionaryDraft, DictionaryEntry, DictionaryStore, EntrySource, IgnoredPair, MAX_DICTIONARY_ENTRIES, MAX_HEARD_AS, ReplacementRule, Vocabulary,
    validate_dictionary_draft,
};
use crate::history::HistoryText;
use crate::mcp::candidates::{MAX_PAIRS, PairExample, candidates};

/// Most suggestions one call returns (the MCP tool's [`MAX_PAIRS`]).
pub const MAX_SUGGESTIONS: usize = MAX_PAIRS;
/// Accepts the core remembers for 撤销; past it the oldest can no longer be undone.
pub const MAX_UNDOS: usize = 100;

/// One suggestion: a pair the AI clean-up changed, counted over the takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Suggestion {
    /// What the recogniser wrote.
    pub heard: String,
    /// What the polished text has instead.
    pub polished: String,
    /// How often, in all takes scanned.
    pub count: u32,
    /// The newest take it happened in.
    pub last_at_ms: u64,
    /// Up to three snippets of the recognised text around it, newest first.
    pub examples: Vec<PairExample>,
}

/// The suggestions over the recent takes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Suggestions {
    /// Takes looked at.
    pub scanned: u32,
    /// Of those, the takes the AI clean-up polished (only those can give a suggestion).
    pub refined: u32,
    /// Takes left out as too long to compare.
    pub skipped: u32,
    /// Suggestions in all, before `pairs` was cut to the limit.
    pub total: u32,
    /// The suggestions, most frequent first, then the most recent.
    pub pairs: Vec<Suggestion>,
}

/// What `vocabulary_suggestions` answers: the suggestions, or that there are none to look for
/// because the history is off.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SuggestionsAnswer {
    /// History recording is off: the page asks for it to be turned on.
    HistoryOff,
    /// The suggestions.
    Ready(Suggestions),
}

/// The pairs of `history` (newest first) the AI clean-up made, as [`candidates`] counts them,
/// without the ones `dictionary` corrects already (`known`) and the ones in `ignored`, most
/// frequent first, then the most recent; at most `limit` of them in `pairs`.
pub fn suggestions(dictionary: &[DictionaryEntry], rules: &[ReplacementRule], history: &[HistoryText], ignored: &[IgnoredPair], limit: usize) -> Suggestions {
    let found = candidates(dictionary, rules, history, None, usize::MAX);
    let scanned = usize::try_from(found.scanned).unwrap_or(usize::MAX).min(history.len());
    let mut pairs: Vec<Suggestion> = found
        .pairs
        .into_iter()
        .filter(|p| !p.known && !ignored.iter().any(|i| i.heard == p.heard && i.polished == p.polished))
        .map(|p| Suggestion { heard: p.heard, polished: p.polished, count: p.count, last_at_ms: p.last_at_ms, examples: p.examples })
        .collect();
    pairs.sort_by(|a, b| b.count.cmp(&a.count).then(b.last_at_ms.cmp(&a.last_at_ms)));
    let total = u32::try_from(pairs.len()).unwrap_or(u32::MAX);
    pairs.truncate(limit);
    Suggestions {
        scanned: found.scanned,
        refined: u32::try_from(history[..scanned].iter().filter(|t| t.refined).count()).unwrap_or(u32::MAX),
        skipped: found.skipped,
        total,
        pairs,
    }
}

/// How an entry stands in the way of an accept.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryClash {
    /// `heard` is the entry's right spelling.
    HeardIsTerm,
    /// `heard` is one of the entry's mis-hearings already.
    HeardTaken,
    /// The right spelling is one of the entry's mis-hearings.
    TermTaken,
    /// The entry with this right spelling is turned off.
    Disabled,
    /// The entry with this right spelling lists [`MAX_HEARD_AS`] mis-hearings already.
    TooMany,
}

/// Why an accept was refused, naming what is in the way. Nothing changed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Refusal {
    /// A rule that runs rewrites the right spelling, so the correction would not last:
    /// `becomes` is the spelling after the rules.
    Rule {
        /// The first rule that changes it.
        rule_id: Uuid,
        /// Its name.
        name: String,
        /// The spelling after all the rules.
        becomes: String,
    },
    /// An entry is in the way.
    Entry {
        /// The entry.
        entry_id: Uuid,
        /// Its right spelling.
        term: String,
        /// How.
        clash: EntryClash,
    },
    /// The dictionary holds [`MAX_DICTIONARY_ENTRIES`] entries already.
    Full {
        /// The limit.
        max: u32,
    },
    /// The pair is not an entry on its own, or the file could not be written: the core's text.
    Failed {
        /// Why.
        message: String,
    },
}

/// What 撤销 names: the accept, the entry it created or extended, and what it was before.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuggestionUndo {
    /// The accept, for `vocabulary_suggestion_undo`.
    pub id: Uuid,
    /// The entry.
    pub entry_id: Uuid,
    /// Its right spelling.
    pub term: String,
    /// The accept created the entry; otherwise it added `heard` to its mis-hearings.
    pub created: bool,
    /// The entry's mis-hearings before (empty when it was created).
    pub previous_heard_as: Vec<String>,
}

/// What `vocabulary_suggestion_accept` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AcceptOutcome {
    /// The dictionary has the pair now.
    Accepted {
        /// What 撤销 sends back.
        undo: SuggestionUndo,
    },
    /// Nothing changed.
    Refused {
        /// Why.
        refusal: Refusal,
    },
}

/// What `vocabulary_suggestion_undo` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UndoOutcome {
    /// The entry is as it was before the accept.
    Undone,
    /// The core does not know this accept (undone already, too old, or the app restarted).
    Expired,
    /// The entry was deleted since.
    Gone,
    /// The entry was changed since: undoing would lose that change.
    Changed,
    /// The file could not be written: the core's text.
    Failed {
        /// Why.
        message: String,
    },
}

/// What an accept changed: the entry before (`None`: the accept created it) and as it left it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Accepted {
    /// Before.
    pub before: Option<DictionaryEntry>,
    /// After.
    pub after: DictionaryEntry,
}

/// Add `heard` → `polished` to the dictionary in `store` (docs/dictation.md §16.10): to the entry
/// whose right spelling it is (ignoring ASCII case), or as a new entry from history entry
/// `history_id`. Refused, with nothing changed, when a rule that runs rewrites the spelling, an
/// entry is in the way, or the dictionary is full.
pub fn accept(store: &mut DictionaryStore, rules: &[ReplacementRule], heard: &str, polished: &str, history_id: Uuid, now_ms: u64) -> Result<Accepted, Refusal> {
    let failed = |e: super::VocabularyError| Refusal::Failed { message: e.to_string() };
    let draft = validate_dictionary_draft(&DictionaryDraft { term: polished.into(), heard_as: vec![heard.into()], enabled: true }).map_err(failed)?;
    let (term, heard) = (draft.term, draft.heard_as.into_iter().next().unwrap_or_default());
    // The rules run on the final text after the dictionary: one that rewrites the spelling would
    // undo the correction every time.
    let ruled = Vocabulary::compile(&[], rules).apply_rules(&term);
    if ruled.text != term
        && let Some(rule) = ruled.hits.first().and_then(|hit| rules.iter().find(|r| r.id == hit.id))
    {
        return Err(Refusal::Rule { rule_id: rule.id, name: rule.name.clone(), becomes: ruled.text });
    }
    let entries = store.entries();
    let target = entries.iter().find(|e| e.term.eq_ignore_ascii_case(&term)).cloned();
    let in_the_way = |entry: &DictionaryEntry, clash: EntryClash| Refusal::Entry { entry_id: entry.id, term: entry.term.clone(), clash };
    let has_form = |entry: &DictionaryEntry, form: &str| entry.heard_as.iter().any(|h| h.eq_ignore_ascii_case(form));
    for other in entries.iter().filter(|e| target.as_ref().is_none_or(|t| t.id != e.id)) {
        if other.term.eq_ignore_ascii_case(&heard) {
            return Err(in_the_way(other, EntryClash::HeardIsTerm));
        }
        if has_form(other, &heard) {
            return Err(in_the_way(other, EntryClash::HeardTaken));
        }
        if target.is_none() && has_form(other, &term) {
            return Err(in_the_way(other, EntryClash::TermTaken));
        }
    }
    let id = match &target {
        Some(entry) => {
            if !entry.enabled {
                return Err(in_the_way(entry, EntryClash::Disabled));
            }
            if has_form(entry, &heard) {
                return Err(in_the_way(entry, EntryClash::HeardTaken));
            }
            if entry.heard_as.len() >= MAX_HEARD_AS {
                return Err(in_the_way(entry, EntryClash::TooMany));
            }
            let mut heard_as = entry.heard_as.clone();
            heard_as.push(heard);
            store.update(entry.id, &DictionaryDraft { term: entry.term.clone(), heard_as, enabled: true }, now_ms).map_err(failed)?;
            entry.id
        }
        None => {
            if entries.len() >= MAX_DICTIONARY_ENTRIES {
                return Err(Refusal::Full { max: u32::try_from(MAX_DICTIONARY_ENTRIES).unwrap_or(u32::MAX) });
            }
            store.add(&DictionaryDraft { term, heard_as: vec![heard], enabled: true }, EntrySource::History { history_id }, now_ms).map_err(failed)?
        }
    };
    let after =
        store.entries().iter().find(|e| e.id == id).cloned().ok_or_else(|| Refusal::Failed { message: "dictionary: 词条保存后不见了".into() })?;
    Ok(Accepted { before: target, after })
}

/// Put the entry `accepted` changed back as it was, while it is still exactly as the accept left
/// it.
pub fn undo(store: &mut DictionaryStore, accepted: &Accepted) -> UndoOutcome {
    match store.entries().iter().find(|e| e.id == accepted.after.id) {
        None => return UndoOutcome::Gone,
        Some(current) if *current != accepted.after => return UndoOutcome::Changed,
        Some(_) => {}
    }
    let result = match &accepted.before {
        None => store.remove(accepted.after.id),
        Some(before) => store.restore(before.clone()),
    };
    match result {
        Ok(()) => UndoOutcome::Undone,
        Err(e) => UndoOutcome::Failed { message: e.to_string() },
    }
}

/// The accepts 撤销 can still take back, newest last, at most [`MAX_UNDOS`]; in memory only.
#[derive(Debug, Default)]
pub struct UndoLog {
    records: VecDeque<(Uuid, Accepted)>,
}

impl UndoLog {
    /// Remember `accepted`; returns what the page keeps for 撤销.
    pub fn push(&mut self, accepted: Accepted) -> SuggestionUndo {
        let id = Uuid::new_v4();
        let undo = SuggestionUndo {
            id,
            entry_id: accepted.after.id,
            term: accepted.after.term.clone(),
            created: accepted.before.is_none(),
            previous_heard_as: accepted.before.as_ref().map(|b| b.heard_as.clone()).unwrap_or_default(),
        };
        self.records.push_back((id, accepted));
        while self.records.len() > MAX_UNDOS {
            self.records.pop_front();
        }
        undo
    }

    /// Take the accept `id` names out of the log.
    pub fn take(&mut self, id: Uuid) -> Option<Accepted> {
        let at = self.records.iter().position(|(held, _)| *held == id)?;
        self.records.remove(at).map(|(_, accepted)| accepted)
    }
}

/// Where a command's answer goes: the caller waits on the receiver, the core sends once.
pub struct Reply<T>(Arc<Mutex<Option<tokio::sync::oneshot::Sender<T>>>>);

impl<T> Reply<T> {
    /// The reply to hand the command, and the receiver the caller waits on.
    pub fn channel() -> (Self, tokio::sync::oneshot::Receiver<T>) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        (Self(Arc::new(Mutex::new(Some(tx)))), rx)
    }

    /// Answer; a second answer, or one nobody waits for any more, goes nowhere.
    pub fn send(&self, value: T) {
        if let Some(tx) = self.0.lock().take() {
            let _ = tx.send(value);
        }
    }
}

impl<T> Clone for Reply<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> std::fmt::Debug for Reply<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Reply")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vocabulary::{RuleKind, VocabularyHits};

    fn take(raw: &str, text: &str, at_ms: u64) -> HistoryText {
        HistoryText { id: Uuid::new_v4(), at_ms, raw_text: raw.into(), text: text.into(), refined: true, vocabulary: VocabularyHits::default() }
    }

    fn rule(name: &str, kind: RuleKind, pattern: &str, replacement: &str) -> ReplacementRule {
        ReplacementRule {
            id: Uuid::new_v4(),
            name: name.into(),
            kind,
            pattern: pattern.into(),
            replacement: replacement.into(),
            case_sensitive: true,
            enabled: true,
            created_at_ms: 1,
            updated_at_ms: 1,
            source: EntrySource::Manual,
            pending: false,
        }
    }

    fn ignored(heard: &str, polished: &str) -> IgnoredPair {
        IgnoredPair { heard: heard.into(), polished: polished.into(), at_ms: 1 }
    }

    fn pairs(out: &Suggestions) -> Vec<(&str, &str, u32)> {
        out.pairs.iter().map(|p| (p.heard.as_str(), p.polished.as_str(), p.count)).collect()
    }

    /// A store in a fresh directory holding `entries` (term, mis-hearings, on).
    fn store_with(dir: &std::path::Path, entries: &[(&str, &[&str], bool)]) -> DictionaryStore {
        let (mut store, _) = DictionaryStore::open(dir);
        for (term, heard, enabled) in entries {
            let draft = DictionaryDraft { term: (*term).into(), heard_as: heard.iter().map(|h| (*h).to_owned()).collect(), enabled: *enabled };
            store.add(&draft, EntrySource::Manual, 1).unwrap();
        }
        store
    }

    fn history() -> Vec<HistoryText> {
        vec![
            take("我们用维他命开会", "我们用 Voltip 开会。", 50),
            take("发到非书群", "发到飞书群。", 40),
            take("又是非书", "又是飞书。", 30),
            take("库伯内特斯部署", "Kubernetes 部署", 20),
            HistoryText { refined: false, ..take("没有润色的非书", "没有润色的飞书", 10) },
        ]
    }

    /// docs/dictation.md §16.10: the pairs the clean-up made, counted, most frequent first and then
    /// the most recent, with the takes looked at and the polished ones among them.
    #[test]
    fn suggestions_are_counted_and_sorted() {
        let history = history();
        let out = suggestions(&[], &[], &history, &[], MAX_SUGGESTIONS);
        assert_eq!((out.scanned, out.refined, out.skipped, out.total), (5, 4, 0, 3));
        assert_eq!(pairs(&out), [("非", "飞", 2), ("维他命", "Voltip", 1), ("库伯内特斯", "Kubernetes", 1)], "count, then the newest first");
        assert_eq!(out.pairs[0].last_at_ms, 40);
        assert_eq!(out.pairs[0].examples.iter().map(|e| e.history_id).collect::<Vec<_>>(), [history[1].id, history[2].id], "newest first");
        assert_eq!(out.pairs[1].examples[0].snippet, "我们用维他命开会");
        let one = suggestions(&[], &[], &history, &[], 1);
        assert_eq!((one.total, pairs(&one)), (3, vec![("非", "飞", 2)]), "the limit cuts the list, the total stays");
        let none = suggestions(&[], &[], &[], &[], MAX_SUGGESTIONS);
        assert_eq!((none.scanned, none.refined, none.total, none.pairs.len()), (0, 0, 0, 0));
    }

    /// A pair whose `heard` an enabled entry corrects already, and an ignored pair, are not
    /// suggested; an entry that is off does not count, and an ignored pair is matched exactly.
    #[test]
    fn known_and_ignored_pairs_are_left_out() {
        let history = history();
        let dir = tempfile::tempdir().unwrap();
        let store = store_with(dir.path(), &[("Voltip", &["维他命"], true), ("飞书", &["非"], false)]);
        let out = suggestions(store.entries(), &[], &history, &[ignored("库伯内特斯", "Kubernetes")], MAX_SUGGESTIONS);
        assert_eq!(pairs(&out), [("非", "飞", 2)], "known: 维他命; ignored: 库伯内特斯; the entry that is off does not count");
        assert_eq!(out.total, 1);
        let other_case = suggestions(&[], &[], &history, &[ignored("库伯内特斯", "kubernetes")], MAX_SUGGESTIONS);
        assert_eq!(other_case.total, 3, "an ignored pair is matched exactly");
    }

    /// docs/dictation.md §16.10: once a suggestion is in the dictionary, the next look leaves it
    /// out (the dictionary explains it now), whether it made a new entry or extended one.
    #[test]
    fn regression_a_pair_the_dictionary_already_explains_is_not_suggested() {
        let history = vec![take("我们用维他命开会", "我们用 Voltip 开会。", 50), take("发到非书群", "发到飞书群。", 40)];
        let dir = tempfile::tempdir().unwrap();
        let mut store = store_with(dir.path(), &[("飞书", &["飞输"], true)]);
        let before = suggestions(store.entries(), &[], &history, &[], MAX_SUGGESTIONS);
        assert_eq!(pairs(&before), [("维他命", "Voltip", 1), ("非", "飞", 1)]);
        accept(&mut store, &[], "维他命", "Voltip", history[0].id, 100).unwrap();
        accept(&mut store, &[], "非", "飞", history[1].id, 101).unwrap();
        let after = suggestions(store.entries(), &[], &history, &[], MAX_SUGGESTIONS);
        assert!(after.pairs.is_empty(), "{:?}", after.pairs);
        assert_eq!(after.total, 0);
    }

    /// A new right spelling becomes an entry from the history; a known one gets the mis-hearing
    /// added, its own flag, source and creation kept (ASCII case of the spelling ignored).
    #[test]
    fn accept_creates_an_entry_or_adds_to_the_one_with_the_spelling() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store_with(dir.path(), &[("kubernetes", &["酷伯"], true)]);
        let from = Uuid::new_v4();
        let created = accept(&mut store, &[], "维他命", "Voltip", from, 100).unwrap();
        assert!(created.before.is_none());
        let entry = store.entries().iter().find(|e| e.term == "Voltip").unwrap().clone();
        assert_eq!(created.after, entry);
        assert_eq!((entry.heard_as.clone(), entry.enabled, entry.source.clone()), (vec!["维他命".to_owned()], true, EntrySource::History { history_id: from }));
        assert_eq!((entry.created_at_ms, entry.updated_at_ms), (100, 100));

        let original = store.entries()[0].clone();
        let added = accept(&mut store, &[], "库伯内特斯", "Kubernetes", Uuid::new_v4(), 200).unwrap();
        assert_eq!(added.before.as_ref(), Some(&original));
        let entry = store.entries()[0].clone();
        assert_eq!(added.after, entry);
        assert_eq!(entry.term, "kubernetes", "the entry's own spelling stays");
        assert_eq!(entry.heard_as, ["酷伯", "库伯内特斯"]);
        assert_eq!((entry.source, entry.created_at_ms, entry.updated_at_ms), (EntrySource::Manual, original.created_at_ms, 200));
        assert_eq!(store.entries().len(), 2);
    }

    /// An entry with [`MAX_HEARD_AS`] mis-hearings, one that is off, and a full dictionary refuse,
    /// naming the entry; nothing changes.
    #[test]
    fn accept_refuses_a_full_entry_an_entry_that_is_off_and_a_full_dictionary() {
        let dir = tempfile::tempdir().unwrap();
        let many: Vec<String> = (0..MAX_HEARD_AS).map(|i| format!("沃{i}")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        let mut store = store_with(dir.path(), &[("Voltip", &many, true), ("飞书", &[], false)]);
        let before = store.entries().to_vec();
        let voltip = before[0].clone();
        assert_eq!(
            accept(&mut store, &[], "维他命", "Voltip", Uuid::new_v4(), 2).unwrap_err(),
            Refusal::Entry { entry_id: voltip.id, term: "Voltip".into(), clash: EntryClash::TooMany }
        );
        assert_eq!(
            accept(&mut store, &[], "非书", "飞书", Uuid::new_v4(), 2).unwrap_err(),
            Refusal::Entry { entry_id: before[1].id, term: "飞书".into(), clash: EntryClash::Disabled }
        );
        assert_eq!(store.entries(), before.as_slice(), "nothing changed");

        let dir = tempfile::tempdir().unwrap();
        let (mut full, _) = DictionaryStore::open(dir.path());
        for i in 0..MAX_DICTIONARY_ENTRIES {
            full.add(&DictionaryDraft { term: format!("词{i}"), heard_as: vec![], enabled: true }, EntrySource::Manual, 1).unwrap();
        }
        assert_eq!(accept(&mut full, &[], "维他命", "Voltip", Uuid::new_v4(), 2).unwrap_err(), Refusal::Full { max: 500 });
        assert_eq!(full.entries().len(), MAX_DICTIONARY_ENTRIES);
    }

    /// A rule that runs and rewrites the right spelling refuses the accept and is named, with the
    /// spelling it ends as; a rule that is off or waits for approval does not count.
    #[test]
    fn accept_refuses_when_a_rule_rewrites_the_spelling() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store_with(dir.path(), &[]);
        let back = rule("改回去", RuleKind::Literal, "Voltip", "维他命");
        let shout = rule("大写", RuleKind::Regex, "(?i)voltip", "VOLTIP");
        let refused = accept(&mut store, &[shout.clone(), back.clone()], "维他命", "Voltip", Uuid::new_v4(), 2).unwrap_err();
        assert_eq!(
            refused,
            Refusal::Rule { rule_id: shout.id, name: "大写".into(), becomes: "VOLTIP".into() },
            "the first rule that changes it, and the end result"
        );
        let refused = accept(&mut store, std::slice::from_ref(&back), "维他命", "Voltip", Uuid::new_v4(), 2).unwrap_err();
        assert_eq!(refused, Refusal::Rule { rule_id: back.id, name: "改回去".into(), becomes: "维他命".into() }, "changed back into what was heard");
        assert!(store.entries().is_empty());
        let off = ReplacementRule { enabled: false, ..back.clone() };
        let waiting = ReplacementRule { pending: true, enabled: false, ..shout };
        let unrelated = rule("其他", RuleKind::Literal, "飞书", "Lark");
        assert!(accept(&mut store, &[off, waiting, unrelated], "维他命", "Voltip", Uuid::new_v4(), 2).is_ok());
    }

    /// `heard` that is another entry's spelling or mis-hearing, and a right spelling that is
    /// another entry's mis-hearing, refuse the accept and name that entry; nothing changes.
    #[test]
    fn accept_refuses_when_another_entry_is_in_the_way() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store_with(dir.path(), &[("维他命", &[], true), ("Vitamin", &["维他名"], false), ("Wolt", &["voltip"], true)]);
        let before = store.entries().to_vec();
        let refusal = |store: &mut DictionaryStore, heard: &str, polished: &str| accept(store, &[], heard, polished, Uuid::new_v4(), 2).unwrap_err();
        assert_eq!(
            refusal(&mut store, "维他命", "Vitamin C"),
            Refusal::Entry { entry_id: before[0].id, term: "维他命".into(), clash: EntryClash::HeardIsTerm }
        );
        assert_eq!(
            refusal(&mut store, "维他名", "Vitamin C"),
            Refusal::Entry { entry_id: before[1].id, term: "Vitamin".into(), clash: EntryClash::HeardTaken },
            "an entry that is off is still in the way"
        );
        assert_eq!(
            refusal(&mut store, "沃提普", "Voltip"),
            Refusal::Entry { entry_id: before[2].id, term: "Wolt".into(), clash: EntryClash::TermTaken },
            "ASCII case ignored, as the dictionary does"
        );
        assert!(matches!(refusal(&mut store, "同", "同"), Refusal::Failed { .. }), "not an entry on its own");
        assert_eq!(store.entries(), before.as_slice());
    }

    /// 撤销 deletes an entry the accept created and gives an extended one its mis-hearings back,
    /// exactly as it was; not after the entry was changed or deleted since. The log forgets an
    /// accept once taken and keeps the newest [`MAX_UNDOS`].
    #[test]
    fn undo_puts_the_entry_back_while_it_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store_with(dir.path(), &[("Kubernetes", &["酷伯"], true)]);
        let original = store.entries().to_vec();
        let mut log = UndoLog::default();
        let created = accept(&mut store, &[], "维他命", "Voltip", Uuid::new_v4(), 100).unwrap();
        let added = accept(&mut store, &[], "库伯内特斯", "Kubernetes", Uuid::new_v4(), 200).unwrap();
        let created_undo = log.push(created);
        let added_undo = log.push(added);
        assert_eq!((created_undo.term.as_str(), created_undo.created, created_undo.previous_heard_as.is_empty()), ("Voltip", true, true));
        assert_eq!((added_undo.entry_id, added_undo.created, added_undo.previous_heard_as.clone()), (original[0].id, false, vec!["酷伯".to_owned()]));

        let added = log.take(added_undo.id).expect("remembered");
        assert!(log.take(added_undo.id).is_none(), "taken once");
        assert_eq!(undo(&mut store, &added), UndoOutcome::Undone);
        let created = log.take(created_undo.id).expect("remembered");
        assert_eq!(undo(&mut store, &created), UndoOutcome::Undone);
        assert_eq!(store.entries(), original.as_slice(), "exactly as before, timestamps included");

        let again = accept(&mut store, &[], "维他命", "Voltip", Uuid::new_v4(), 300).unwrap();
        let id = again.after.id;
        store.update(id, &DictionaryDraft { term: "Voltip".into(), heard_as: vec!["维他命".into(), "沃提普".into()], enabled: true }, 301).unwrap();
        assert_eq!(undo(&mut store, &again), UndoOutcome::Changed, "the change since would be lost");
        store.remove(id).unwrap();
        assert_eq!(undo(&mut store, &again), UndoOutcome::Gone);

        let mut log = UndoLog::default();
        let undos: Vec<SuggestionUndo> = (0..=MAX_UNDOS).map(|_| log.push(again.clone())).collect();
        assert!(log.take(undos[0].id).is_none(), "the oldest went");
        assert!(log.take(undos[MAX_UNDOS].id).is_some());
    }
}
