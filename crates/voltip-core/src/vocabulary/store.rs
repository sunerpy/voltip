//! `dictionary.json` and `rules.json` in the app data directory (docs/dictation.md §16.1), on the
//! shared list persistence of [`crate::list_file`]: every mutation validates the whole new list,
//! writes it atomically and only then replaces the list in memory; a file that cannot be used is
//! moved aside to `<file>.corrupt-<unix seconds>` (never deleted) and the store starts empty.

use std::path::Path;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{
    DictionaryDraft, DictionaryEntry, EntrySource, ImportMode, ReplacementRule, RuleDraft, VocabularyError, check_dictionary, check_rules,
    validate_dictionary_draft, validate_rule_draft,
};
use crate::list_file::{ListFile, permute};

/// File name of the dictionary inside the app data directory.
pub const DICTIONARY_FILE_NAME: &str = "dictionary.json";
/// File name of the rule list inside the app data directory.
pub const RULES_FILE_NAME: &str = "rules.json";
/// On-disk schema of both files.
pub const VOCABULARY_SCHEMA: u16 = 1;

#[derive(Serialize, Deserialize)]
struct DictionaryFile {
    schema: u16,
    entries: Vec<DictionaryEntry>,
}

#[derive(Serialize, Deserialize)]
struct RulesFile {
    schema: u16,
    rules: Vec<ReplacementRule>,
}

/// A write failure of either file, as the UI sees it (`vocabulary: <path>：<reason>`).
fn store_err(reason: String) -> VocabularyError {
    VocabularyError::Store(reason)
}

fn dictionary_of(file: DictionaryFile) -> Option<Vec<DictionaryEntry>> {
    (file.schema == VOCABULARY_SCHEMA).then_some(file.entries)
}

/// A stored dictionary is usable when every entry is in its normalised form and the list keeps the rules.
fn check_stored_dictionary(entries: &[DictionaryEntry]) -> Result<(), String> {
    let valid = || -> Result<(), VocabularyError> {
        for e in entries {
            let draft = DictionaryDraft { term: e.term.clone(), heard_as: e.heard_as.clone(), enabled: e.enabled };
            if validate_dictionary_draft(&draft)? != draft {
                return Err(VocabularyError::Dictionary(format!("词条「{}」不是规范形式", e.term)));
            }
        }
        check_dictionary(entries)
    };
    valid().map_err(|e| e.to_string())
}

fn rules_of(file: RulesFile) -> Option<Vec<ReplacementRule>> {
    (file.schema == VOCABULARY_SCHEMA).then_some(file.rules)
}

/// Stored rules are usable when every rule is in its normalised form (its regex compiles) and the
/// list keeps the rules.
fn check_stored_rules(rules: &[ReplacementRule]) -> Result<(), String> {
    let valid = || -> Result<(), VocabularyError> {
        for r in rules {
            let draft = RuleDraft::from(r);
            if validate_rule_draft(&draft)? != draft {
                return Err(VocabularyError::Rules(format!("规则「{}」不是规范形式", r.name)));
            }
        }
        check_rules(rules)
    };
    valid().map_err(|e| e.to_string())
}

/// The personal dictionary (`dictionary.json`).
#[derive(Debug)]
pub struct DictionaryStore {
    file: ListFile,
    entries: Vec<DictionaryEntry>,
}

impl DictionaryStore {
    /// Open `dir/dictionary.json`; the second value is the notice for the UI when the file could
    /// not be used (see the module docs).
    pub fn open(dir: &Path) -> (Self, Option<String>) {
        let (file, entries, notice) = ListFile::load(dir, DICTIONARY_FILE_NAME, VOCABULARY_SCHEMA, dictionary_of, check_stored_dictionary);
        (Self { file, entries }, notice)
    }

    /// The entries of `dir/dictionary.json`, read with the same checks as [`Self::open`] and no
    /// side effect (docs/dictation.md §23).
    pub fn read(dir: &Path) -> Result<Vec<DictionaryEntry>, String> {
        ListFile::read(dir, DICTIONARY_FILE_NAME, VOCABULARY_SCHEMA, dictionary_of, check_stored_dictionary)
    }

    /// Current entries, in order.
    pub fn entries(&self) -> &[DictionaryEntry] {
        &self.entries
    }

    fn commit(&mut self, entries: Vec<DictionaryEntry>) -> Result<(), VocabularyError> {
        check_dictionary(&entries)?;
        self.file.save(&DictionaryFile { schema: VOCABULARY_SCHEMA, entries: entries.clone() }).map_err(store_err)?;
        self.entries = entries;
        Ok(())
    }

    /// Append a new entry; returns its id.
    pub fn add(&mut self, draft: &DictionaryDraft, source: EntrySource, now_ms: u64) -> Result<Uuid, VocabularyError> {
        let draft = validate_dictionary_draft(draft)?;
        let id = Uuid::new_v4();
        let mut entries = self.entries.clone();
        entries.push(DictionaryEntry {
            id,
            term: draft.term,
            heard_as: draft.heard_as,
            enabled: draft.enabled,
            source,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        });
        self.commit(entries)?;
        Ok(id)
    }

    /// Replace an entry's term, mis-hearings and flag (its source and creation time stay).
    pub fn update(&mut self, id: Uuid, draft: &DictionaryDraft, now_ms: u64) -> Result<(), VocabularyError> {
        let draft = validate_dictionary_draft(draft)?;
        let mut entries = self.entries.clone();
        let Some(entry) = entries.iter_mut().find(|e| e.id == id) else { return Err(unknown_entry(id)) };
        entry.term = draft.term;
        entry.heard_as = draft.heard_as;
        entry.enabled = draft.enabled;
        entry.updated_at_ms = now_ms;
        self.commit(entries)
    }

    /// Delete an entry.
    pub fn remove(&mut self, id: Uuid) -> Result<(), VocabularyError> {
        if !self.entries.iter().any(|e| e.id == id) {
            return Err(unknown_entry(id));
        }
        let entries = self.entries.iter().filter(|e| e.id != id).cloned().collect();
        self.commit(entries)
    }

    /// Put the entries in the order of `ids`, which must be exactly the current ids.
    pub fn reorder(&mut self, ids: &[Uuid]) -> Result<(), VocabularyError> {
        let entries = permute(&self.entries, ids, |e| e.id).ok_or_else(|| VocabularyError::Dictionary("新的顺序必须恰好包含现有的全部词条".into()))?;
        self.commit(entries)
    }
}

fn unknown_entry(id: Uuid) -> VocabularyError {
    VocabularyError::Dictionary(format!("没有 id 为 {id} 的词条"))
}

fn unknown_rule(id: Uuid) -> VocabularyError {
    VocabularyError::Rules(format!("没有 id 为 {id} 的规则"))
}

/// The replacement rules (`rules.json`), in execution order.
#[derive(Debug)]
pub struct RuleStore {
    file: ListFile,
    rules: Vec<ReplacementRule>,
}

fn rule_from(draft: RuleDraft, id: Uuid, created_at_ms: u64, now_ms: u64) -> ReplacementRule {
    ReplacementRule {
        id,
        name: draft.name,
        kind: draft.kind,
        pattern: draft.pattern,
        replacement: draft.replacement,
        case_sensitive: draft.case_sensitive,
        enabled: draft.enabled,
        created_at_ms,
        updated_at_ms: now_ms,
    }
}

impl RuleStore {
    /// Open `dir/rules.json`; the second value is the notice for the UI when the file could not be
    /// used (a stored regex that no longer compiles counts as unusable).
    pub fn open(dir: &Path) -> (Self, Option<String>) {
        let (file, rules, notice) = ListFile::load(dir, RULES_FILE_NAME, VOCABULARY_SCHEMA, rules_of, check_stored_rules);
        (Self { file, rules }, notice)
    }

    /// The rules of `dir/rules.json`, read with the same checks as [`Self::open`] and no side
    /// effect (docs/dictation.md §23).
    pub fn read(dir: &Path) -> Result<Vec<ReplacementRule>, String> {
        ListFile::read(dir, RULES_FILE_NAME, VOCABULARY_SCHEMA, rules_of, check_stored_rules)
    }

    /// Current rules, in execution order.
    pub fn rules(&self) -> &[ReplacementRule] {
        &self.rules
    }

    fn commit(&mut self, rules: Vec<ReplacementRule>) -> Result<(), VocabularyError> {
        check_rules(&rules)?;
        self.file.save(&RulesFile { schema: VOCABULARY_SCHEMA, rules: rules.clone() }).map_err(store_err)?;
        self.rules = rules;
        Ok(())
    }

    /// Append a new rule; returns its id.
    pub fn add(&mut self, draft: &RuleDraft, now_ms: u64) -> Result<Uuid, VocabularyError> {
        let draft = validate_rule_draft(draft)?;
        let id = Uuid::new_v4();
        let mut rules = self.rules.clone();
        rules.push(rule_from(draft, id, now_ms, now_ms));
        self.commit(rules)?;
        Ok(id)
    }

    /// Replace a rule (id, position and creation time stay).
    pub fn update(&mut self, id: Uuid, draft: &RuleDraft, now_ms: u64) -> Result<(), VocabularyError> {
        let draft = validate_rule_draft(draft)?;
        let mut rules = self.rules.clone();
        let Some(rule) = rules.iter_mut().find(|r| r.id == id) else { return Err(unknown_rule(id)) };
        *rule = rule_from(draft, id, rule.created_at_ms, now_ms);
        self.commit(rules)
    }

    /// Delete a rule.
    pub fn remove(&mut self, id: Uuid) -> Result<(), VocabularyError> {
        if !self.rules.iter().any(|r| r.id == id) {
            return Err(unknown_rule(id));
        }
        let rules = self.rules.iter().filter(|r| r.id != id).cloned().collect();
        self.commit(rules)
    }

    /// Put the rules in the order of `ids`, which must be exactly the current ids.
    pub fn reorder(&mut self, ids: &[Uuid]) -> Result<(), VocabularyError> {
        let rules = permute(&self.rules, ids, |r| r.id).ok_or_else(|| VocabularyError::Rules("新的顺序必须恰好包含现有的全部规则".into()))?;
        self.commit(rules)
    }

    /// `rules_import` (docs/dictation.md §16.5): `replace` makes `drafts` the list; `merge` updates
    /// same-name rules in place and appends the rest. All or nothing.
    pub fn import(&mut self, drafts: &[RuleDraft], mode: ImportMode, now_ms: u64) -> Result<(), VocabularyError> {
        let drafts = drafts.iter().map(validate_rule_draft).collect::<Result<Vec<_>, _>>()?;
        let rules = match mode {
            ImportMode::Replace => drafts.into_iter().map(|d| rule_from(d, Uuid::new_v4(), now_ms, now_ms)).collect(),
            ImportMode::Merge => {
                let mut rules = self.rules.clone();
                for draft in drafts {
                    match rules.iter_mut().find(|r| r.name == draft.name) {
                        Some(rule) => *rule = rule_from(draft, rule.id, rule.created_at_ms, now_ms),
                        None => rules.push(rule_from(draft, Uuid::new_v4(), now_ms, now_ms)),
                    }
                }
                rules
            }
        };
        self.commit(rules)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vocabulary::{MAX_DICTIONARY_ENTRIES, MAX_RULES, RuleKind};

    fn draft(term: &str, heard: &[&str]) -> DictionaryDraft {
        DictionaryDraft { term: term.into(), heard_as: heard.iter().map(|h| (*h).to_owned()).collect(), enabled: true }
    }

    fn rule(name: &str, kind: RuleKind, pattern: &str, replacement: &str) -> RuleDraft {
        RuleDraft { name: name.into(), kind, pattern: pattern.into(), replacement: replacement.into(), case_sensitive: true, enabled: true }
    }

    fn corrupt_files(dir: &Path, name: &str) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| n.starts_with(&format!("{name}.corrupt-")))
            .collect();
        names.sort();
        names
    }

    /// Add / update / remove / reorder round-trip through the file; the temporary file is renamed
    /// away; a refused change touches neither the file nor the list.
    #[test]
    fn dictionary_store_round_trips_every_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, notice) = DictionaryStore::open(dir.path());
        assert!(notice.is_none() && store.entries().is_empty());
        let a = store.add(&draft(" Voltip ", &["沃提普", " ", "我提", "沃提普"]), EntrySource::Manual, 10).unwrap();
        let history = Uuid::new_v4();
        let b = store.add(&draft("good idea", &["谷歌IDR"]), EntrySource::History { history_id: history }, 20).unwrap();
        assert_eq!(store.entries()[0].term, "Voltip", "trimmed");
        assert_eq!(store.entries()[0].heard_as, ["沃提普", "我提"], "blank dropped, duplicate collapsed");
        let reopened = DictionaryStore::open(dir.path()).0;
        assert_eq!(reopened.entries(), store.entries());
        assert_eq!(reopened.entries()[1].source, EntrySource::History { history_id: history });
        store.update(a, &DictionaryDraft { enabled: false, ..draft("Voltip", &["沃提普"]) }, 30).unwrap();
        let e = &store.entries()[0];
        assert_eq!((e.enabled, e.heard_as.len(), e.created_at_ms, e.updated_at_ms), (false, 1, 10, 30));
        store.reorder(&[b, a]).unwrap();
        assert_eq!(DictionaryStore::open(dir.path()).0.entries()[0].id, b);
        for bad in [vec![a], vec![a, a], vec![a, Uuid::new_v4()], vec![b, a, a]] {
            assert!(store.reorder(&bad).unwrap_err().to_string().contains("全部词条"), "{bad:?}");
        }
        // Refusals leave everything as it was.
        let before = std::fs::read_to_string(dir.path().join(DICTIONARY_FILE_NAME)).unwrap();
        assert!(store.add(&draft("voltip", &[]), EntrySource::Manual, 40).unwrap_err().to_string().contains("已有「Voltip」"));
        assert!(store.add(&draft("X", &["沃提普"]), EntrySource::Manual, 40).unwrap_err().to_string().contains("已是「Voltip」的误识别写法"));
        assert!(store.add(&draft("Y", &["good idea"]), EntrySource::Manual, 40).unwrap_err().to_string().contains("正确写法"));
        assert!(store.update(Uuid::new_v4(), &draft("Z", &[]), 40).unwrap_err().to_string().contains("没有 id"));
        assert!(store.remove(Uuid::new_v4()).unwrap_err().to_string().starts_with("dictionary: 没有 id"));
        assert_eq!(std::fs::read_to_string(dir.path().join(DICTIONARY_FILE_NAME)).unwrap(), before);
        assert_eq!(store.entries().len(), 2);
        store.remove(a).unwrap();
        assert_eq!(DictionaryStore::open(dir.path()).0.entries().len(), 1);
        assert!(!dir.path().join("dictionary.json.tmp").exists());
        assert!(format!("{store:?}").contains("dictionary.json"));
    }

    #[test]
    fn dictionary_store_caps_the_list() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, _) = DictionaryStore::open(dir.path());
        for i in 0..MAX_DICTIONARY_ENTRIES {
            store.add(&draft(&format!("term{i}"), &[]), EntrySource::Manual, 1).unwrap();
        }
        let err = store.add(&draft("one more", &[]), EntrySource::Manual, 1).unwrap_err();
        assert!(err.to_string().contains("最多 500 条"), "{err}");
        assert_eq!(DictionaryStore::open(dir.path()).0.entries().len(), MAX_DICTIONARY_ENTRIES);
    }

    /// A corrupt file never stops the store: it is renamed to `<file>.corrupt-<secs>` (twice in one
    /// second gets a suffix, nothing is ever overwritten or deleted), the store starts empty and
    /// says so, and it writes the original path again afterwards.
    #[test]
    fn corrupt_dictionary_files_are_quarantined_not_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DICTIONARY_FILE_NAME);
        std::fs::write(&path, b"{not json").unwrap();
        let (mut store, notice) = DictionaryStore::open(dir.path());
        let notice = notice.unwrap();
        assert!(notice.contains("dictionary.json 无法使用") && notice.contains(".corrupt-"), "{notice}");
        assert!(store.entries().is_empty() && !path.exists());
        let aside = corrupt_files(dir.path(), DICTIONARY_FILE_NAME);
        assert_eq!(aside.len(), 1);
        assert_eq!(std::fs::read(dir.path().join(&aside[0])).unwrap(), b"{not json");
        // Wrong schema, and a well-formed file whose content breaks the rules, are unusable too.
        std::fs::write(&path, br#"{"schema":9,"entries":[]}"#).unwrap();
        assert!(DictionaryStore::open(dir.path()).1.unwrap().contains("schema 不是 1"));
        let dup = format!(
            r#"{{"schema":1,"entries":[{{"id":"{}","term":"A","heard_as":[],"enabled":true,"source":{{"kind":"manual"}},"created_at_ms":1,"updated_at_ms":1}},{{"id":"{}","term":"a","heard_as":[],"enabled":true,"source":{{"kind":"manual"}},"created_at_ms":1,"updated_at_ms":1}}]}}"#,
            Uuid::new_v4(),
            Uuid::new_v4()
        );
        std::fs::write(&path, dup).unwrap();
        assert!(DictionaryStore::open(dir.path()).1.unwrap().contains("已有「A」"));
        let untrimmed = format!(
            r#"{{"schema":1,"entries":[{{"id":"{}","term":" A ","enabled":true,"source":{{"kind":"manual"}},"created_at_ms":1,"updated_at_ms":1}}]}}"#,
            Uuid::new_v4()
        );
        std::fs::write(&path, untrimmed).unwrap();
        assert!(DictionaryStore::open(dir.path()).1.unwrap().contains("不是规范形式"));
        assert_eq!(corrupt_files(dir.path(), DICTIONARY_FILE_NAME).len(), 4, "every bad file is kept, none overwritten");
        store.add(&draft("fresh", &[]), EntrySource::Manual, 1).unwrap();
        assert_eq!(DictionaryStore::open(dir.path()).0.entries()[0].term, "fresh");
    }

    /// A file that cannot be read (here: a directory in its place) is neither moved nor
    /// overwritten: the store starts empty, says so, and refuses to write.
    #[test]
    fn unreadable_files_make_the_store_read_only() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(RULES_FILE_NAME)).unwrap();
        let (mut rules, notice) = RuleStore::open(dir.path());
        assert!(notice.unwrap().contains("rules.json 无法读取"));
        let err = rules.add(&rule("a", RuleKind::Literal, "a", "b"), 1).unwrap_err();
        assert!(matches!(err, VocabularyError::Store(_)) && err.to_string().contains("为避免覆盖不写入"), "{err}");
        assert!(dir.path().join(RULES_FILE_NAME).is_dir(), "untouched");
        // A data directory that is a file: nothing can be written under it.
        let blocked = dir.path().join("blocked");
        std::fs::write(&blocked, b"x").unwrap();
        let (mut store, _) = DictionaryStore::open(&blocked);
        assert!(store.add(&draft("x", &[]), EntrySource::Manual, 1).unwrap_err().to_string().starts_with("vocabulary:"));
    }

    /// Rules keep their order; add / update / remove / reorder persist; an invalid regex is refused
    /// before it reaches the file; names are unique.
    #[test]
    fn rule_store_keeps_order_and_rejects_invalid_rules() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, _) = RuleStore::open(dir.path());
        let a = store.add(&rule("a", RuleKind::Literal, "x", "y"), 1).unwrap();
        let b = store.add(&rule("b", RuleKind::Regex, r"issue (\d+)", "#$1"), 2).unwrap();
        assert_eq!(RuleStore::open(dir.path()).0.rules().iter().map(|r| r.id).collect::<Vec<_>>(), [a, b]);
        let err = store.add(&rule("c", RuleKind::Regex, "(unclosed", ""), 3).unwrap_err();
        assert!(err.to_string().starts_with("rules: 规则「c」：正则无法编译"), "{err}");
        assert!(store.add(&rule("a", RuleKind::Literal, "p", ""), 3).unwrap_err().to_string().contains("已有名为「a」"));
        store.update(a, &RuleDraft { replacement: "z".into(), ..rule("a2", RuleKind::Literal, "x", "") }, 5).unwrap();
        let r = &store.rules()[0];
        assert_eq!((r.id, r.name.as_str(), r.replacement.as_str(), r.created_at_ms, r.updated_at_ms), (a, "a2", "z", 1, 5));
        assert!(store.update(Uuid::new_v4(), &rule("q", RuleKind::Literal, "q", ""), 5).unwrap_err().to_string().contains("没有 id"));
        store.reorder(&[b, a]).unwrap();
        assert_eq!(RuleStore::open(dir.path()).0.rules()[0].id, b);
        assert!(store.reorder(&[b]).is_err());
        store.remove(b).unwrap();
        assert!(store.remove(b).is_err());
        assert_eq!(RuleStore::open(dir.path()).0.rules().len(), 1);
        // A stored regex that does not compile makes the file unusable (quarantined).
        let bad = format!(
            r#"{{"schema":1,"rules":[{{"id":"{}","name":"x","kind":"regex","pattern":"(","replacement":"","case_sensitive":true,"enabled":true,"created_at_ms":1,"updated_at_ms":1}}]}}"#,
            Uuid::new_v4()
        );
        std::fs::write(dir.path().join(RULES_FILE_NAME), bad).unwrap();
        let (store, notice) = RuleStore::open(dir.path());
        assert!(store.rules().is_empty());
        assert!(notice.unwrap().contains("正则无法编译"));
        assert_eq!(corrupt_files(dir.path(), RULES_FILE_NAME).len(), 1);
    }

    /// Import: replace swaps the list, merge updates same-name rules in place and appends the rest;
    /// a merge over the cap or an invalid draft changes nothing.
    #[test]
    fn rule_import_replaces_or_merges_all_or_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, _) = RuleStore::open(dir.path());
        let keep = store.add(&rule("keep", RuleKind::Literal, "k", "K"), 1).unwrap();
        let update = store.add(&rule("update", RuleKind::Literal, "u", "U"), 1).unwrap();
        store.import(&[rule("update", RuleKind::Regex, "u+", "UU"), rule("new", RuleKind::Literal, "n", "N")], ImportMode::Merge, 9).unwrap();
        let names: Vec<(&str, Uuid)> = store.rules().iter().map(|r| (r.name.as_str(), r.id)).collect();
        assert_eq!(&names[..2], &[("keep", keep), ("update", update)], "same-name rules keep id and position");
        assert_eq!(names[2].0, "new");
        assert_eq!((store.rules()[1].kind, store.rules()[1].replacement.as_str(), store.rules()[1].created_at_ms), (RuleKind::Regex, "UU", 1));
        let before = store.rules().to_vec();
        assert!(store.import(&[rule("bad", RuleKind::Regex, "[", "")], ImportMode::Merge, 9).is_err());
        let many: Vec<RuleDraft> = (0..MAX_RULES).map(|i| rule(&format!("r{i}"), RuleKind::Literal, "p", "")).collect();
        assert!(store.import(&many, ImportMode::Merge, 9).unwrap_err().to_string().contains("最多 200 条"));
        assert_eq!(store.rules(), &before[..], "refused imports change nothing");
        store.import(&many, ImportMode::Replace, 9).unwrap();
        assert_eq!(store.rules().len(), MAX_RULES);
        assert!(store.rules().iter().all(|r| r.id != keep), "replace gives new ids");
        store.import(&[], ImportMode::Replace, 9).unwrap();
        assert!(RuleStore::open(dir.path()).0.rules().is_empty());
    }

    /// docs/dictation.md §23: the readers see what the stores saved and never touch the files; an
    /// unusable file is reported and left exactly where it was (no `.corrupt-` copy).
    #[test]
    fn reading_the_dictionary_and_the_rules_never_moves_the_files() {
        let dir = tempfile::tempdir().unwrap();
        assert!(DictionaryStore::read(dir.path()).unwrap().is_empty() && RuleStore::read(dir.path()).unwrap().is_empty());
        let (mut dictionary, _) = DictionaryStore::open(dir.path());
        let term = dictionary.add(&draft("Voltip", &["沃提普"]), EntrySource::Manual, 1).unwrap();
        let (mut rules, _) = RuleStore::open(dir.path());
        let rule_id = rules.add(&rule("keep", RuleKind::Literal, "k", "K"), 1).unwrap();
        assert_eq!(DictionaryStore::read(dir.path()).unwrap()[0].id, term);
        assert_eq!(RuleStore::read(dir.path()).unwrap()[0].id, rule_id);
        for name in [DICTIONARY_FILE_NAME, RULES_FILE_NAME] {
            std::fs::write(dir.path().join(name), b"{broken").unwrap();
        }
        assert!(DictionaryStore::read(dir.path()).unwrap_err().contains("dictionary.json 无法使用"));
        assert!(RuleStore::read(dir.path()).unwrap_err().contains("rules.json 无法使用"));
        for name in [DICTIONARY_FILE_NAME, RULES_FILE_NAME] {
            assert_eq!(std::fs::read(dir.path().join(name)).unwrap(), b"{broken");
            assert!(corrupt_files(dir.path(), name).is_empty());
        }
    }
}
