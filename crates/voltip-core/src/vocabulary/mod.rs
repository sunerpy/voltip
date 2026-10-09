//! Personal dictionary and replacement rules (docs/dictation.md §16).
//!
//! Two user-maintained tables the pipeline really uses. Dictionary entries correct known
//! mis-hearings right after recognition and travel as a glossary to the cloud recogniser and the
//! LLM clean-up; replacement rules rewrite the final text, in order, before it is injected. This
//! module owns the wire types, the validation, the compiled matchers ([`Vocabulary`]), the stores
//! (`dictionary.json`, `rules.json`) and the TOML exchange format of the rules.
//!
//! Nothing in here may stop a take: [`Vocabulary::correct`] and [`Vocabulary::apply_rules`] never
//! fail — a problem (text over [`MAX_TEXT_BYTES`], a rule that blows the text up, a panic) comes
//! back in [`Step::error`] together with the unchanged input.

mod matcher;
pub mod packs;
mod store;
mod toml_io;

use std::collections::BTreeMap;
use std::panic::AssertUnwindSafe;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use matcher::LiteralMatcher;
pub use matcher::is_word_char;
pub use store::{DICTIONARY_FILE_NAME, DictionaryStore, RULES_FILE_NAME, RuleStore, VOCABULARY_SCHEMA};
pub use toml_io::{RULES_TOML_VERSION, export_rules_toml, parse_rules_toml};

/// Most dictionary entries kept.
pub const MAX_DICTIONARY_ENTRIES: usize = 500;
/// Longest `term` / `heard_as` variant, in characters.
pub const MAX_TERM_CHARS: usize = 64;
/// Most mis-hearings one entry lists.
pub const MAX_HEARD_AS: usize = 10;
/// Most replacement rules kept.
pub const MAX_RULES: usize = 200;
/// Longest rule name, in characters.
pub const MAX_RULE_NAME_CHARS: usize = 64;
/// Longest rule pattern, in characters.
pub const MAX_PATTERN_CHARS: usize = 256;
/// Longest rule replacement, in characters.
pub const MAX_REPLACEMENT_CHARS: usize = 256;
/// Compiled program size one regex rule may take (`RegexBuilder::size_limit`).
pub const REGEX_SIZE_LIMIT: usize = 256 * 1024;
/// Lazy-DFA cache one regex rule may take (`RegexBuilder::dfa_size_limit`).
pub const REGEX_DFA_SIZE_LIMIT: usize = 256 * 1024;
/// Largest text the dictionary and the rules work on, and the largest they may produce.
pub const MAX_TEXT_BYTES: usize = 64 * 1024;
/// Most terms in the glossary sent to the recogniser and the refiner (dictionary order = priority).
pub const MAX_GLOSSARY_TERMS: usize = 200;
/// Longest glossary, in characters, counting the `", "` separators of [`glossary_prompt`].
pub const MAX_GLOSSARY_CHARS: usize = 1000;
/// Largest TOML text [`parse_rules_toml`] accepts.
pub const MAX_TOML_BYTES: usize = 256 * 1024;

/// Separator of the glossary prompt sent to the cloud recogniser.
pub const GLOSSARY_SEPARATOR: &str = ", ";

/// Where a dictionary entry came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EntrySource {
    /// Typed on the dictionary page.
    Manual,
    /// Added from a history entry ("加入词典").
    History {
        /// The history entry it was added from (may have been deleted since).
        history_id: Uuid,
    },
}

/// One personal dictionary entry: the right spelling and the ways the recogniser gets it wrong.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DictionaryEntry {
    /// Stable id.
    pub id: Uuid,
    /// The right spelling (1–[`MAX_TERM_CHARS`] characters, trimmed, no control characters).
    pub term: String,
    /// Mis-hearings replaced by `term` right after recognition (0–[`MAX_HEARD_AS`]).
    #[serde(default)]
    pub heard_as: Vec<String>,
    /// Off: neither corrected nor in the glossary.
    pub enabled: bool,
    /// Manual or from a history entry.
    pub source: EntrySource,
    /// Unix milliseconds.
    pub created_at_ms: u64,
    /// Unix milliseconds of the last change.
    pub updated_at_ms: u64,
}

/// What the UI sends to create or change a dictionary entry (`dictionary_add` / `dictionary_update`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DictionaryDraft {
    /// The right spelling.
    pub term: String,
    /// Mis-hearings (empty strings are dropped, duplicates collapse).
    #[serde(default)]
    pub heard_as: Vec<String>,
    /// Defaults to on.
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
}

fn enabled_by_default() -> bool {
    true
}

/// How a replacement rule matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleKind {
    /// The pattern is plain text (word boundaries for non-CJK scripts, see [`is_word_char`]).
    #[default]
    Literal,
    /// The pattern is a `regex` crate expression; the replacement may use `$1` / `${name}` / `$$`.
    Regex,
}

/// One replacement rule; the rule list runs in order on the final text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplacementRule {
    /// Stable id.
    pub id: Uuid,
    /// Display name, unique in the list (1–[`MAX_RULE_NAME_CHARS`] characters).
    pub name: String,
    /// Literal or regex.
    pub kind: RuleKind,
    /// What to find (1–[`MAX_PATTERN_CHARS`] characters, not trimmed).
    pub pattern: String,
    /// What to put instead (0–[`MAX_REPLACEMENT_CHARS`] characters, not trimmed).
    #[serde(default)]
    pub replacement: String,
    /// Off: ASCII letters match regardless of case (regex: `case_insensitive`).
    pub case_sensitive: bool,
    /// Off: skipped.
    pub enabled: bool,
    /// Unix milliseconds.
    pub created_at_ms: u64,
    /// Unix milliseconds of the last change.
    pub updated_at_ms: u64,
}

/// What the UI sends to create or change a rule (`rules_add` / `rules_update`), and one
/// `[[rule]]` table of the TOML format.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleDraft {
    /// Display name.
    pub name: String,
    /// Defaults to literal.
    #[serde(default)]
    pub kind: RuleKind,
    /// What to find.
    pub pattern: String,
    /// What to put instead; defaults to empty (delete the match).
    #[serde(default)]
    pub replacement: String,
    /// Defaults to on.
    #[serde(default = "enabled_by_default")]
    pub case_sensitive: bool,
    /// Defaults to on.
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
}

impl From<&ReplacementRule> for RuleDraft {
    fn from(rule: &ReplacementRule) -> Self {
        Self {
            name: rule.name.clone(),
            kind: rule.kind,
            pattern: rule.pattern.clone(),
            replacement: rule.replacement.clone(),
            case_sensitive: rule.case_sensitive,
            enabled: rule.enabled,
        }
    }
}

/// How `rules_import` combines the imported rules with the current ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportMode {
    /// The imported rules become the whole list.
    Replace,
    /// Same-name rules are updated in place, the others are appended.
    Merge,
}

/// How often one entry / rule fired in a take.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VocabularyHit {
    /// Entry or rule id (the nil UUID for a preview's unsaved draft rule).
    pub id: Uuid,
    /// Replacements made.
    pub count: u32,
}

/// Which corrections and rules fired (`HistoryEntry.vocabulary`).
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct VocabularyHits {
    /// Dictionary entries, in dictionary order.
    #[serde(default)]
    pub corrections: Vec<VocabularyHit>,
    /// Rules, in rule order.
    #[serde(default)]
    pub rules: Vec<VocabularyHit>,
}

impl VocabularyHits {
    /// Nothing fired.
    pub fn is_empty(&self) -> bool {
        self.corrections.is_empty() && self.rules.is_empty()
    }

    /// `None` when nothing fired (what the history stores).
    pub fn into_option(self) -> Option<Self> {
        (!self.is_empty()).then_some(self)
    }

    /// Add `other`'s counts (a `live_inject` take sums its sentences), keeping first-seen order.
    pub fn merge(&mut self, other: Self) {
        fn fold(into: &mut Vec<VocabularyHit>, from: Vec<VocabularyHit>) {
            for hit in from {
                match into.iter_mut().find(|h| h.id == hit.id) {
                    Some(h) => h.count = h.count.saturating_add(hit.count),
                    None => into.push(hit),
                }
            }
        }
        fold(&mut self.corrections, other.corrections);
        fold(&mut self.rules, other.rules);
    }
}

/// `vocabulary_preview`'s "as if this rule were saved": replaces the rule with the same `id`, or
/// is appended when `id` is `None`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewDraft {
    /// The rule being edited, or `None` for a new one.
    #[serde(default)]
    pub id: Option<Uuid>,
    /// The draft.
    pub rule: RuleDraft,
}

/// What `vocabulary_preview` answers: the text after the dictionary and after the rules (no LLM).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VocabularyPreview {
    /// After the dictionary corrections.
    pub corrected: String,
    /// After the replacement rules.
    pub output: String,
    /// Entries that fired.
    pub corrections: Vec<VocabularyHit>,
    /// Rules that fired.
    pub rules: Vec<VocabularyHit>,
    /// A step fell back to its input (the runtime would do the same), with the reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Why a dictionary / rule command, a store or an import was refused. The text is what the UI
/// shows: an area prefix and a Chinese detail.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum VocabularyError {
    /// A dictionary entry or list is not acceptable.
    #[error("dictionary: {0}")]
    Dictionary(String),
    /// A rule, the rule list or an import is not acceptable.
    #[error("rules: {0}")]
    Rules(String),
    /// A file could not be written.
    #[error("vocabulary: {0}")]
    Store(String),
}

fn dict_err(message: impl Into<String>) -> VocabularyError {
    VocabularyError::Dictionary(message.into())
}

fn rules_err(message: impl Into<String>) -> VocabularyError {
    VocabularyError::Rules(message.into())
}

/// A trimmed single-line value of 1..=`max` characters, or why not.
fn clean_line(value: &str, what: &str, max: usize) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{what}不能为空"));
    }
    let chars = value.chars().count();
    if chars > max {
        return Err(format!("{what}最多 {max} 个字符（当前 {chars}）"));
    }
    if value.chars().any(char::is_control) {
        return Err(format!("{what}不能包含换行或控制字符"));
    }
    Ok(value.to_owned())
}

/// Validate and normalise a dictionary draft on its own (trim, drop empty variants, collapse
/// duplicates ignoring ASCII case). Cross-entry checks happen in the store.
pub fn validate_dictionary_draft(draft: &DictionaryDraft) -> Result<DictionaryDraft, VocabularyError> {
    let term = clean_line(&draft.term, "正确写法", MAX_TERM_CHARS).map_err(dict_err)?;
    let mut heard_as: Vec<String> = Vec::new();
    for raw in &draft.heard_as {
        if raw.trim().is_empty() {
            continue;
        }
        let variant = clean_line(raw, "误识别写法", MAX_TERM_CHARS).map_err(dict_err)?;
        if variant == term {
            return Err(dict_err(format!("误识别写法「{variant}」和正确写法相同")));
        }
        if !heard_as.iter().any(|h| h.eq_ignore_ascii_case(&variant)) {
            heard_as.push(variant);
        }
    }
    if heard_as.len() > MAX_HEARD_AS {
        return Err(dict_err(format!("一个词条最多 {MAX_HEARD_AS} 个误识别写法（当前 {}）", heard_as.len())));
    }
    Ok(DictionaryDraft { term, heard_as, enabled: draft.enabled })
}

/// Cross-entry rules of the whole dictionary (docs/dictation.md §16.1): at most
/// [`MAX_DICTIONARY_ENTRIES`], unique terms, a mis-hearing belongs to one entry and is nobody's term.
pub fn check_dictionary(entries: &[DictionaryEntry]) -> Result<(), VocabularyError> {
    if entries.len() > MAX_DICTIONARY_ENTRIES {
        return Err(dict_err(format!("词典最多 {MAX_DICTIONARY_ENTRIES} 条")));
    }
    // Each entry against the ones before it, so a conflict names the entry that was there first
    // (new entries are appended).
    for (i, a) in entries.iter().enumerate() {
        for b in &entries[..i] {
            if a.term.eq_ignore_ascii_case(&b.term) {
                return Err(dict_err(format!("词典里已有「{}」", b.term)));
            }
            if b.heard_as.iter().any(|h| h.eq_ignore_ascii_case(&a.term)) {
                return Err(dict_err(format!("「{}」已是「{}」的误识别写法，不能再作为正确写法", a.term, b.term)));
            }
            for variant in &a.heard_as {
                if b.term.eq_ignore_ascii_case(variant) {
                    return Err(dict_err(format!("「{variant}」是词条「{}」的正确写法，不能再作为「{}」的误识别写法", b.term, a.term)));
                }
                if b.heard_as.iter().any(|h| h.eq_ignore_ascii_case(variant)) {
                    return Err(dict_err(format!("「{variant}」已是「{}」的误识别写法", b.term)));
                }
            }
        }
    }
    Ok(())
}

/// Compile a regex rule's pattern with the §16 limits.
pub fn compile_regex(pattern: &str, case_sensitive: bool) -> Result<regex::Regex, String> {
    regex::RegexBuilder::new(pattern).case_insensitive(!case_sensitive).size_limit(REGEX_SIZE_LIMIT).dfa_size_limit(REGEX_DFA_SIZE_LIMIT).build().map_err(|e| {
        match e {
            regex::Error::CompiledTooBig(limit) => format!("正则过大（编译后超过 {limit} 字节）"),
            other => format!("正则无法编译：{other}"),
        }
    })
}

/// Validate and normalise a rule draft on its own (name trimmed; pattern and replacement kept as
/// typed); a regex is compiled here, so an invalid pattern never reaches the list.
pub fn validate_rule_draft(draft: &RuleDraft) -> Result<RuleDraft, VocabularyError> {
    let name = clean_line(&draft.name, "规则名称", MAX_RULE_NAME_CHARS).map_err(rules_err)?;
    if draft.pattern.is_empty() {
        return Err(rules_err(format!("规则「{name}」的匹配内容不能为空")));
    }
    let chars = draft.pattern.chars().count();
    if chars > MAX_PATTERN_CHARS {
        return Err(rules_err(format!("规则「{name}」的匹配内容最多 {MAX_PATTERN_CHARS} 个字符（当前 {chars}）")));
    }
    let chars = draft.replacement.chars().count();
    if chars > MAX_REPLACEMENT_CHARS {
        return Err(rules_err(format!("规则「{name}」的替换内容最多 {MAX_REPLACEMENT_CHARS} 个字符（当前 {chars}）")));
    }
    if draft.kind == RuleKind::Regex {
        compile_regex(&draft.pattern, draft.case_sensitive).map_err(|e| rules_err(format!("规则「{name}」：{e}")))?;
    }
    Ok(RuleDraft { name, ..draft.clone() })
}

/// Rules of the whole list: at most [`MAX_RULES`], names unique.
pub fn check_rules(rules: &[ReplacementRule]) -> Result<(), VocabularyError> {
    if rules.len() > MAX_RULES {
        return Err(rules_err(format!("规则最多 {MAX_RULES} 条")));
    }
    for (i, a) in rules.iter().enumerate() {
        if rules[..i].iter().any(|b| b.name == a.name) {
            return Err(rules_err(format!("已有名为「{}」的规则", a.name)));
        }
    }
    Ok(())
}

/// The glossary prompt for the cloud recogniser (OpenAI `prompt` field): the terms joined with
/// [`GLOSSARY_SEPARATOR`]; `None` when there are none (the field is then not sent).
pub fn glossary_prompt(glossary: &[String]) -> Option<String> {
    (!glossary.is_empty()).then(|| glossary.join(GLOSSARY_SEPARATOR))
}

/// The outcome of one vocabulary step on a text. Never an error: `error` says why the step fell
/// back to its input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    /// The text after the step (the input when it fell back).
    pub text: String,
    /// What fired (empty when it fell back).
    pub hits: Vec<VocabularyHit>,
    /// Why the step fell back.
    pub error: Option<String>,
}

impl Step {
    fn unchanged(text: &str, error: Option<String>) -> Self {
        Self { text: text.to_owned(), hits: Vec::new(), error }
    }
}

/// Run `f`, turning an error or a panic into an unchanged [`Step`].
fn guarded(text: &str, f: impl FnOnce() -> Result<Step, String>) -> Step {
    match std::panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(step)) => step,
        Ok(Err(reason)) => Step::unchanged(text, Some(reason)),
        Err(_) => Step::unchanged(text, Some("内部错误（已回退到原文）".to_owned())),
    }
}

#[derive(Clone)]
enum RuleMatcher {
    Literal(LiteralMatcher),
    Regex(regex::Regex),
}

#[derive(Clone)]
struct CompiledRule {
    id: Uuid,
    name: String,
    matcher: RuleMatcher,
    replacement: String,
}

/// The compiled dictionary and rules the pipeline runs (one snapshot per change, shared behind an
/// `Arc`): the correction matcher, the rules in order, and the glossary.
#[derive(Clone)]
pub struct Vocabulary {
    corrections: Option<LiteralMatcher>,
    /// Matcher target → (entry id, term).
    terms: Vec<(Uuid, String)>,
    rules: Vec<CompiledRule>,
    glossary: Vec<String>,
}

impl std::fmt::Debug for Vocabulary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Counts only: terms and patterns are the user's data.
        f.debug_struct("Vocabulary").field("terms", &self.terms.len()).field("rules", &self.rules.len()).field("glossary", &self.glossary.len()).finish()
    }
}

impl Default for Vocabulary {
    fn default() -> Self {
        Self::empty()
    }
}

impl Vocabulary {
    /// No dictionary, no rules, no glossary: every step passes its text through.
    pub fn empty() -> Self {
        Self { corrections: None, terms: Vec::new(), rules: Vec::new(), glossary: Vec::new() }
    }

    /// Compile the enabled entries and rules. An entry or rule that does not compile (a store
    /// validated at save time never has one) is skipped with a warning, never an error.
    pub fn compile(dictionary: &[DictionaryEntry], rules: &[ReplacementRule]) -> Self {
        let mut terms = Vec::new();
        let mut patterns = Vec::new();
        for entry in dictionary.iter().filter(|e| e.enabled) {
            let target = terms.len();
            terms.push((entry.id, entry.term.clone()));
            patterns.extend(entry.heard_as.iter().map(|variant| (variant.clone(), target)));
        }
        let corrections = if patterns.is_empty() {
            None
        } else {
            match LiteralMatcher::new(&patterns, true) {
                Ok(m) => Some(m),
                Err(e) => {
                    tracing::warn!(error = %e, "dictionary corrections could not be compiled; corrections are off");
                    None
                }
            }
        };
        let compiled = rules
            .iter()
            .filter(|r| r.enabled)
            .filter_map(|rule| {
                let matcher = match rule.kind {
                    RuleKind::Literal => LiteralMatcher::new(&[(rule.pattern.clone(), 0)], !rule.case_sensitive).map(RuleMatcher::Literal),
                    RuleKind::Regex => compile_regex(&rule.pattern, rule.case_sensitive).map(RuleMatcher::Regex),
                };
                match matcher {
                    Ok(matcher) => Some(CompiledRule { id: rule.id, name: rule.name.clone(), matcher, replacement: rule.replacement.clone() }),
                    Err(e) => {
                        tracing::warn!(rule = %rule.id, error = %e, "replacement rule skipped: it does not compile");
                        None
                    }
                }
            })
            .collect();
        Self { corrections, terms, rules: compiled, glossary: build_glossary(dictionary) }
    }

    /// No correction and no rule would ever fire, and the glossary is empty.
    pub fn is_empty(&self) -> bool {
        self.corrections.is_none() && self.rules.is_empty() && self.glossary.is_empty()
    }

    /// The enabled terms for the recogniser prompt and the refiner (capped, dictionary order).
    pub fn glossary(&self) -> &[String] {
        &self.glossary
    }

    /// The same vocabulary with `terms` (a built-in scene's pack, §18.10) after the dictionary's in
    /// the glossary, inside the same limits: a term already there (ignoring ASCII case) is skipped,
    /// the list stops at the first term that would pass [`MAX_GLOSSARY_TERMS`] or
    /// [`MAX_GLOSSARY_CHARS`]. Corrections and rules are unchanged.
    pub fn with_terms(&self, terms: &[&str]) -> Self {
        let mut next = self.clone();
        let mut chars = glossary_chars(&next.glossary);
        for term in terms {
            if next.glossary.len() == MAX_GLOSSARY_TERMS {
                break;
            }
            if next.glossary.iter().any(|t| t.eq_ignore_ascii_case(term)) {
                continue;
            }
            let cost = term.chars().count() + if next.glossary.is_empty() { 0 } else { GLOSSARY_SEPARATOR.chars().count() };
            if chars + cost > MAX_GLOSSARY_CHARS {
                break;
            }
            chars += cost;
            next.glossary.push((*term).to_owned());
        }
        next
    }

    /// Dictionary corrections: every mis-hearing of every enabled entry in one left-to-right pass,
    /// longest match first, replaced text never rescanned (docs/dictation.md §16.2).
    pub fn correct(&self, text: &str) -> Step {
        let Some(matcher) = &self.corrections else { return Step::unchanged(text, None) };
        guarded(text, || {
            if text.len() > MAX_TEXT_BYTES {
                return Err(format!("文本超过 {} KiB，跳过词典纠正", MAX_TEXT_BYTES / 1024));
            }
            let (out, changed) = matcher
                .replace(text, |target| self.terms.get(target).map_or("", |(_, term)| term.as_str()), MAX_TEXT_BYTES)
                .map_err(|_| format!("词典纠正让文本超过 {} KiB，本次不纠正", MAX_TEXT_BYTES / 1024))?;
            let mut counts: BTreeMap<usize, u32> = BTreeMap::new();
            for target in changed {
                let count = counts.entry(target).or_default();
                *count = count.saturating_add(1);
            }
            let hits = counts.into_iter().filter_map(|(target, count)| self.terms.get(target).map(|(id, _)| VocabularyHit { id: *id, count })).collect();
            Ok(Step { text: out, hits, error: None })
        })
    }

    /// The replacement rules in order, each on the previous one's output (docs/dictation.md §16.2).
    /// Input or output over [`MAX_TEXT_BYTES`] falls back to the input.
    pub fn apply_rules(&self, text: &str) -> Step {
        if self.rules.is_empty() {
            return Step::unchanged(text, None);
        }
        guarded(text, || {
            if text.len() > MAX_TEXT_BYTES {
                return Err(format!("文本超过 {} KiB，跳过替换规则", MAX_TEXT_BYTES / 1024));
            }
            let mut current = text.to_owned();
            let mut hits = Vec::new();
            for rule in &self.rules {
                let (next, count) = rule.apply(&current).map_err(|e| format!("规则「{}」{e}，本次不应用替换规则", rule.name))?;
                if count > 0 {
                    hits.push(VocabularyHit { id: rule.id, count });
                    current = next;
                }
            }
            Ok(Step { text: current, hits, error: None })
        })
    }
}

impl CompiledRule {
    /// Apply once; `(text, replacements)`. Errs when the text would exceed [`MAX_TEXT_BYTES`].
    fn apply(&self, text: &str) -> Result<(String, u32), String> {
        let too_long = || format!("让文本超过 {} KiB", MAX_TEXT_BYTES / 1024);
        match &self.matcher {
            RuleMatcher::Literal(matcher) => {
                let (out, changed) = matcher.replace(text, |_| self.replacement.as_str(), MAX_TEXT_BYTES).map_err(|_| too_long())?;
                Ok((out, u32::try_from(changed.len()).unwrap_or(u32::MAX)))
            }
            RuleMatcher::Regex(re) => {
                let mut out = String::with_capacity(text.len());
                let mut expanded = String::new();
                let mut last = 0;
                let mut count = 0u32;
                for caps in re.captures_iter(text) {
                    let Some(m) = caps.get(0) else { continue };
                    // An empty match would insert the replacement between every two characters.
                    if m.as_str().is_empty() {
                        continue;
                    }
                    expanded.clear();
                    caps.expand(&self.replacement, &mut expanded);
                    out.push_str(&text[last..m.start()]);
                    out.push_str(&expanded);
                    last = m.end();
                    if expanded != m.as_str() {
                        count = count.saturating_add(1);
                    }
                    if out.len() > MAX_TEXT_BYTES {
                        return Err(too_long());
                    }
                }
                out.push_str(&text[last..]);
                if out.len() > MAX_TEXT_BYTES {
                    return Err(too_long());
                }
                Ok((out, count))
            }
        }
    }
}

/// Enabled terms in dictionary order, deduplicated ignoring ASCII case, capped at
/// [`MAX_GLOSSARY_TERMS`] and [`MAX_GLOSSARY_CHARS`] (separators included).
/// Characters of `glossary` joined by [`GLOSSARY_SEPARATOR`].
fn glossary_chars(glossary: &[String]) -> usize {
    glossary.iter().map(|t| t.chars().count()).sum::<usize>() + GLOSSARY_SEPARATOR.chars().count() * glossary.len().saturating_sub(1)
}

fn build_glossary(dictionary: &[DictionaryEntry]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut chars = 0usize;
    for entry in dictionary.iter().filter(|e| e.enabled) {
        if out.len() == MAX_GLOSSARY_TERMS {
            break;
        }
        if out.iter().any(|t| t.eq_ignore_ascii_case(&entry.term)) {
            continue;
        }
        let cost = entry.term.chars().count() + if out.is_empty() { 0 } else { GLOSSARY_SEPARATOR.chars().count() };
        if chars + cost > MAX_GLOSSARY_CHARS {
            break;
        }
        chars += cost;
        out.push(entry.term.clone());
    }
    out
}

/// `vocabulary_preview` (docs/dictation.md §16.4): `text` through the dictionary and the rules —
/// with `draft` standing in for the rule it names (or appended) — exactly as the pipeline would,
/// minus the LLM. Errs when `text` is over [`MAX_TEXT_BYTES`] or the draft does not validate.
pub fn preview(
    dictionary: &[DictionaryEntry],
    rules: &[ReplacementRule],
    text: &str,
    draft: Option<&PreviewDraft>,
) -> Result<VocabularyPreview, VocabularyError> {
    if text.len() > MAX_TEXT_BYTES {
        return Err(rules_err(format!("试写文本最多 {} KiB", MAX_TEXT_BYTES / 1024)));
    }
    let rules: Vec<ReplacementRule> = match draft {
        None => rules.to_vec(),
        Some(draft) => {
            let rule = validate_rule_draft(&draft.rule)?;
            let as_rule = |id: Uuid| ReplacementRule {
                id,
                name: rule.name.clone(),
                kind: rule.kind,
                pattern: rule.pattern.clone(),
                replacement: rule.replacement.clone(),
                case_sensitive: rule.case_sensitive,
                enabled: rule.enabled,
                created_at_ms: 0,
                updated_at_ms: 0,
            };
            let mut list = rules.to_vec();
            match draft.id.and_then(|id| list.iter().position(|r| r.id == id)) {
                Some(i) => list[i] = as_rule(list[i].id),
                None => list.push(as_rule(draft.id.unwrap_or_else(Uuid::nil))),
            }
            list
        }
    };
    let vocabulary = Vocabulary::compile(dictionary, &rules);
    let corrected = vocabulary.correct(text);
    let ruled = vocabulary.apply_rules(&corrected.text);
    Ok(VocabularyPreview {
        corrected: corrected.text,
        output: ruled.text,
        corrections: corrected.hits,
        rules: ruled.hits,
        error: corrected.error.or(ruled.error),
    })
}

#[cfg(test)]
mod tests;
