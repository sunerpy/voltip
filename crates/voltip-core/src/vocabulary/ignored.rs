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

/// A batch 忽略, for its 撤销 (docs/dictation.md §16.10): the batch's number in the store's
/// journal and how many pairs it put on the list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IgnoredBatch {
    /// Its number in the journal, for [`IgnoreStore::remove_batch`].
    pub id: u64,
    /// The pairs it put on the list (pairs ignored already are not counted).
    pub added: usize,
}

/// One change to the list after [`IgnoreStore::base`], kept while a batch 撤销 may need it.
#[derive(Clone, Debug)]
enum Change {
    /// 忽略 of one pair.
    Add { heard: String, polished: String, at_ms: u64 },
    /// 取消忽略 of one pair.
    Remove { heard: String, polished: String },
    /// A batch 忽略, and whether its 撤销 took it back.
    Batch { id: u64, pairs: Vec<(String, String)>, at_ms: u64, undone: bool },
}

impl Change {
    fn live_batch(&self) -> bool {
        matches!(self, Self::Batch { undone: false, .. })
    }
}

/// Most changes kept for 撤销; past it the oldest batch can no longer be taken back.
const MAX_CHANGES: usize = 10_000;

/// `list` after `change`, as [`IgnoreStore`] applies it: a pair 忽略 again goes to the end with
/// its new time, a batch skips the pairs on the list already, and past [`MAX_IGNORED`] the oldest
/// pairs are dropped.
fn apply(list: &mut Vec<IgnoredPair>, change: &Change) {
    match change {
        Change::Add { heard, polished, at_ms } => {
            list.retain(|p| p.heard != *heard || p.polished != *polished);
            list.push(IgnoredPair { heard: heard.clone(), polished: polished.clone(), at_ms: *at_ms });
        }
        Change::Remove { heard, polished } => list.retain(|p| p.heard != *heard || p.polished != *polished),
        Change::Batch { undone: true, .. } => {}
        Change::Batch { pairs, at_ms, undone: false, .. } => {
            let mut on: std::collections::HashSet<(String, String)> = list.iter().map(|p| (p.heard.clone(), p.polished.clone())).collect();
            for (heard, polished) in pairs {
                if on.insert((heard.clone(), polished.clone())) {
                    list.push(IgnoredPair { heard: heard.clone(), polished: polished.clone(), at_ms: *at_ms });
                }
            }
        }
    }
    let over = list.len().saturating_sub(MAX_IGNORED);
    list.drain(..over);
}

/// The ignored pairs, oldest first.
#[derive(Debug)]
pub struct IgnoreStore {
    file: ListFile,
    pairs: Vec<IgnoredPair>,
    /// The list before `journal`: applying the journal to it gives `pairs`. While no batch can be
    /// taken back the journal is empty and this is the list itself.
    base: Vec<IgnoredPair>,
    /// The changes since `base`, oldest first, in memory only: 撤销 of a batch marks it undone and
    /// applies the rest again, so the list is what it would be had the batch never happened.
    journal: Vec<Change>,
    /// The number the next batch gets.
    next_batch: u64,
}

impl IgnoreStore {
    /// Open `dir/vocabulary-ignored.json`; the second value is the notice for the UI when the file
    /// could not be used (it is moved aside and the list starts empty).
    pub fn open(dir: &Path) -> (Self, Option<String>) {
        let (file, pairs, notice) = ListFile::load(dir, IGNORED_FILE_NAME, IGNORED_SCHEMA, pairs_of, check_stored);
        (Self { file, base: pairs.clone(), pairs, journal: Vec::new(), next_batch: 0 }, notice)
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
        self.change(Change::Add { heard, polished, at_ms: now_ms })
    }

    /// Show `heard` → `polished` again (取消忽略); a pair not in the list changes nothing.
    pub fn remove(&mut self, heard: &str, polished: &str) -> Result<(), VocabularyError> {
        let (heard, polished) = validate_pair(heard, polished)?;
        if !self.pairs.iter().any(|p| p.heard == heard && p.polished == polished) {
            return Ok(());
        }
        self.change(Change::Remove { heard, polished })
    }

    /// Ignore every pair of `pairs` from now on, with one write (a batch 忽略, docs/dictation.md
    /// §16.10): each goes to the end with `now_ms`, once; a pair ignored already keeps its place and
    /// time. Past [`MAX_IGNORED`] the oldest pairs are dropped. Returns what 撤销 needs. A pair that
    /// is not one refuses the whole batch; nothing changes.
    pub fn add_batch(&mut self, pairs: &[SuggestionPair], now_ms: u64) -> Result<IgnoredBatch, VocabularyError> {
        let pairs: Vec<(String, String)> = pairs.iter().map(|p| validate_pair(&p.heard, &p.polished)).collect::<Result<_, _>>()?;
        let id = self.next_batch;
        let change = Change::Batch { id, pairs, at_ms: now_ms, undone: false };
        let mut list = self.pairs.clone();
        apply(&mut list, &change);
        let added = list.iter().filter(|p| p.at_ms == now_ms && !self.pairs.contains(p)).count();
        if list != self.pairs {
            self.commit(list)?;
        }
        self.next_batch += 1;
        self.journal.push(change);
        self.trim_journal();
        Ok(IgnoredBatch { id, added })
    }

    /// 撤销 of a batch 忽略, with one write: the list becomes what it would be had the batch never
    /// happened, every other change applied again in its order, whichever order the batches are
    /// taken back in. A pair taken off (取消忽略) or ignored again since stays as it is now, and the
    /// oldest pairs past [`MAX_IGNORED`] are the ones dropped. `None` when the batch is not one this
    /// run of the app can take back (taken back already, or too old); otherwise whether the list
    /// changed.
    pub fn remove_batch(&mut self, batch: &IgnoredBatch) -> Result<Option<bool>, VocabularyError> {
        let Some(at) = self.journal.iter().position(|c| matches!(c, Change::Batch { id, undone: false, .. } if *id == batch.id)) else {
            return Ok(None);
        };
        let mut journal = self.journal.clone();
        if let Change::Batch { pairs, undone, .. } = &mut journal[at] {
            // An undone batch applies nothing: its pairs are not needed any more.
            pairs.clear();
            *undone = true;
        }
        let mut list = self.base.clone();
        for change in &journal {
            apply(&mut list, change);
        }
        let changed = list != self.pairs;
        if changed {
            self.commit(list)?;
        }
        self.journal = journal;
        self.trim_journal();
        Ok(Some(changed))
    }

    /// Apply `change` to the list, with one write, and keep it while a batch may be taken back.
    fn change(&mut self, change: Change) -> Result<(), VocabularyError> {
        let mut list = self.pairs.clone();
        apply(&mut list, &change);
        if list != self.pairs {
            self.commit(list)?;
        }
        self.journal.push(change);
        self.trim_journal();
        Ok(())
    }

    /// Fold into `base` the changes no batch 撤销 can need: everything before the oldest batch
    /// that can still be taken back; past [`super::MAX_UNDOS`] such batches or [`MAX_CHANGES`]
    /// changes, the oldest batch too, which can then no longer be taken back.
    fn trim_journal(&mut self) {
        let first = self.journal.iter().position(Change::live_batch).unwrap_or(self.journal.len());
        self.settle(first);
        while self.journal.iter().filter(|c| c.live_batch()).count() > super::MAX_UNDOS || self.journal.len() > MAX_CHANGES {
            let next = self.journal.iter().skip(1).position(Change::live_batch).map_or(self.journal.len(), |i| i + 1);
            self.settle(next);
        }
    }

    /// Fold the first `n` changes of the journal into `base`.
    fn settle(&mut self, n: usize) {
        for change in self.journal.drain(..n) {
            apply(&mut self.base, &change);
        }
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

    fn stored(heard: &str, polished: &str, at_ms: u64) -> IgnoredPair {
        IgnoredPair { heard: heard.into(), polished: polished.into(), at_ms }
    }

    /// A store opened on a file that holds `pairs`.
    fn store_holding(dir: &Path, pairs: &[IgnoredPair]) -> IgnoreStore {
        std::fs::write(dir.join(IGNORED_FILE_NAME), serde_json::to_vec(&IgnoredFile { schema: IGNORED_SCHEMA, pairs: pairs.to_vec() }).unwrap()).unwrap();
        let (store, notice) = IgnoreStore::open(dir);
        assert!(notice.is_none(), "{notice:?}");
        store
    }

    /// A full list as a file holds it: `听成0` … `听成1999`, oldest first, ignored at 1 + i.
    fn full_list() -> Vec<IgnoredPair> {
        (0..MAX_IGNORED).map(|i| stored(&format!("听成{i}"), "正确", 1 + i as u64)).collect()
    }

    /// docs/dictation.md §16.10, a batch 忽略: the pairs go to the end of the list with one write,
    /// each once; a pair ignored already keeps its place and time, and is not one the batch added.
    /// Taking the batch back off the list is one write too, once, and leaves the other pairs alone.
    #[test]
    fn a_batch_is_ignored_and_taken_back_with_one_write_each() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, _) = IgnoreStore::open(dir.path());
        store.add("维他命", "Voltip", 10).unwrap();
        let saves = crate::list_file::saves();
        let batch = store.add_batch(&[pair("非", "飞"), pair("维他命", "Voltip"), pair(" 集成 ", "继承"), pair("非", "飞")], 20).unwrap();
        assert_eq!(crate::list_file::saves() - saves, 1, "one write");
        assert_eq!(batch.added, 2, "what the batch added, each once");
        let listed: Vec<(&str, u64)> = store.list().iter().map(|p| (p.heard.as_str(), p.at_ms)).collect();
        assert_eq!(listed, [("维他命", 10), ("非", 20), ("集成", 20)], "trimmed, in order");
        let (reread, _) = IgnoreStore::open(dir.path());
        assert_eq!(reread.list(), store.list(), "on disk");

        let saves = crate::list_file::saves();
        let again = store.add_batch(&[pair("非", "飞"), pair("维他命", "Voltip")], 30).unwrap();
        assert_eq!(again.added, 0, "ignored already");
        assert_eq!(store.remove_batch(&IgnoredBatch { id: 99, added: 1 }).unwrap(), None, "not a batch of this store");
        assert_eq!(store.remove_batch(&again).unwrap(), Some(false), "it added nothing");
        assert_eq!(crate::list_file::saves(), saves, "nothing to write any of these times");

        assert_eq!(store.remove_batch(&batch).unwrap(), Some(true));
        assert_eq!(crate::list_file::saves() - saves, 1, "one write");
        assert_eq!(heard_of(&store), ["维他命"], "the pair ignored before stays");
        assert_eq!(store.remove_batch(&batch).unwrap(), None, "once");
        let (reread, _) = IgnoreStore::open(dir.path());
        assert_eq!(heard_of(&reread), ["维他命"]);

        // A pair that is not one refuses the batch; nothing changes.
        assert!(store.add_batch(&[pair("集成", "继承"), pair("同", "同")], 40).is_err());
        assert_eq!(heard_of(&store), ["维他命"]);
    }

    /// A batch past [`MAX_IGNORED`] drops the oldest pairs, never its own; its 撤销 puts them back
    /// with their times: the list is as it was before the batch.
    #[test]
    fn a_batch_past_the_cap_drops_the_oldest_pairs_and_its_undo_puts_them_back() {
        let dir = tempfile::tempdir().unwrap();
        let full = full_list();
        let mut store = store_holding(dir.path(), &full);
        let batch = store.add_batch(&[pair("新1", "正确"), pair("新2", "正确"), pair("新3", "正确")], 9000).unwrap();
        assert_eq!(batch.added, 3);
        assert_eq!(store.list().len(), MAX_IGNORED);
        assert_eq!(store.list()[0].heard, "听成3", "the three oldest went");
        assert_eq!(store.list()[MAX_IGNORED - 3..].iter().map(|p| p.heard.as_str()).collect::<Vec<_>>(), ["新1", "新2", "新3"]);

        let saves = crate::list_file::saves();
        assert_eq!(store.remove_batch(&batch).unwrap(), Some(true));
        assert_eq!(crate::list_file::saves() - saves, 1, "one write");
        assert_eq!(store.list(), full.as_slice(), "as before the batch");
        let (reread, _) = IgnoreStore::open(dir.path());
        assert_eq!(reread.list(), full.as_slice(), "on disk");
    }

    /// 撤销 of a batch 忽略 leaves the list as it would be had the batch never happened: what the
    /// user did since stays done. A pair of the batch taken off (取消忽略) stays off, one ignored
    /// again stays with its new time, and the pairs those later changes push past the cap are the
    /// oldest ones, as they would have been.
    #[test]
    fn undo_of_a_batch_ignore_keeps_what_changed_since() {
        let dir = tempfile::tempdir().unwrap();
        let full = full_list();
        let mut store = store_holding(dir.path(), &full);
        let batch = store.add_batch(&[pair("新1", "正确"), pair("新2", "正确"), pair("新3", "正确")], 9000).unwrap();
        store.remove("新1", "正确").unwrap();
        store.add("新2", "正确", 9100).unwrap();
        store.add("听成0", "正确", 9200).unwrap();
        store.remove("听成10", "正确").unwrap();
        assert_eq!(store.remove_batch(&batch).unwrap(), Some(true));

        // The same changes without the batch: 新2 and 听成0 go to the end and push the two oldest
        // out (听成0 first, then 听成1), and 听成10 is taken off.
        let mut without = full[2..].to_vec();
        without.retain(|p| p.heard != "听成10");
        without.push(stored("新2", "正确", 9100));
        without.push(stored("听成0", "正确", 9200));
        assert_eq!(store.list(), without.as_slice());
        assert!(!store.list().iter().any(|p| p.heard == "新1" || p.heard == "新3"), "the batch's other pairs are off the list");
    }

    /// Two batches on a full list, taken back in either order (completion review, round 2): the
    /// list is the one as opened again, and the next pair past the cap pushes out the oldest one.
    #[test]
    fn regression_batches_taken_back_in_any_order_keep_the_oldest_first() {
        for older_first in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let full: Vec<IgnoredPair> = (0..MAX_IGNORED).map(|i| stored(&format!("听成{i}"), "正确", 1)).collect();
            let mut store = store_holding(dir.path(), &full);
            let a = store.add_batch(&[pair("新A", "正确")], 2).unwrap();
            let b = store.add_batch(&[pair("新B", "正确")], 3).unwrap();
            assert_eq!(store.list()[0].heard, "听成2", "each pushed one out");
            let (first, second) = if older_first { (&a, &b) } else { (&b, &a) };
            assert_eq!(store.remove_batch(first).unwrap(), Some(true));
            assert_eq!(store.remove_batch(second).unwrap(), Some(true));
            assert_eq!(store.list(), full.as_slice(), "older batch first: {older_first}");
            store.add("再忽略", "正确", 4).unwrap();
            assert_eq!(store.list()[0].heard, "听成1", "the oldest went: {older_first}");
            let (reread, _) = IgnoreStore::open(dir.path());
            assert_eq!(reread.list(), store.list(), "on disk");
        }
    }

    /// The journal 撤销 replays is bounded: past [`crate::vocabulary::MAX_UNDOS`] batches the oldest
    /// can no longer be taken back, and the others still can.
    #[test]
    fn the_oldest_batches_expire_past_the_undo_limit() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, _) = IgnoreStore::open(dir.path());
        let made: Vec<IgnoredBatch> =
            (0..=crate::vocabulary::MAX_UNDOS).map(|i| store.add_batch(&[pair(&format!("批{i}"), "正确")], i as u64).unwrap()).collect();
        assert_eq!(store.remove_batch(&made[0]).unwrap(), None, "the oldest expired");
        assert_eq!(store.remove_batch(&made[1]).unwrap(), Some(true));
        assert_eq!(store.list().len(), crate::vocabulary::MAX_UNDOS, "101 pairs, one batch taken back");
        assert_eq!(store.list()[0].heard, "批0", "an expired batch stays");
        assert!(store.journal.len() <= crate::vocabulary::MAX_UNDOS, "{}", store.journal.len());
    }

    /// Eleven batches of 200 on a full list, the eleventh pushing out the first one's pairs, taken
    /// back in several orders (completion review, round 3). After each 撤销 the list is the one the
    /// batches left over would have made on their own; once all are taken back it is the list as
    /// opened. An undone batch's pairs never come back with a later batch's 撤销.
    #[test]
    fn regression_eleven_full_batches_taken_back_in_any_order_leave_the_list_as_opened() {
        let full: Vec<IgnoredPair> = (0..MAX_IGNORED).map(|i| stored(&format!("听成{i}"), "正确", 1)).collect();
        let batches: Vec<Vec<SuggestionPair>> = (0..11).map(|b| (0..200).map(|i| pair(&format!("批{b}-{i}"), "正确")).collect()).collect();
        // What the batches still standing make of the list as opened: each pair not on it goes to
        // the end, then the oldest go past the cap.
        let reference = |standing: &[usize]| {
            let mut list = full.clone();
            for &b in standing {
                for p in &batches[b] {
                    if !list.iter().any(|q| q.heard == p.heard && q.polished == p.polished) {
                        list.push(stored(&p.heard, &p.polished, 10 + b as u64));
                    }
                }
                let over = list.len().saturating_sub(MAX_IGNORED);
                list.drain(..over);
            }
            list
        };
        let orders: [Vec<usize>; 3] = [
            std::iter::once(0).chain((1..11).rev()).collect(), // the review's: 1, 11, 10, …, 2
            (0..11).collect(),
            (0..11).rev().collect(),
        ];
        for order in orders {
            let dir = tempfile::tempdir().unwrap();
            let mut store = store_holding(dir.path(), &full);
            let made: Vec<IgnoredBatch> = batches.iter().enumerate().map(|(b, pairs)| store.add_batch(pairs, 10 + b as u64).unwrap()).collect();
            assert_eq!(store.list(), reference(&(0..11).collect::<Vec<_>>()).as_slice());
            let mut standing: Vec<usize> = (0..11).collect();
            for b in order.iter().copied() {
                // The first batch's pairs are pushed out by the eleventh already: taking it back
                // first may change nothing, and is still done.
                assert!(store.remove_batch(&made[b]).unwrap().is_some(), "batch {b}, order {order:?}");
                standing.retain(|&s| s != b);
                assert_eq!(store.list(), reference(&standing).as_slice(), "after taking back batch {b}, order {order:?}");
            }
            assert_eq!(store.list(), full.as_slice(), "all taken back, order {order:?}");
            let (reread, _) = IgnoreStore::open(dir.path());
            assert_eq!(reread.list(), full.as_slice(), "on disk");
        }
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
