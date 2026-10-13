//! What the tools return (docs/dictation.md §25): the dictionary and the rules with their hits,
//! and a history entry as the full history shares it — its texts, application, scene, preset,
//! models, duration and how it ended; not the selection a voice edit rewrote, not a failure's
//! wording, not a window title. Every result stays under [`MAX_RESULT_BYTES`]: a page ends early
//! (its cursor goes on from there), and an entry too large on its own has its texts shortened.

use serde::Serialize;
use uuid::Uuid;

use super::MAX_RESULT_BYTES;
use crate::dictation::TakeKind;
use crate::history::{HistoryEntry, OriginKind, Outcome};
use crate::scenes::AppRef;
use crate::vocabulary::{DictionaryEntry, EntrySource, ReplacementRule, RuleKind};

/// Room kept for the fields around a page's items.
const ENVELOPE_BYTES: usize = 4 * 1024;

/// Where an entry or rule came from, as the tools show it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SourceView {
    /// `manual`, `history` or `mcp`.
    pub kind: &'static str,
    /// The AI tool that wrote it (`mcp`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
    /// The history entry it was added from (`history`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_id: Option<Uuid>,
}

impl From<&EntrySource> for SourceView {
    fn from(source: &EntrySource) -> Self {
        match source {
            EntrySource::Manual => Self { kind: "manual", client: None, history_id: None },
            EntrySource::History { history_id } => Self { kind: "history", client: None, history_id: Some(*history_id) },
            EntrySource::Mcp { client } => Self { kind: "mcp", client: Some(client.clone()), history_id: None },
        }
    }
}

/// A dictionary entry as `dictionary_list` returns it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EntryView {
    /// Its id.
    pub id: Uuid,
    /// The right spelling.
    pub term: String,
    /// The forms it corrects.
    pub heard_as: Vec<String>,
    /// On.
    pub enabled: bool,
    /// Where it came from.
    pub source: SourceView,
    /// Corrections it made in the history.
    pub hits: u32,
}

impl EntryView {
    /// `entry` with `hits`.
    pub fn new(entry: &DictionaryEntry, hits: u32) -> Self {
        Self { id: entry.id, term: entry.term.clone(), heard_as: entry.heard_as.clone(), enabled: entry.enabled, source: (&entry.source).into(), hits }
    }
}

/// A rule as `rules_list` returns it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RuleView {
    /// Its id.
    pub id: Uuid,
    /// Its place in the execution order, from 1.
    pub position: usize,
    /// Its name.
    pub name: String,
    /// Literal or regex.
    pub kind: RuleKind,
    /// What it finds.
    pub pattern: String,
    /// What it puts instead.
    pub replacement: String,
    /// ASCII case matters.
    pub case_sensitive: bool,
    /// On.
    pub enabled: bool,
    /// It waits for the user's approval and does not run.
    pub pending: bool,
    /// Where it came from.
    pub source: SourceView,
    /// Replacements it made in the history.
    pub hits: u32,
}

impl RuleView {
    /// `rule` at `position` (from 1) with `hits`.
    pub fn new(rule: &ReplacementRule, position: usize, hits: u32) -> Self {
        Self {
            id: rule.id,
            position,
            name: rule.name.clone(),
            kind: rule.kind,
            pattern: rule.pattern.clone(),
            replacement: rule.replacement.clone(),
            case_sensitive: rule.case_sensitive,
            enabled: rule.enabled,
            pending: rule.pending,
            source: (&rule.source).into(),
            hits,
        }
    }
}

/// A history entry as the full history shares it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HistoryRecord {
    /// Its id.
    pub id: Uuid,
    /// Unix milliseconds.
    pub at_ms: u64,
    /// Dictation or voice edit.
    pub kind: TakeKind,
    /// What the recogniser wrote (a voice edit's spoken instruction).
    pub raw_text: String,
    /// The text that was inserted (a voice edit's result).
    pub text: String,
    /// What 用 AI 预设处理 made of the text, when it ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processed_text: Option<String>,
    /// AI polish's text was used.
    pub refined: bool,
    /// The application in front.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<AppRef>,
    /// The scene's name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scene: Option<String>,
    /// The AI preset's name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// The recognition model.
    pub asr_model: String,
    /// The AI polish model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refine_model: Option<String>,
    /// Recording length.
    pub duration_ms: u64,
    /// `inserted`, `clipboard` (the text stayed on the clipboard) or `failed`.
    pub outcome: &'static str,
    /// What a phone sent, when it came from one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<OriginKind>,
    /// The texts were shortened to fit one result.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

impl From<&HistoryEntry> for HistoryRecord {
    fn from(entry: &HistoryEntry) -> Self {
        Self {
            id: entry.id,
            at_ms: entry.at_ms,
            kind: entry.kind,
            raw_text: entry.raw_text.clone(),
            text: entry.text.clone(),
            processed_text: entry.processed.as_ref().map(|p| p.text.clone()),
            refined: entry.refined,
            app: entry.app.clone(),
            scene: entry.scene.as_ref().map(|s| s.name.clone()),
            preset: entry.preset.as_ref().map(|p| p.name.clone()),
            asr_model: entry.asr_model.clone(),
            refine_model: entry.refine_model.clone(),
            duration_ms: entry.duration_ms,
            outcome: match entry.outcome {
                Outcome::Inserted { .. } => "inserted",
                Outcome::Clipboard { .. } => "clipboard",
                Outcome::Failed { .. } => "failed",
            },
            origin: entry.origin.as_ref().map(|o| o.kind),
            truncated: false,
        }
    }
}

/// `text` cut to at most `max` bytes at a character boundary, `…` added when cut.
fn cut(text: &mut String, max: usize) -> bool {
    if text.len() <= max {
        return false;
    }
    let mut end = max.saturating_sub('…'.len_utf8());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text.push('…');
    true
}

impl HistoryRecord {
    /// The record with its texts shortened so that it serialises to at most `budget` bytes.
    pub fn fitted(mut self, budget: usize) -> Self {
        let mut share = budget / 3;
        while json_len(&self) > budget && share > 0 {
            let mut cut_any = cut(&mut self.raw_text, share);
            cut_any |= cut(&mut self.text, share);
            if let Some(processed) = &mut self.processed_text {
                cut_any |= cut(processed, share);
            }
            self.truncated |= cut_any;
            // JSON escapes can make a text larger than its bytes: shrink until it fits.
            share = share * 3 / 4;
        }
        self
    }
}

/// Bytes of `value` as compact JSON.
pub fn json_len<T: Serialize>(value: &T) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX, |v| v.len())
}

/// The items of a page from `offset`, at most `page` of them and within the result's byte limit;
/// the second value is where the next page starts (`None` at the end). At least one item comes
/// back while any is left, so a page always moves on.
pub fn page<T: Serialize + Clone>(items: &[T], offset: usize, page: usize) -> (Vec<T>, Option<usize>) {
    let budget = MAX_RESULT_BYTES - ENVELOPE_BYTES;
    let mut out = Vec::new();
    let mut used = 0usize;
    for item in items.iter().skip(offset).take(page) {
        let size = json_len(item) + 1;
        if !out.is_empty() && used + size > budget {
            break;
        }
        used += size;
        out.push(item.clone());
    }
    let next = offset + out.len();
    (out, (next < items.len()).then_some(next))
}

/// History records within the result's byte limit: as many as fit (the first one shortened when
/// it alone is too large); the count kept.
pub fn fit_records(records: Vec<HistoryRecord>) -> Vec<HistoryRecord> {
    let budget = MAX_RESULT_BYTES - ENVELOPE_BYTES;
    let mut out = Vec::new();
    let mut used = 0usize;
    for record in records {
        let record = if out.is_empty() { record.fitted(budget) } else { record };
        let size = json_len(&record) + 1;
        if !out.is_empty() && used + size > budget {
            break;
        }
        used += size;
        out.push(record);
    }
    out
}

/// A single record within the result's byte limit.
pub fn fit_record(record: HistoryRecord) -> HistoryRecord {
    record.fitted(MAX_RESULT_BYTES - ENVELOPE_BYTES)
}
