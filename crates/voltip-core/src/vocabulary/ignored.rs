//! The pairs the dictionary's learning suggestions leave out because the user chose 忽略
//! (docs/dictation.md §16.10): `<data dir>/vocabulary-ignored.json`, on the shared list persistence
//! of [`crate::list_file`] (validated whole, written atomically, an unusable file moved aside). The
//! list is not part of a backup (§4.6): it only shapes the suggestions of this computer.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::{MAX_TERM_CHARS, SuggestionPair, VocabularyError, clean_line};
use crate::list_file::ListFile;

/// File name of the list inside the app data directory.
pub const IGNORED_FILE_NAME: &str = "vocabulary-ignored.json";
/// On-disk schema of the file.
pub const IGNORED_SCHEMA: u16 = 1;
/// Most pairs kept; past it the oldest is dropped.
pub const MAX_IGNORED: usize = 2000;

/// One pair left out of the suggestions: what the recogniser wrote and what the polished text has
/// instead, as the suggestion named them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IgnoredPair {
    /// What the recogniser wrote.
    pub heard: String,
    /// What the polished text has instead.
    pub polished: String,
    /// When it was ignored, Unix milliseconds.
    pub at_ms: u64,
}

#[derive(Serialize, Deserialize)]
struct IgnoredFile {
    schema: u16,
    pairs: Vec<IgnoredPair>,
}

fn pairs_of(file: IgnoredFile) -> Option<Vec<IgnoredPair>> {
    (file.schema == IGNORED_SCHEMA).then_some(file.pairs)
}

/// A stored list is usable when it keeps the cap, every pair is in its normalised form and none is
/// there twice.
fn check_stored(pairs: &[IgnoredPair]) -> Result<(), String> {
    if pairs.len() > MAX_IGNORED {
        return Err(format!("最多 {MAX_IGNORED} 条"));
    }
    for (i, p) in pairs.iter().enumerate() {
        let (heard, polished) = validate_pair(&p.heard, &p.polished).map_err(|e| e.to_string())?;
        if heard != p.heard || polished != p.polished {
            return Err(format!("「{}」不是规范形式", p.heard));
        }
        if pairs[..i].iter().any(|q| q.heard == p.heard && q.polished == p.polished) {
            return Err(format!("「{}」重复", p.heard));
        }
    }
    Ok(())
}

/// A pair as the suggestions name it: both sides trimmed single lines of 1–[`MAX_TERM_CHARS`]
/// characters, not the same.
pub fn validate_pair(heard: &str, polished: &str) -> Result<(String, String), VocabularyError> {
    let heard = clean_line(heard, "识别结果", MAX_TERM_CHARS).map_err(VocabularyError::Dictionary)?;
    let polished = clean_line(polished, "润色结果", MAX_TERM_CHARS).map_err(VocabularyError::Dictionary)?;
    if heard == polished {
        return Err(VocabularyError::Dictionary(format!("「{heard}」前后相同")));
    }
    Ok((heard, polished))
}

/// The ignored pairs, oldest first.
#[derive(Debug)]
pub struct IgnoreStore {
    file: ListFile,
    pairs: Vec<IgnoredPair>,
}

impl IgnoreStore {
    /// Open `dir/vocabulary-ignored.json`; the second value is the notice for the UI when the file
    /// could not be used (it is moved aside and the list starts empty).
    pub fn open(dir: &Path) -> (Self, Option<String>) {
        let (file, pairs, notice) = ListFile::load(dir, IGNORED_FILE_NAME, IGNORED_SCHEMA, pairs_of, check_stored);
        (Self { file, pairs }, notice)
    }

    /// The pairs, oldest first.
    pub fn list(&self) -> &[IgnoredPair] {
        &self.pairs
    }

    /// Whether `heard` → `polished` is ignored (exactly as the suggestion names it).
    pub fn contains(&self, heard: &str, polished: &str) -> bool {
        validate_pair(heard, polished).is_ok_and(|(heard, polished)| self.pairs.iter().any(|p| p.heard == heard && p.polished == polished))
    }

    /// Ignore `heard` → `polished` from now on; a pair ignored already moves to the end with the
    /// new time. Past [`MAX_IGNORED`] the oldest pairs are dropped.
    pub fn add(&mut self, heard: &str, polished: &str, now_ms: u64) -> Result<(), VocabularyError> {
        let (heard, polished) = validate_pair(heard, polished)?;
        let mut pairs: Vec<IgnoredPair> = self.pairs.iter().filter(|p| p.heard != heard || p.polished != polished).cloned().collect();
        pairs.push(IgnoredPair { heard, polished, at_ms: now_ms });
        let over = pairs.len().saturating_sub(MAX_IGNORED);
        pairs.drain(..over);
        self.commit(pairs)
    }

    /// Show `heard` → `polished` again (取消忽略); a pair not in the list changes nothing.
    pub fn remove(&mut self, heard: &str, polished: &str) -> Result<(), VocabularyError> {
        let (heard, polished) = validate_pair(heard, polished)?;
        if !self.pairs.iter().any(|p| p.heard == heard && p.polished == polished) {
            return Ok(());
        }
        let pairs = self.pairs.iter().filter(|p| p.heard != heard || p.polished != polished).cloned().collect();
        self.commit(pairs)
    }

    /// Ignore every pair of `pairs` from now on, with one write (a batch 忽略, docs/dictation.md
    /// §16.10): each goes to the end with `now_ms`, once; a pair ignored already keeps its place and
    /// time. Past [`MAX_IGNORED`] the oldest pairs are dropped. Returns the pairs the batch put on
    /// the list, as stored. A pair that is not one refuses the whole batch; nothing changes.
    pub fn add_batch(&mut self, pairs: &[SuggestionPair], now_ms: u64) -> Result<Vec<SuggestionPair>, VocabularyError> {
        let mut list = self.pairs.clone();
        let mut added = Vec::new();
        for pair in pairs {
            let (heard, polished) = validate_pair(&pair.heard, &pair.polished)?;
            if list.iter().any(|p| p.heard == heard && p.polished == polished) {
                continue;
            }
            list.push(IgnoredPair { heard: heard.clone(), polished: polished.clone(), at_ms: now_ms });
            added.push(SuggestionPair { heard, polished });
        }
        if added.is_empty() {
            return Ok(added);
        }
        let over = list.len().saturating_sub(MAX_IGNORED);
        list.drain(..over);
        added.retain(|a| list.iter().any(|p| p.heard == a.heard && p.polished == a.polished));
        self.commit(list)?;
        Ok(added)
    }

    /// Show every pair of `pairs` again, with one write (撤销 of a batch 忽略); pairs not in the
    /// list change nothing. Returns whether the list changed. A pair that is not one refuses the
    /// whole batch.
    pub fn remove_batch(&mut self, pairs: &[SuggestionPair]) -> Result<bool, VocabularyError> {
        let gone: Vec<(String, String)> = pairs.iter().map(|p| validate_pair(&p.heard, &p.polished)).collect::<Result<_, _>>()?;
        let list: Vec<IgnoredPair> =
            self.pairs.iter().filter(|p| !gone.iter().any(|(heard, polished)| *heard == p.heard && *polished == p.polished)).cloned().collect();
        if list.len() == self.pairs.len() {
            return Ok(false);
        }
        self.commit(list)?;
        Ok(true)
    }

    fn commit(&mut self, pairs: Vec<IgnoredPair>) -> Result<(), VocabularyError> {
        self.file.save(&IgnoredFile { schema: IGNORED_SCHEMA, pairs: pairs.clone() }).map_err(VocabularyError::Store)?;
        self.pairs = pairs;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heard_of(store: &IgnoreStore) -> Vec<&str> {
        store.list().iter().map(|p| p.heard.as_str()).collect()
    }

    /// docs/dictation.md §16.10: an ignored pair is kept in the file and read back after a
    /// restart; ignoring it again moves it to the end with the new time; 取消忽略 removes it.
    #[test]
    fn ignored_pairs_round_trip_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, notice) = IgnoreStore::open(dir.path());
        assert!(notice.is_none() && store.list().is_empty(), "a missing file is an empty list");
        assert!(!dir.path().join(IGNORED_FILE_NAME).exists(), "opening writes nothing");
        store.add("维他命", "Voltip", 10).unwrap();
        store.add(" 非 ", "飞", 20).unwrap();
        assert!(store.contains("维他命", "Voltip") && store.contains("非", "飞"), "trimmed like the suggestions");
        assert!(!store.contains("维他命", "voltip"), "exactly as the suggestion names it");
        store.add("维他命", "Voltip", 30).unwrap();
        assert_eq!(heard_of(&store), ["非", "维他命"], "ignored again: to the end");
        assert_eq!(store.list()[1].at_ms, 30);

        let (mut again, notice) = IgnoreStore::open(dir.path());
        assert!(notice.is_none());
        assert_eq!(again.list(), store.list(), "read back after a restart");
        again.remove("非", "飞").unwrap();
        again.remove("没有", "这条").unwrap();
        assert_eq!(heard_of(&again), ["维他命"], "取消忽略; an unknown pair changes nothing");
        let (reread, _) = IgnoreStore::open(dir.path());
        assert_eq!(heard_of(&reread), ["维他命"]);
    }

    /// The list keeps [`MAX_IGNORED`] pairs, dropping the oldest; a pair that is not one is refused
    /// and nothing changes.
    #[test]
    fn the_list_keeps_the_newest_pairs_and_refuses_bad_ones() {
        let dir = tempfile::tempdir().unwrap();
        let full: Vec<IgnoredPair> = (0..MAX_IGNORED).map(|i| IgnoredPair { heard: format!("听成{i}"), polished: "正确".into(), at_ms: 1 }).collect();
        std::fs::write(dir.path().join(IGNORED_FILE_NAME), serde_json::to_vec(&IgnoredFile { schema: IGNORED_SCHEMA, pairs: full }).unwrap()).unwrap();
        let (mut store, notice) = IgnoreStore::open(dir.path());
        assert!(notice.is_none(), "{notice:?}");
        store.add(&format!("听成{MAX_IGNORED}"), "正确", 2).unwrap();
        assert_eq!(store.list().len(), MAX_IGNORED);
        assert_eq!(store.list()[0].heard, "听成1", "the oldest went");
        assert_eq!(store.list().last().unwrap().heard, format!("听成{MAX_IGNORED}"));
        let (reread, notice) = IgnoreStore::open(dir.path());
        assert!(notice.is_none());
        assert_eq!(reread.list().len(), MAX_IGNORED);

        let (mut store, _) = IgnoreStore::open(tempfile::tempdir().unwrap().path());
        for (heard, polished) in [("", "x"), ("x", "  "), ("同", "同"), ("a\nb", "c"), (&*"长".repeat(MAX_TERM_CHARS + 1), "短")] {
            assert!(store.add(heard, polished, 1).is_err(), "{heard:?} → {polished:?}");
        }
        assert!(store.list().is_empty());
    }

    fn pair(heard: &str, polished: &str) -> SuggestionPair {
        SuggestionPair { heard: heard.into(), polished: polished.into() }
    }

    /// docs/dictation.md §16.10, a batch 忽略: the pairs go to the end of the list with one write,
    /// each once; a pair ignored already keeps its place and time, and is not one the batch added.
    /// Taking the batch back off the list is one write too, and leaves the other pairs alone.
    #[test]
    fn a_batch_is_ignored_and_taken_back_with_one_write_each() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, _) = IgnoreStore::open(dir.path());
        store.add("维他命", "Voltip", 10).unwrap();
        let saves = crate::list_file::saves();
        let added = store.add_batch(&[pair("非", "飞"), pair("维他命", "Voltip"), pair(" 集成 ", "继承"), pair("非", "飞")], 20).unwrap();
        assert_eq!(crate::list_file::saves() - saves, 1, "one write");
        assert_eq!(added, [pair("非", "飞"), pair("集成", "继承")], "what the batch added, trimmed, each once");
        let listed: Vec<(&str, u64)> = store.list().iter().map(|p| (p.heard.as_str(), p.at_ms)).collect();
        assert_eq!(listed, [("维他命", 10), ("非", 20), ("集成", 20)]);
        let (reread, _) = IgnoreStore::open(dir.path());
        assert_eq!(reread.list(), store.list(), "on disk");

        let saves = crate::list_file::saves();
        assert_eq!(store.add_batch(&[pair("非", "飞"), pair("维他命", "Voltip")], 30).unwrap(), [], "ignored already");
        store.remove_batch(&[pair("没有", "这组")]).unwrap();
        assert_eq!(crate::list_file::saves(), saves, "nothing to write either time");

        store.remove_batch(&added).unwrap();
        assert_eq!(crate::list_file::saves() - saves, 1, "one write");
        assert_eq!(heard_of(&store), ["维他命"], "the pair ignored before stays");
        let (reread, _) = IgnoreStore::open(dir.path());
        assert_eq!(heard_of(&reread), ["维他命"]);

        // A pair that is not one refuses the batch; nothing changes.
        assert!(store.add_batch(&[pair("集成", "继承"), pair("同", "同")], 40).is_err());
        assert!(store.remove_batch(&[pair("维他命", "Voltip"), pair("", "x")]).is_err());
        assert_eq!(heard_of(&store), ["维他命"]);
    }

    /// A batch past [`MAX_IGNORED`] drops the oldest pairs, never its own.
    #[test]
    fn a_batch_past_the_cap_drops_the_oldest_pairs() {
        let dir = tempfile::tempdir().unwrap();
        let full: Vec<IgnoredPair> = (0..MAX_IGNORED).map(|i| IgnoredPair { heard: format!("听成{i}"), polished: "正确".into(), at_ms: 1 }).collect();
        std::fs::write(dir.path().join(IGNORED_FILE_NAME), serde_json::to_vec(&IgnoredFile { schema: IGNORED_SCHEMA, pairs: full }).unwrap()).unwrap();
        let (mut store, _) = IgnoreStore::open(dir.path());
        let added = store.add_batch(&[pair("新1", "正确"), pair("新2", "正确"), pair("新3", "正确")], 2).unwrap();
        assert_eq!(added.len(), 3);
        assert_eq!(store.list().len(), MAX_IGNORED);
        assert_eq!(store.list()[0].heard, "听成3", "the three oldest went");
        assert_eq!(store.list()[MAX_IGNORED - 3..].iter().map(|p| p.heard.as_str()).collect::<Vec<_>>(), ["新1", "新2", "新3"]);
    }

    /// A file that cannot be used is moved aside, never deleted, and the list starts empty.
    #[test]
    fn an_unusable_file_is_moved_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(IGNORED_FILE_NAME);
        let twice = r#"{"schema":1,"pairs":[{"heard":"a","polished":"b","at_ms":1},{"heard":"a","polished":"b","at_ms":2}]}"#;
        std::fs::write(&path, twice).unwrap();
        let (store, notice) = IgnoreStore::open(dir.path());
        assert!(store.list().is_empty());
        let notice = notice.expect("a notice");
        assert!(notice.contains(IGNORED_FILE_NAME), "{notice}");
        assert!(!path.exists());
        let aside: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        assert!(aside.iter().any(|n| n.starts_with("vocabulary-ignored.json.corrupt-")), "{aside:?}");
    }
}
