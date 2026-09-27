//! The literal matcher behind the dictionary corrections and the literal replacement rules
//! (docs/dictation.md §16.2): aho-corasick over every pattern at once, ASCII case folding on
//! request, word boundaries only for the scripts that separate words with spaces, and the
//! leftmost-longest non-overlapping matches among those that respect the boundaries.

use aho_corasick::{AhoCorasick, MatchKind};

/// A "word character" for the boundary rule: `_` or a Unicode letter / digit, except the scripts
/// written without spaces between words (Han, kana, Hangul, Thai, Lao, Khmer, Myanmar, Bopomofo).
/// A pattern that starts (ends) with a word character only matches where the character before
/// (after) it is not one — `cat` never hits `concatenate`, but does hit `我用cat命令`.
pub fn is_word_char(c: char) -> bool {
    c == '_' || (c.is_alphanumeric() && !is_unspaced_script(c))
}

/// Scripts that do not put spaces between words; their characters never form a boundary.
fn is_unspaced_script(c: char) -> bool {
    matches!(
        c as u32,
        0x0E00..=0x0EFF // Thai, Lao
            | 0x1000..=0x109F // Myanmar
            | 0x1100..=0x11FF // Hangul Jamo
            | 0x1780..=0x17FF // Khmer
            | 0x2E80..=0x2FDF // CJK radicals, Kangxi radicals
            | 0x3005..=0x3007 // 々 〆 〇
            | 0x3040..=0x30FF // Hiragana, Katakana
            | 0x3100..=0x312F // Bopomofo
            | 0x3130..=0x318F // Hangul compatibility Jamo
            | 0x31A0..=0x31FF // Bopomofo extended, Katakana phonetic extensions
            | 0x3400..=0x4DBF // CJK extension A
            | 0x4E00..=0x9FFF // CJK unified ideographs
            | 0xA960..=0xA97F // Hangul Jamo extended-A
            | 0xAC00..=0xD7FF // Hangul syllables, Jamo extended-B
            | 0xF900..=0xFAFF // CJK compatibility ideographs
            | 0xFF66..=0xFF9F // halfwidth Katakana
            | 0xFFA0..=0xFFDC // halfwidth Hangul
            | 0x20000..=0x3134F // CJK extensions B–G
    )
}

/// Many literal patterns, each mapped to a target (a dictionary entry, or the one rule).
pub(crate) struct LiteralMatcher {
    ac: AhoCorasick,
    /// Per pattern: (needs a boundary before, needs one after).
    edges: Vec<(bool, bool)>,
    /// Per pattern: its target.
    targets: Vec<usize>,
}

impl LiteralMatcher {
    /// Build from `(pattern, target)` pairs; `ascii_case_insensitive` folds A–Z only. Empty
    /// patterns are skipped (they never match).
    pub(crate) fn new(patterns: &[(String, usize)], ascii_case_insensitive: bool) -> Result<Self, String> {
        let patterns: Vec<&(String, usize)> = patterns.iter().filter(|(p, _)| !p.is_empty()).collect();
        let ac = AhoCorasick::builder()
            .ascii_case_insensitive(ascii_case_insensitive)
            .match_kind(MatchKind::Standard)
            .build(patterns.iter().map(|(p, _)| p.as_str()))
            .map_err(|e| e.to_string())?;
        let edges = patterns.iter().map(|(p, _)| (p.chars().next().is_some_and(is_word_char), p.chars().next_back().is_some_and(is_word_char))).collect();
        let targets = patterns.iter().map(|(_, t)| *t).collect();
        Ok(Self { ac, edges, targets })
    }

    /// The accepted matches as `(start, end, target)`, left to right, non-overlapping: among the
    /// matches that respect their boundaries, the leftmost, and at one position the longest.
    pub(crate) fn find_all(&self, text: &str) -> Result<Vec<(usize, usize, usize)>, String> {
        let mut candidates: Vec<(usize, usize, usize)> = Vec::new();
        for m in self.ac.try_find_overlapping_iter(text).map_err(|e| e.to_string())? {
            let pattern = m.pattern().as_usize();
            if self.respects_boundaries(text, m.start(), m.end(), pattern) {
                candidates.push((m.start(), m.end(), pattern));
            }
        }
        candidates.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
        let mut accepted = Vec::new();
        let mut last_end = 0;
        for (start, end, pattern) in candidates {
            if start >= last_end {
                accepted.push((start, end, self.targets[pattern]));
                last_end = end;
            }
        }
        Ok(accepted)
    }

    /// Replace every accepted match with `replacement(target)`. Returns the text and, per match
    /// that changed something, its target (a match already spelled like its replacement — a
    /// correctly cased term, say — is no change). Errs when the output would pass `limit` bytes.
    pub(crate) fn replace<'r>(&self, text: &str, replacement: impl Fn(usize) -> &'r str, limit: usize) -> Result<(String, Vec<usize>), String> {
        let mut out = String::with_capacity(text.len());
        let mut changed = Vec::new();
        let mut last = 0;
        for (start, end, target) in self.find_all(text)? {
            let with = replacement(target);
            out.push_str(text.get(last..start).unwrap_or_default());
            out.push_str(with);
            if text.get(start..end) != Some(with) {
                changed.push(target);
            }
            last = end;
            if out.len() > limit {
                return Err(format!("output over {limit} bytes"));
            }
        }
        out.push_str(text.get(last..).unwrap_or_default());
        if out.len() > limit {
            return Err(format!("output over {limit} bytes"));
        }
        Ok((out, changed))
    }

    fn respects_boundaries(&self, text: &str, start: usize, end: usize, pattern: usize) -> bool {
        let (before, after) = self.edges.get(pattern).copied().unwrap_or((false, false));
        // A match always starts and ends on a character boundary (patterns are valid UTF-8 and
        // only ASCII bytes are folded); `get` keeps that an invariant rather than a panic.
        let (Some(head), Some(tail)) = (text.get(..start), text.get(end..)) else { return false };
        !(before && head.chars().next_back().is_some_and(is_word_char)) && !(after && tail.chars().next().is_some_and(is_word_char))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matcher(patterns: &[&str], ci: bool) -> LiteralMatcher {
        let pairs: Vec<(String, usize)> = patterns.iter().enumerate().map(|(i, p)| ((*p).to_owned(), i)).collect();
        LiteralMatcher::new(&pairs, ci).unwrap()
    }

    fn replace(m: &LiteralMatcher, text: &str, with: &[&str]) -> String {
        m.replace(text, |t| with[t], 1 << 20).unwrap().0
    }

    #[test]
    fn word_characters_exclude_the_unspaced_scripts() {
        for c in ['a', 'Z', '0', '_', 'é', 'ж', 'ω', 'Ａ'] {
            assert!(is_word_char(c), "{c}");
        }
        for c in ['中', '文', 'あ', 'カ', '한', 'ก', ' ', ',', '，', '。', '-', '\'', '々', '𠀀'] {
            assert!(!is_word_char(c), "{c}");
        }
    }

    /// Latin patterns match whole words only; CJK patterns match anywhere; mixed CJK / Latin text
    /// without spaces still has boundaries between the scripts.
    #[test]
    fn boundaries_apply_to_spaced_scripts_only() {
        let m = matcher(&["cat"], false);
        assert_eq!(replace(&m, "concatenate cat category", &["dog"]), "concatenate dog category");
        assert_eq!(replace(&m, "我用cat命令", &["dog"]), "我用dog命令");
        assert_eq!(replace(&m, "cat's cat_x (cat)", &["dog"]), "dog's cat_x (dog)", "an apostrophe is a boundary, an underscore is not");
        let m = matcher(&["提成"], false);
        assert_eq!(replace(&m, "提成在Teams里面，提成", &["集成"]), "集成在Teams里面，集成");
        // Only the word-character side of a pattern needs a boundary.
        let m = matcher(&["给他push"], false);
        assert_eq!(replace(&m, "我给他push了", &["git push"]), "我git push了");
        assert_eq!(replace(&m, "给他pushed", &["git push"]), "给他pushed");
        let m = matcher(&["C++"], false);
        assert_eq!(replace(&m, "学C++和C++11", &["cpp"]), "学cpp和cpp11", "`+` is no word character, so `C++11` matches too");
        assert_eq!(replace(&m, "ABC++", &["cpp"]), "ABC++", "but `C` still needs a boundary before it");
    }

    #[test]
    fn ascii_case_folding_is_opt_in_and_ascii_only() {
        let m = matcher(&["open ai"], true);
        assert_eq!(replace(&m, "Open AI and OPEN ai", &["OpenAI"]), "OpenAI and OpenAI");
        let m = matcher(&["open ai"], false);
        assert_eq!(replace(&m, "Open AI and open ai", &["OpenAI"]), "Open AI and OpenAI");
        let m = matcher(&["école"], true);
        assert_eq!(replace(&m, "École école", &["ecole"]), "École ecole", "non-ASCII letters compare as written");
    }

    /// One pass, leftmost first, longest at a position; a match rejected by its boundary does not
    /// hide a shorter valid one at the same place; replaced text is never rescanned.
    #[test]
    fn leftmost_longest_non_overlapping_and_no_rescan() {
        let m = matcher(&["good", "good idea", "idea"], true);
        assert_eq!(replace(&m, "a good idea, good ideas", &["G", "GI", "I"]), "a GI, G ideas");
        let m = matcher(&["foo", "foo bar"], false);
        assert_eq!(replace(&m, "foo barx", &["F", "FB"]), "F barx", "the longer match fails its boundary; the shorter one stands");
        let m = matcher(&["ab", "bc"], false);
        assert_eq!(replace(&m, "xabcx ab bc", &["1", "2"]), "xabcx 1 2");
        let m = matcher(&["沃提普", "提"], false);
        assert_eq!(replace(&m, "沃提普提", &["Voltip", "T"]), "VoltipT", "longest wins at a position, then continue after it");
        let m = matcher(&["a"], false);
        assert_eq!(replace(&m, "a a", &["a a"]), "a a a a", "the replacement is not scanned again");
        assert_eq!(m.find_all("").unwrap(), Vec::<(usize, usize, usize)>::new());
    }

    #[test]
    fn empty_patterns_are_ignored_and_the_limit_is_enforced() {
        let m = LiteralMatcher::new(&[(String::new(), 0), ("x".into(), 1)], false).unwrap();
        assert_eq!(m.find_all("x y x").unwrap(), vec![(0, 1, 1), (4, 5, 1)]);
        let m = matcher(&["x"], false);
        assert!(m.replace("x x x", |_| "0123456789", 20).unwrap_err().contains("20 bytes"));
        assert_eq!(m.replace("x x x", |_| "01", 20).unwrap(), ("01 01 01".to_owned(), vec![0, 0, 0]));
        let m = matcher(&["teams"], true);
        assert_eq!(m.replace("teams Teams", |_| "Teams", 100).unwrap(), ("Teams Teams".to_owned(), vec![0]), "an already right spelling is no change");
        assert!(m.replace(&"y".repeat(30), |_| "", 20).unwrap_err().contains("20 bytes"), "an over-long pass-through is refused too");
    }
}
