//! The tools and the prompt the service lists (docs/dictation.md §25), their input schemas, and the
//! parsing of a call's arguments into a [`McpRequest`]. The descriptions are written for the
//! model that reads them: what the tool does, what it needs from the settings, the limits.

use serde::Deserialize;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::{MAX_HISTORY_PAGE, ToolError};
use crate::vocabulary::{BACKTEST_LIMIT, BacktestDraft, DictionaryDraft, MAX_BACKTEST_LIMIT, PreviewDraft, RuleDraft, RuleKind};

/// Every tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    /// The dictionary, with hits.
    DictionaryList,
    /// The rules, with hits.
    RulesList,
    /// A text through the dictionary and the rules, optionally with a draft rule.
    VocabularyPreview,
    /// What a draft entry or rule would change in the recent history.
    VocabularyBacktest,
    /// The pairs the AI clean-up changed.
    VocabularyCandidates,
    /// A page of full history entries.
    HistorySearch,
    /// One full history entry.
    HistoryGet,
    /// Add an entry, or the forms to an entry with the same spelling.
    DictionaryAdd,
    /// Change an entry.
    DictionaryUpdate,
    /// Create a rule.
    RulesAdd,
    /// Change a rule.
    RulesUpdate,
}

impl Tool {
    /// In the order `tools/list` gives them.
    pub const ALL: [Self; 11] = [
        Self::DictionaryList,
        Self::RulesList,
        Self::VocabularyPreview,
        Self::VocabularyBacktest,
        Self::VocabularyCandidates,
        Self::HistorySearch,
        Self::HistoryGet,
        Self::DictionaryAdd,
        Self::DictionaryUpdate,
        Self::RulesAdd,
        Self::RulesUpdate,
    ];

    /// Its name on the wire.
    pub const fn name(self) -> &'static str {
        match self {
            Self::DictionaryList => "dictionary_list",
            Self::RulesList => "rules_list",
            Self::VocabularyPreview => "vocabulary_preview",
            Self::VocabularyBacktest => "vocabulary_backtest",
            Self::VocabularyCandidates => "vocabulary_candidates",
            Self::HistorySearch => "history_search",
            Self::HistoryGet => "history_get",
            Self::DictionaryAdd => "dictionary_add",
            Self::DictionaryUpdate => "dictionary_update",
            Self::RulesAdd => "rules_add",
            Self::RulesUpdate => "rules_update",
        }
    }

    /// The tool called `name`.
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.name() == name)
    }

    /// Whether it changes nothing.
    pub const fn read_only(self) -> bool {
        !matches!(self, Self::DictionaryAdd | Self::DictionaryUpdate | Self::RulesAdd | Self::RulesUpdate)
    }
}

/// A tool as `tools/list` describes it.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolSpec {
    /// Its name.
    pub name: &'static str,
    /// A short title.
    pub title: &'static str,
    /// What it does, for the model.
    pub description: &'static str,
    /// JSON Schema of its arguments (an object).
    pub input_schema: Value,
    /// It changes nothing (`readOnlyHint`).
    pub read_only: bool,
    /// Calling it again with the same arguments changes nothing more (`idempotentHint`).
    pub idempotent: bool,
}

/// One argument of a prompt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptArgument {
    /// Its name.
    pub name: &'static str,
    /// What it is.
    pub description: &'static str,
    /// Whether it must be given.
    pub required: bool,
}

/// A prompt as `prompts/list` describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptSpec {
    /// Its name.
    pub name: &'static str,
    /// A short title.
    pub title: &'static str,
    /// What it is for.
    pub description: &'static str,
    /// Its arguments.
    pub arguments: Vec<PromptArgument>,
}

/// The one prompt: review the recent misrecognitions and suggest fixes.
pub const REVIEW_PROMPT: &str = "review_misrecognitions";

/// The prompts the service lists.
pub fn prompts() -> Vec<PromptSpec> {
    vec![PromptSpec {
        name: REVIEW_PROMPT,
        title: "Review misrecognitions",
        description: "Find words that Voltip's speech recognition keeps getting wrong, check what a fix would change, and suggest dictionary entries or replacement rules for the user to approve.",
        arguments: vec![PromptArgument { name: "days", description: "How many days of history to look at (default 30).", required: false }],
    }]
}

/// The text of prompt `name`.
pub fn prompt_text(name: &str, arguments: &Map<String, Value>) -> Result<String, ToolError> {
    if name != REVIEW_PROMPT {
        return Err(ToolError::Unknown(format!("没有名为「{name}」的提示 / no prompt named {name}")));
    }
    let days = match arguments.get("days") {
        None | Some(Value::Null) => 30,
        Some(Value::Number(n)) => n.as_u64().filter(|d| (1..=3650).contains(d)).ok_or_else(|| invalid("days must be a whole number of days, 1–3650"))?,
        Some(Value::String(s)) => {
            s.trim().parse::<u64>().ok().filter(|d| (1..=3650).contains(d)).ok_or_else(|| invalid("days must be a whole number of days, 1–3650"))?
        }
        Some(_) => return Err(invalid("days must be a whole number of days, 1–3650")),
    };
    Ok(format!(
        "Review what Voltip's speech recognition got wrong over the last {days} days and suggest fixes.\n\n\
         1. Call `vocabulary_candidates` with `since` set to {days} days ago. Each pair is a stretch the recogniser wrote (`heard`) \
         and what the AI clean-up made of it (`polished`), with how often it happened. Skip pairs marked `known` (the dictionary \
         already fixes them) and pairs that are only rewording or punctuation.\n\
         2. For every likely misrecognition, draft a dictionary entry (`term` = the right spelling, `heard_as` = the wrong forms). \
         Use a replacement rule only for text that is not a misheard word (a phrase the clean-up keeps writing wrongly).\n\
         3. Call `vocabulary_backtest` for each draft. Drop or narrow a draft when `kept` is high: the clean-up kept that form, so it \
         is probably right in those entries. Be careful with forms of one or two characters.\n\
         4. Show the user a short table of the drafts (wrong form → right form, how often, what the backtest found) and ask which \
         ones to save. Write only what the user approves, with `dictionary_add` or `rules_add`. New rules wait for the user's \
         approval in Voltip unless Voltip allows AI tools to turn rules on.\n\n\
         If a tool says Voltip does not share something, tell the user what it said. Talk to the user in their language."
    ))
}

fn invalid(message: &str) -> ToolError {
    ToolError::InvalidArguments(format!("参数不正确 / invalid arguments: {message}"))
}

/// A UUID string in a schema.
fn uuid_schema(description: &str) -> Value {
    json!({ "type": "string", "format": "uuid", "description": description })
}

/// An instant in a schema: Unix milliseconds or an RFC 3339 date / date-time.
fn instant_schema(description: &str) -> Value {
    json!({
        "description": format!("{description} Unix time in milliseconds, or an RFC 3339 date-time (2026-10-01T09:00:00+08:00) or date (2026-10-01, midnight UTC)."),
        "anyOf": [{ "type": "integer", "minimum": 0 }, { "type": "string" }]
    })
}

fn dictionary_draft_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "term": { "type": "string", "description": "The right spelling, 1–64 characters." },
            "heard_as": { "type": "array", "items": { "type": "string" }, "maxItems": 10, "description": "Forms the recogniser writes instead, up to 10, each 1–64 characters." },
            "enabled": { "type": "boolean", "default": true }
        },
        "required": ["term"],
        "additionalProperties": false
    })
}

fn rule_draft_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": { "type": "string", "description": "A name, 1–64 characters, unique among the rules." },
            "kind": { "type": "string", "enum": ["literal", "regex"], "default": "literal", "description": "literal: plain text (whole words in scripts with spaces); regex: Rust regex syntax, the replacement may use $1, ${name} and $$." },
            "pattern": { "type": "string", "description": "What to find, 1–256 characters." },
            "replacement": { "type": "string", "default": "", "description": "What to put instead, up to 256 characters; empty deletes the match." },
            "case_sensitive": { "type": "boolean", "default": true },
            "enabled": { "type": "boolean", "default": true }
        },
        "required": ["name", "pattern"],
        "additionalProperties": false
    })
}

fn list_schema() -> Value {
    json!({
        "type": "object",
        "properties": { "cursor": { "type": "string", "description": "next_cursor of the previous page." } },
        "additionalProperties": false
    })
}

/// Every tool as `tools/list` describes it.
pub fn specs() -> Vec<ToolSpec> {
    Tool::ALL.into_iter().map(spec).collect()
}

/// `tool` as `tools/list` describes it.
pub fn spec(tool: Tool) -> ToolSpec {
    let (title, description, input_schema) = match tool {
        Tool::DictionaryList => (
            "List the dictionary",
            "The personal dictionary in matching order: each entry's right spelling (`term`), the forms the recogniser writes instead (`heard_as`), whether it is on, where it came from and how often it corrected a take (`hits`). Voltip corrects these forms right after recognition and sends the right spellings to cloud recognition and AI polish as terms. Paged: pass `next_cursor` back as `cursor`.",
            list_schema(),
        ),
        Tool::RulesList => (
            "List the replacement rules",
            "The replacement rules in execution order (they run on the final text after AI polish): name, literal or regex, pattern, replacement, whether it is on, whether it waits for the user's approval (`pending`: such a rule does not run), where it came from and its `hits`. Paged: pass `next_cursor` back as `cursor`.",
            list_schema(),
        ),
        Tool::VocabularyPreview => (
            "Try a text",
            "Run a text through the dictionary corrections and the rules exactly as dictation would, without AI polish; with `draft`, as if that rule were saved (`draft.id` names the rule it replaces, none appends it). Rules waiting for approval do not run. Returns the text after the dictionary (`corrected`), after the rules (`output`) and which entries and rules fired.",
            json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "Up to 64 KiB." },
                    "draft": {
                        "type": "object",
                        "properties": { "id": { "type": ["string", "null"], "format": "uuid" }, "rule": rule_draft_schema() },
                        "required": ["rule"],
                        "additionalProperties": false
                    }
                },
                "required": ["text"],
                "additionalProperties": false
            }),
        ),
        Tool::VocabularyBacktest => (
            "Backtest a draft",
            "What a dictionary entry or a rule that is not saved yet would change in the most recent takes (500 unless `limit` says otherwise, at most 2000): how many it scanned, how many it would change (`matched`) and, for an entry, in how many of those the final text kept a form the entry replaces (`kept`, a sign the form may be right there). A dictionary draft is compared with the current dictionary (`draft.id` names the entry it replaces); a rule draft runs alone on each final text. Examples (±12 characters around the change) need the correction summary to be shared in Voltip's settings; without it they are left out.",
            json!({
                "type": "object",
                "properties": {
                    "draft": {
                        "oneOf": [
                            {
                                "type": "object",
                                "properties": {
                                    "kind": { "const": "dictionary" },
                                    "id": { "type": ["string", "null"], "format": "uuid" },
                                    "entry": dictionary_draft_schema()
                                },
                                "required": ["kind", "entry"],
                                "additionalProperties": false
                            },
                            {
                                "type": "object",
                                "properties": {
                                    "kind": { "const": "rule" },
                                    "id": { "type": ["string", "null"], "format": "uuid" },
                                    "rule": rule_draft_schema()
                                },
                                "required": ["kind", "rule"],
                                "additionalProperties": false
                            }
                        ]
                    },
                    "history_id": uuid_schema("A history entry to leave out (the one the draft came from)."),
                    "limit": { "type": "integer", "minimum": 1, "maximum": MAX_BACKTEST_LIMIT, "default": BACKTEST_LIMIT }
                },
                "required": ["draft"],
                "additionalProperties": false
            }),
        ),
        Tool::VocabularyCandidates => (
            "Summarise what AI polish corrected",
            "Pairs of what the recogniser wrote (`heard`) and what AI polish made of it (`polished`) in the recent takes, most frequent first: how often, when last, whether the dictionary already fixes the form (`known`) and up to three snippets of ±12 characters around it. Punctuation-only changes are left out. Needs the correction summary to be shared in Voltip's settings (the default).",
            json!({
                "type": "object",
                "properties": {
                    "since": instant_schema("Only takes at or after this instant."),
                    "limit": { "type": "integer", "minimum": 1, "maximum": super::candidates::MAX_PAIRS, "default": super::candidates::DEFAULT_PAIRS }
                },
                "additionalProperties": false
            }),
        ),
        Tool::HistorySearch => (
            "Search the history",
            "Full history entries, newest first: the recognised text (`raw_text`), the final text (`text`), the processed text, application, scene, preset, models, duration and how it ended. Needs the full history to be shared in Voltip's settings, and only reaches back as far as the user allowed. `query` matches the texts, the application and the scene (case-insensitive). At most 50 entries a page: pass `next_cursor` back as `cursor`.",
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "since": instant_schema("Entries at or after this instant."),
                    "until": instant_schema("Entries before this instant."),
                    "app": { "type": "string", "description": "An application's id or (part of) its name." },
                    "cursor": { "type": "string", "description": "next_cursor of the previous page." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": MAX_HISTORY_PAGE, "default": DEFAULT_HISTORY_PAGE }
                },
                "additionalProperties": false
            }),
        ),
        Tool::HistoryGet => (
            "Read one history entry",
            "Every text field of one history entry, as `history_search` returns it. Needs the full history to be shared in Voltip's settings, and the entry to be within the period the user allowed.",
            json!({ "type": "object", "properties": { "id": uuid_schema("The entry.") }, "required": ["id"], "additionalProperties": false }),
        ),
        Tool::DictionaryAdd => (
            "Add a dictionary entry",
            "Add an entry: the right spelling (`term`) and the forms the recogniser writes instead (`heard_as`). When an entry with that spelling exists (ignoring ASCII case), the new forms are added to it; forms it already has change nothing, so a repeated call is harmless. Voltip checks the entry as its dictionary page does (a form may belong to one entry only and may not be another entry's spelling). The user sees every change and can undo it. Needs Voltip to allow AI tools to edit the dictionary.",
            dictionary_draft_schema(),
        ),
        Tool::DictionaryUpdate => (
            "Change a dictionary entry",
            "Change an entry's right spelling, its forms (the whole list) or whether it is on; fields left out stay as they are. No entry can be deleted, only switched off. The user sees every change and can undo it. Needs Voltip to allow AI tools to edit the dictionary.",
            json!({
                "type": "object",
                "properties": {
                    "id": uuid_schema("The entry (dictionary_list gives the ids)."),
                    "term": { "type": "string" },
                    "heard_as": { "type": "array", "items": { "type": "string" }, "maxItems": 10 },
                    "enabled": { "type": "boolean" }
                },
                "required": ["id"],
                "additionalProperties": false
            }),
        ),
        Tool::RulesAdd => (
            "Create a replacement rule",
            "Create a rule at the end of the list. Unless Voltip allows AI tools to turn rules on, the rule is created off and waits for the user's approval (`pending`); it runs nowhere until the user turns it on. The user sees every change and can undo it. Needs Voltip to allow AI tools to edit the dictionary.",
            rule_draft_schema(),
        ),
        Tool::RulesUpdate => (
            "Change a replacement rule",
            "Change a rule; fields left out stay as they are. Unless Voltip allows AI tools to turn rules on, only a rule this tool created that still waits for approval can be changed, and it keeps waiting. No rule can be deleted. The user sees every change and can undo it.",
            json!({
                "type": "object",
                "properties": {
                    "id": uuid_schema("The rule (rules_list gives the ids)."),
                    "name": { "type": "string" },
                    "kind": { "type": "string", "enum": ["literal", "regex"] },
                    "pattern": { "type": "string" },
                    "replacement": { "type": "string" },
                    "case_sensitive": { "type": "boolean" },
                    "enabled": { "type": "boolean" }
                },
                "required": ["id"],
                "additionalProperties": false
            }),
        ),
    };
    ToolSpec {
        name: tool.name(),
        title,
        description,
        input_schema,
        read_only: tool.read_only(),
        // Adding the same forms or the same fields again changes nothing more; a new rule is a new rule.
        idempotent: tool.read_only() || matches!(tool, Tool::DictionaryAdd | Tool::DictionaryUpdate | Tool::RulesUpdate),
    }
}

/// Entries a `history_search` page has unless `limit` says otherwise.
pub const DEFAULT_HISTORY_PAGE: usize = 20;
/// Items a `dictionary_list` / `rules_list` page has (fewer when they would pass
/// [`super::MAX_RESULT_BYTES`]).
pub const LIST_PAGE: usize = 100;

/// A change to some fields of a dictionary entry (`dictionary_update`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EntryPatch {
    /// A new spelling.
    pub term: Option<String>,
    /// The new list of forms.
    pub heard_as: Option<Vec<String>>,
    /// On or off.
    pub enabled: Option<bool>,
}

impl EntryPatch {
    /// `draft` with this patch applied.
    pub fn apply(&self, mut draft: DictionaryDraft) -> DictionaryDraft {
        if let Some(term) = &self.term {
            draft.term.clone_from(term);
        }
        if let Some(heard_as) = &self.heard_as {
            draft.heard_as.clone_from(heard_as);
        }
        if let Some(enabled) = self.enabled {
            draft.enabled = enabled;
        }
        draft
    }
}

/// A change to some fields of a rule (`rules_update`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RulePatch {
    /// A new name.
    pub name: Option<String>,
    /// Literal or regex.
    pub kind: Option<RuleKind>,
    /// A new pattern.
    pub pattern: Option<String>,
    /// A new replacement.
    pub replacement: Option<String>,
    /// Case sensitivity.
    pub case_sensitive: Option<bool>,
    /// On or off.
    pub enabled: Option<bool>,
}

impl RulePatch {
    /// `draft` with this patch applied.
    pub fn apply(&self, mut draft: RuleDraft) -> RuleDraft {
        if let Some(name) = &self.name {
            draft.name.clone_from(name);
        }
        if let Some(kind) = self.kind {
            draft.kind = kind;
        }
        if let Some(pattern) = &self.pattern {
            draft.pattern.clone_from(pattern);
        }
        if let Some(replacement) = &self.replacement {
            draft.replacement.clone_from(replacement);
        }
        if let Some(case_sensitive) = self.case_sensitive {
            draft.case_sensitive = case_sensitive;
        }
        if let Some(enabled) = self.enabled {
            draft.enabled = enabled;
        }
        draft
    }
}

/// Where a `history_search` page starts: after the entry at `at_ms` with database row `row`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistoryCursor {
    /// The last entry's time.
    pub at_ms: u64,
    /// The last entry's row.
    pub row: i64,
}

impl HistoryCursor {
    /// The cursor as the client gets it.
    pub fn encode(self) -> String {
        format!("{}:{}", self.at_ms, self.row)
    }

    fn decode(text: &str) -> Option<Self> {
        let (at, row) = text.split_once(':')?;
        Some(Self { at_ms: at.parse().ok()?, row: row.parse().ok()? })
    }
}

/// A call, its arguments parsed and checked on their own (the settings and the lists are the
/// core's to check).
#[derive(Clone, Debug, PartialEq)]
pub enum McpRequest {
    /// `dictionary_list`.
    DictionaryList {
        /// The first entry of the page.
        offset: usize,
    },
    /// `rules_list`.
    RulesList {
        /// The first rule of the page.
        offset: usize,
    },
    /// `vocabulary_preview`.
    Preview {
        /// The text.
        text: String,
        /// A rule standing in.
        draft: Option<PreviewDraft>,
    },
    /// `vocabulary_backtest`.
    Backtest {
        /// The draft.
        draft: BacktestDraft,
        /// The entry left out.
        exclude: Option<Uuid>,
        /// Takes to scan.
        limit: usize,
    },
    /// `vocabulary_candidates`.
    Candidates {
        /// Takes at or after this.
        since_ms: Option<u64>,
        /// Pairs to return.
        limit: usize,
    },
    /// `history_search`.
    HistorySearch {
        /// Text to match.
        query: Option<String>,
        /// At or after.
        since_ms: Option<u64>,
        /// Before.
        until_ms: Option<u64>,
        /// Application id or name.
        app: Option<String>,
        /// Where the page starts.
        cursor: Option<HistoryCursor>,
        /// Entries on the page.
        limit: usize,
    },
    /// `history_get`.
    HistoryGet {
        /// The entry.
        id: Uuid,
    },
    /// `dictionary_add`.
    DictionaryAdd {
        /// The entry.
        draft: DictionaryDraft,
    },
    /// `dictionary_update`.
    DictionaryUpdate {
        /// The entry.
        id: Uuid,
        /// What changes.
        patch: EntryPatch,
    },
    /// `rules_add`.
    RulesAdd {
        /// The rule.
        draft: RuleDraft,
    },
    /// `rules_update`.
    RulesUpdate {
        /// The rule.
        id: Uuid,
        /// What changes.
        patch: RulePatch,
    },
}

impl McpRequest {
    /// The tool it calls.
    pub const fn tool(&self) -> Tool {
        match self {
            Self::DictionaryList { .. } => Tool::DictionaryList,
            Self::RulesList { .. } => Tool::RulesList,
            Self::Preview { .. } => Tool::VocabularyPreview,
            Self::Backtest { .. } => Tool::VocabularyBacktest,
            Self::Candidates { .. } => Tool::VocabularyCandidates,
            Self::HistorySearch { .. } => Tool::HistorySearch,
            Self::HistoryGet { .. } => Tool::HistoryGet,
            Self::DictionaryAdd { .. } => Tool::DictionaryAdd,
            Self::DictionaryUpdate { .. } => Tool::DictionaryUpdate,
            Self::RulesAdd { .. } => Tool::RulesAdd,
            Self::RulesUpdate { .. } => Tool::RulesUpdate,
        }
    }
}

/// An instant argument: Unix milliseconds or an RFC 3339 date-time / date.
#[derive(Deserialize)]
#[serde(untagged)]
enum InstantArg {
    Millis(u64),
    Text(String),
}

impl InstantArg {
    fn millis(self, field: &str) -> Result<u64, ToolError> {
        match self {
            Self::Millis(ms) => Ok(ms),
            Self::Text(text) => parse_instant(&text).ok_or_else(|| invalid(&format!("{field}: 「{text}」 is neither Unix milliseconds nor an RFC 3339 date"))),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListArgs {
    #[serde(default)]
    cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewArgs {
    text: String,
    #[serde(default)]
    draft: Option<PreviewDraft>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BacktestArgs {
    draft: BacktestDraft,
    #[serde(default)]
    history_id: Option<Uuid>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CandidatesArgs {
    #[serde(default)]
    since: Option<InstantArg>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchArgs {
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    since: Option<InstantArg>,
    #[serde(default)]
    until: Option<InstantArg>,
    #[serde(default)]
    app: Option<String>,
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IdArgs {
    id: Uuid,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EntryUpdateArgs {
    id: Uuid,
    #[serde(default)]
    term: Option<String>,
    #[serde(default)]
    heard_as: Option<Vec<String>>,
    #[serde(default)]
    enabled: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleUpdateArgs {
    id: Uuid,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    kind: Option<RuleKind>,
    #[serde(default)]
    pattern: Option<String>,
    #[serde(default)]
    replacement: Option<String>,
    #[serde(default)]
    case_sensitive: Option<bool>,
    #[serde(default)]
    enabled: Option<bool>,
}

fn args<T: serde::de::DeserializeOwned>(arguments: Map<String, Value>) -> Result<T, ToolError> {
    serde_json::from_value(Value::Object(arguments)).map_err(|e| invalid(&e.to_string()))
}

/// A page size within `1..=max`, `default` when absent.
fn limit(given: Option<usize>, default: usize, max: usize) -> Result<usize, ToolError> {
    match given {
        None => Ok(default),
        Some(n) if (1..=max).contains(&n) => Ok(n),
        Some(n) => Err(invalid(&format!("limit must be 1–{max} (got {n})"))),
    }
}

fn list_offset(cursor: Option<String>) -> Result<usize, ToolError> {
    match cursor {
        None => Ok(0),
        Some(text) => text.trim().parse().map_err(|_| invalid("cursor: pass next_cursor of the previous page")),
    }
}

/// Parse the arguments of tool `name` (`ToolError::Unknown` for a name no tool has).
pub fn parse_call(name: &str, arguments: Map<String, Value>) -> Result<McpRequest, ToolError> {
    let tool = Tool::named(name).ok_or_else(|| ToolError::Unknown(format!("没有名为「{name}」的工具 / no tool named {name}")))?;
    Ok(match tool {
        Tool::DictionaryList => McpRequest::DictionaryList { offset: list_offset(args::<ListArgs>(arguments)?.cursor)? },
        Tool::RulesList => McpRequest::RulesList { offset: list_offset(args::<ListArgs>(arguments)?.cursor)? },
        Tool::VocabularyPreview => {
            let PreviewArgs { text, draft } = args(arguments)?;
            McpRequest::Preview { text, draft }
        }
        Tool::VocabularyBacktest => {
            let BacktestArgs { draft, history_id, limit: given } = args(arguments)?;
            McpRequest::Backtest { draft, exclude: history_id, limit: limit(given, BACKTEST_LIMIT, MAX_BACKTEST_LIMIT)? }
        }
        Tool::VocabularyCandidates => {
            let CandidatesArgs { since, limit: given } = args(arguments)?;
            McpRequest::Candidates {
                since_ms: since.map(|s| s.millis("since")).transpose()?,
                limit: limit(given, super::candidates::DEFAULT_PAIRS, super::candidates::MAX_PAIRS)?,
            }
        }
        Tool::HistorySearch => {
            let SearchArgs { query, since, until, app, cursor, limit: given } = args(arguments)?;
            let cursor = cursor.map(|c| HistoryCursor::decode(&c).ok_or_else(|| invalid("cursor: pass next_cursor of the previous page"))).transpose()?;
            McpRequest::HistorySearch {
                query: query.map(|q| q.trim().to_owned()).filter(|q| !q.is_empty()),
                since_ms: since.map(|s| s.millis("since")).transpose()?,
                until_ms: until.map(|u| u.millis("until")).transpose()?,
                app: app.map(|a| a.trim().to_owned()).filter(|a| !a.is_empty()),
                cursor,
                limit: limit(given, DEFAULT_HISTORY_PAGE, MAX_HISTORY_PAGE)?,
            }
        }
        Tool::HistoryGet => McpRequest::HistoryGet { id: args::<IdArgs>(arguments)?.id },
        Tool::DictionaryAdd => McpRequest::DictionaryAdd { draft: args(arguments)? },
        Tool::DictionaryUpdate => {
            let EntryUpdateArgs { id, term, heard_as, enabled } = args(arguments)?;
            McpRequest::DictionaryUpdate { id, patch: EntryPatch { term, heard_as, enabled } }
        }
        Tool::RulesAdd => McpRequest::RulesAdd { draft: args(arguments)? },
        Tool::RulesUpdate => {
            let RuleUpdateArgs { id, name, kind, pattern, replacement, case_sensitive, enabled } = args(arguments)?;
            McpRequest::RulesUpdate { id, patch: RulePatch { name, kind, pattern, replacement, case_sensitive, enabled } }
        }
    })
}

/// Days from 1970-01-01 to the civil date `y-m-d` (proleptic Gregorian).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = i64::from((m + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        _ => 28,
    }
}

fn digits(text: &str, n: usize) -> Option<u32> {
    (text.len() == n && text.bytes().all(|b| b.is_ascii_digit())).then(|| text.parse().ok()).flatten()
}

/// Unix milliseconds of an RFC 3339 date-time (`2026-10-01T09:00:00+08:00`, `…Z`, fractions of a
/// second, a space for the `T`) or date (`2026-10-01`, midnight UTC); `None` for anything else or
/// an instant before 1970.
pub fn parse_instant(text: &str) -> Option<u64> {
    let text = text.trim();
    let (date, rest) = if text.len() > 10 { text.split_at_checked(10)? } else { (text, "") };
    let mut parts = date.split('-');
    let (y, m, d) = (digits(parts.next()?, 4)?, digits(parts.next()?, 2)?, digits(parts.next()?, 2)?);
    let y = i64::from(y);
    if parts.next().is_some() || !(1..=12).contains(&m) || d == 0 || d > days_in_month(y, m) {
        return None;
    }
    let mut ms = days_from_civil(y, m, d) * 86_400_000;
    if !rest.is_empty() {
        let rest = rest.strip_prefix(['T', 't', ' '])?;
        let (clock, offset_ms) = if let Some(clock) = rest.strip_suffix(['Z', 'z']) {
            (clock, 0)
        } else {
            let at = rest.rfind(['+', '-']).filter(|&i| i >= 5)?;
            let (clock, offset) = rest.split_at(at);
            let sign = if offset.starts_with('-') { -1 } else { 1 };
            let offset = &offset[1..];
            let (oh, om) = match offset.split_once(':') {
                Some((h, m)) => (digits(h, 2)?, digits(m, 2)?),
                None => (digits(offset.get(..2)?, 2)?, digits(offset.get(2..)?, 2)?),
            };
            if oh > 23 || om > 59 {
                return None;
            }
            (clock, sign * i64::from(oh * 3600 + om * 60) * 1000)
        };
        let (hms, fraction) = clock.split_once('.').map_or((clock, ""), |(a, b)| (a, b));
        let mut fields = hms.split(':');
        let h = digits(fields.next()?, 2)?;
        let mi = digits(fields.next()?, 2)?;
        let s = match fields.next() {
            Some(s) => digits(s, 2)?,
            None => 0,
        };
        if fields.next().is_some() || h > 23 || mi > 59 || s > 60 || (!fraction.is_empty() && !fraction.bytes().all(|b| b.is_ascii_digit())) {
            return None;
        }
        let frac_ms = fraction.bytes().take(3).enumerate().map(|(i, b)| i64::from(b - b'0') * [100, 10, 1][i]).sum::<i64>();
        ms += i64::from(h * 3600 + mi * 60 + s) * 1000 + frac_ms - offset_ms;
    }
    u64::try_from(ms).ok()
}
