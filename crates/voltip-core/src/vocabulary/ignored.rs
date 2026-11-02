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

/// An ignored pair with its place in the order the pairs came onto the list. Places count up in
/// memory for this run of the app only (the file keeps the order itself); a pair that 撤销 puts
/// back goes to its place again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlacedPair {
    /// Where it came onto the list: lower is older.
    pub place: u64,
    /// The pair as stored.
    pub pair: IgnoredPair,
}

/// What a batch 忽略 changed, for its 撤销 (docs/dictation.md §16.10): the pairs it put on the list
/// and the oldest pairs it pushed out to keep [`MAX_IGNORED`], both as stored and with their places.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IgnoredBatch {
    /// The pairs the batch put on the list, normalised and with the batch's time.
    pub added: Vec<PlacedPair>,
    /// The pairs it pushed out, oldest first, with their times and places.
    pub dropped: Vec<PlacedPair>,
}

/// The ignored pairs, oldest first.
#[derive(Debug)]
pub struct IgnoreStore {
    file: ListFile,
    pairs: Vec<IgnoredPair>,
    /// Each pair's place, rising along `pairs` ([`PlacedPair`]).
    places: Vec<u64>,
    /// The place the next pair gets.
    next_place: u64,
}

impl IgnoreStore {
    /// Open `dir/vocabulary-ignored.json`; the second value is the notice for the UI when the file
    /// could not be used (it is moved aside and the list starts empty).
    pub fn open(dir: &Path) -> (Self, Option<String>) {
        let (file, pairs, notice) = ListFile::load(dir, IGNORED_FILE_NAME, IGNORED_SCHEMA, pairs_of, check_stored);
        let places: Vec<u64> = (0..pairs.len() as u64).collect();
        let next_place = places.len() as u64;
        (Self { file, pairs, places, next_place }, notice)
    }

    /// A place for a pair that comes onto the list now.
    fn new_place(&mut self) -> u64 {
        self.next_place += 1;
        self.next_place - 1
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
        let (mut pairs, mut places) = self.without(|p| p.heard == heard && p.polished == polished);
        pairs.push(IgnoredPair { heard, polished, at_ms: now_ms });
        places.push(self.new_place());
        let over = pairs.len().saturating_sub(MAX_IGNORED);
        pairs.drain(..over);
        places.drain(..over);
        self.commit(pairs, places)
    }

    /// Show `heard` → `polished` again (取消忽略); a pair not in the list changes nothing.
    pub fn remove(&mut self, heard: &str, polished: &str) -> Result<(), VocabularyError> {
        let (heard, polished) = validate_pair(heard, polished)?;
        if !self.pairs.iter().any(|p| p.heard == heard && p.polished == polished) {
            return Ok(());
        }
        let (pairs, places) = self.without(|p| p.heard == heard && p.polished == polished);
        self.commit(pairs, places)
    }

    /// Ignore every pair of `pairs` from now on, with one write (a batch 忽略, docs/dictation.md
    /// §16.10): each goes to the end with `now_ms`, once; a pair ignored already keeps its place and
    /// time. Past [`MAX_IGNORED`] the oldest pairs are dropped. Returns what 撤销 needs: the pairs the
    /// batch put on the list and the ones it pushed out. A pair that is not one refuses the whole
    /// batch; nothing changes.
    pub fn add_batch(&mut self, pairs: &[SuggestionPair], now_ms: u64) -> Result<IgnoredBatch, VocabularyError> {
        let mut list = self.pairs.clone();
        let mut places = self.places.clone();
        let first = self.next_place;
        let mut added = Vec::new();
        for pair in pairs {
            let (heard, polished) = validate_pair(&pair.heard, &pair.polished)?;
            if list.iter().any(|p| p.heard == heard && p.polished == polished) {
                continue;
            }
            let stored = IgnoredPair { heard, polished, at_ms: now_ms };
            let place = first + added.len() as u64;
            list.push(stored.clone());
            places.push(place);
            added.push(PlacedPair { place, pair: stored });
        }
        if added.is_empty() {
            return Ok(IgnoredBatch::default());
        }
        let made = added.len() as u64;
        let over = list.len().saturating_sub(MAX_IGNORED);
        let gone: Vec<PlacedPair> = places.drain(..over).zip(list.drain(..over)).map(|(place, pair)| PlacedPair { place, pair }).collect();
        // The batch's own pairs come after the old ones: only a batch past the cap by itself loses some.
        let (dropped, own): (Vec<PlacedPair>, Vec<PlacedPair>) = gone.into_iter().partition(|g| g.place < first);
        added.retain(|a| !own.contains(a));
        self.commit(list, places)?;
        self.next_place = first + made;
        Ok(IgnoredBatch { added, dropped })
    }

    /// 撤销 of a batch 忽略, with one write: the batch's pairs come off the list and the pairs it
    /// pushed out go back to their places with their times, so the list is as it was before the
    /// batch, the oldest first, whichever order the batches are taken back in. A pair taken off
    /// (取消忽略) or ignored again since stays as it is now; past [`MAX_IGNORED`] the oldest pairs
    /// are dropped. Returns whether the list changed.
    pub fn remove_batch(&mut self, batch: &IgnoredBatch) -> Result<bool, VocabularyError> {
        let (mut list, mut places) = self.without_places(|place| batch.added.iter().any(|a| a.place == place));
        for back in &batch.dropped {
            if list.iter().any(|p| p.heard == back.pair.heard && p.polished == back.pair.polished) {
                continue;
            }
            let at = places.partition_point(|&place| place < back.place);
            list.insert(at, back.pair.clone());
            places.insert(at, back.place);
        }
        let over = list.len().saturating_sub(MAX_IGNORED);
        list.drain(..over);
        places.drain(..over);
        if list == self.pairs && places == self.places {
            return Ok(false);
        }
        self.commit(list, places)?;
        Ok(true)
    }

    /// The pairs and places without the pairs `gone` picks.
    fn without(&self, gone: impl Fn(&IgnoredPair) -> bool) -> (Vec<IgnoredPair>, Vec<u64>) {
        self.pairs.iter().zip(&self.places).filter(|(p, _)| !gone(p)).map(|(p, place)| (p.clone(), *place)).unzip()
    }

    /// The pairs and places without the places `gone` picks.
    fn without_places(&self, gone: impl Fn(u64) -> bool) -> (Vec<IgnoredPair>, Vec<u64>) {
        self.pairs.iter().zip(&self.places).filter(|(_, place)| !gone(**place)).map(|(p, place)| (p.clone(), *place)).unzip()
    }

    fn commit(&mut self, pairs: Vec<IgnoredPair>, places: Vec<u64>) -> Result<(), VocabularyError> {
        self.file.save(&IgnoredFile { schema: IGNORED_SCHEMA, pairs: pairs.clone() }).map_err(VocabularyError::Store)?;
        self.pairs = pairs;
        self.places = places;
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

    fn pairs_of(placed: &[PlacedPair]) -> Vec<IgnoredPair> {
        placed.iter().map(|p| p.pair.clone()).collect()
    }

    /// A store opened on a file that holds `pairs`.
    fn store_holding(dir: &Path, pairs: &[IgnoredPair]) -> IgnoreStore {
        std::fs::write(dir.join(IGNORED_FILE_NAME), serde_json::to_vec(&IgnoredFile { schema: IGNORED_SCHEMA, pairs: pairs.to_vec() }).unwrap()).unwrap();
        let (store, notice) = IgnoreStore::open(dir);
        assert!(notice.is_none(), "{notice:?}");
        store
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
        let batch = store.add_batch(&[pair("非", "飞"), pair("维他命", "Voltip"), pair(" 集成 ", "继承"), pair("非", "飞")], 20).unwrap();
        assert_eq!(crate::list_file::saves() - saves, 1, "one write");
        assert_eq!(pairs_of(&batch.added), [stored("非", "飞", 20), stored("集成", "继承", 20)], "what the batch added, trimmed, each once");
        assert_eq!(batch.dropped, [], "room for it");
        let listed: Vec<(&str, u64)> = store.list().iter().map(|p| (p.heard.as_str(), p.at_ms)).collect();
        assert_eq!(listed, [("维他命", 10), ("非", 20), ("集成", 20)]);
        let (reread, _) = IgnoreStore::open(dir.path());
        assert_eq!(reread.list(), store.list(), "on disk");

        let saves = crate::list_file::saves();
        assert_eq!(store.add_batch(&[pair("非", "飞"), pair("维他命", "Voltip")], 30).unwrap(), IgnoredBatch::default(), "ignored already");
        let elsewhere = IgnoredBatch { added: vec![PlacedPair { place: 99, pair: stored("没有", "这组", 20) }], dropped: vec![] };
        assert!(!store.remove_batch(&elsewhere).unwrap());
        assert_eq!(crate::list_file::saves(), saves, "nothing to write either time");

        assert!(store.remove_batch(&batch).unwrap());
        assert_eq!(crate::list_file::saves() - saves, 1, "one write");
        assert_eq!(heard_of(&store), ["维他命"], "the pair ignored before stays");
        let (reread, _) = IgnoreStore::open(dir.path());
        assert_eq!(heard_of(&reread), ["维他命"]);

        // A pair that is not one refuses the batch; nothing changes.
        assert!(store.add_batch(&[pair("集成", "继承"), pair("同", "同")], 40).is_err());
        assert_eq!(heard_of(&store), ["维他命"]);
    }

    /// A batch past [`MAX_IGNORED`] drops the oldest pairs, never its own, and names them for its
    /// 撤销, which puts them back with their times: the list is as it was before the batch.
    #[test]
    fn a_batch_past_the_cap_drops_the_oldest_pairs_and_its_undo_puts_them_back() {
        let dir = tempfile::tempdir().unwrap();
        let full: Vec<IgnoredPair> = (0..MAX_IGNORED).map(|i| stored(&format!("听成{i}"), "正确", 1 + i as u64)).collect();
        let mut store = store_holding(dir.path(), &full);
        let batch = store.add_batch(&[pair("新1", "正确"), pair("新2", "正确"), pair("新3", "正确")], 9000).unwrap();
        assert_eq!(batch.added.len(), 3);
        assert_eq!(pairs_of(&batch.dropped), full[..3], "the three oldest, as they were");
        assert_eq!(store.list().len(), MAX_IGNORED);
        assert_eq!(store.list()[0].heard, "听成3", "the three oldest went");
        assert_eq!(store.list()[MAX_IGNORED - 3..].iter().map(|p| p.heard.as_str()).collect::<Vec<_>>(), ["新1", "新2", "新3"]);

        let saves = crate::list_file::saves();
        assert!(store.remove_batch(&batch).unwrap());
        assert_eq!(crate::list_file::saves() - saves, 1, "one write");
        assert_eq!(store.list(), full.as_slice(), "as before the batch");
        let (reread, _) = IgnoreStore::open(dir.path());
        assert_eq!(reread.list(), full.as_slice(), "on disk");
    }

    /// 撤销 of a batch 忽略 leaves alone what changed since: a pair of the batch taken off (取消忽略)
    /// or ignored again, and a pushed-out pair that was ignored again, stay as they are now.
    #[test]
    fn undo_of_a_batch_ignore_keeps_what_changed_since() {
        let dir = tempfile::tempdir().unwrap();
        let full: Vec<IgnoredPair> = (0..MAX_IGNORED).map(|i| stored(&format!("听成{i}"), "正确", 1 + i as u64)).collect();
        let mut store = store_holding(dir.path(), &full);
        let batch = store.add_batch(&[pair("新1", "正确"), pair("新2", "正确"), pair("新3", "正确")], 9000).unwrap();
        assert_eq!(batch.dropped.len(), 3);
        store.remove("新1", "正确").unwrap();
        store.add("新2", "正确", 9100).unwrap();
        store.add("听成0", "正确", 9200).unwrap();
        // Room for the two pairs that come back, so the cap does not drop the oldest of them again.
        store.remove("听成10", "正确").unwrap();
        assert!(store.remove_batch(&batch).unwrap());
        let list = store.list();
        assert_eq!(list.len(), MAX_IGNORED);
        assert_eq!(
            list[..2].iter().map(|p| (p.heard.as_str(), p.at_ms)).collect::<Vec<_>>(),
            [("听成1", 2), ("听成2", 3)],
            "the pushed-out pairs back in front, those not ignored again"
        );
        assert_eq!(
            list[MAX_IGNORED - 2..].iter().map(|p| (p.heard.as_str(), p.at_ms)).collect::<Vec<_>>(),
            [("新2", 9100), ("听成0", 9200)],
            "ignored again since: kept as now"
        );
        assert!(!list.iter().any(|p| p.heard == "新1" || p.heard == "新3"), "the batch's other pairs are off the list");
        assert!(!list.iter().any(|p| p.heard == "听成10"), "taken off since: stays off");
    }

    /// Two batches on a full list, taken back in either order (completion review, round 2): each
    /// 撤销 puts its pushed-out pairs back where they were, so the list is oldest first again and
    /// the next pair past the cap pushes out the oldest one.
    #[test]
    fn regression_batches_taken_back_in_any_order_keep_the_oldest_first() {
        for older_first in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let full: Vec<IgnoredPair> = (0..MAX_IGNORED).map(|i| stored(&format!("听成{i}"), "正确", 1)).collect();
            let mut store = store_holding(dir.path(), &full);
            let a = store.add_batch(&[pair("新A", "正确")], 2).unwrap();
            let b = store.add_batch(&[pair("新B", "正确")], 3).unwrap();
            assert_eq!((a.dropped[0].pair.heard.as_str(), b.dropped[0].pair.heard.as_str()), ("听成0", "听成1"));
            let (first, second) = if older_first { (&a, &b) } else { (&b, &a) };
            assert!(store.remove_batch(first).unwrap());
            assert!(store.remove_batch(second).unwrap());
            assert_eq!(store.list(), full.as_slice(), "older batch first: {older_first}");
            store.add("再忽略", "正确", 4).unwrap();
            assert_eq!(store.list()[0].heard, "听成1", "the oldest went: {older_first}");
            let (reread, _) = IgnoreStore::open(dir.path());
            assert_eq!(reread.list(), store.list(), "on disk");
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
