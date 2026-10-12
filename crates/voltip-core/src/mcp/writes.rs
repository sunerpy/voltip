//! What the writing tools do with a call (docs/dictation.md §25), decided on the lists as they
//! are: whether `dictionary_add` creates an entry or adds forms to one with the same spelling,
//! whether a new rule waits for approval, which rules a client may change and how. The core then
//! applies the outcome with the same store methods as the interface's commands, which check the
//! whole list again.

use uuid::Uuid;

use super::tools::RulePatch;
use super::{McpSettings, McpWrite, ToolError, access};
use crate::vocabulary::{DictionaryDraft, DictionaryEntry, EntrySource, ReplacementRule, RuleDraft, validate_dictionary_draft, validate_rule_draft};

/// What `dictionary_add` does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryAdd {
    /// A new entry.
    Create(DictionaryDraft),
    /// Forms added to the entry with the same spelling (ignoring ASCII case).
    Merge {
        /// That entry.
        id: Uuid,
        /// It with the forms added (its spelling and flag stay).
        draft: DictionaryDraft,
        /// The forms added.
        added: Vec<String>,
    },
    /// The entry with that spelling has every form already.
    Unchanged {
        /// That entry.
        id: Uuid,
    },
}

/// The draft of `entry` as it is.
pub fn draft_of(entry: &DictionaryEntry) -> DictionaryDraft {
    DictionaryDraft { term: entry.term.clone(), heard_as: entry.heard_as.clone(), enabled: entry.enabled }
}

fn failed(e: impl std::fmt::Display) -> ToolError {
    ToolError::Failed(e.to_string())
}

/// What `dictionary_add` does with `draft` among `entries`.
pub fn plan_entry_add(entries: &[DictionaryEntry], draft: &DictionaryDraft) -> Result<EntryAdd, ToolError> {
    let draft = validate_dictionary_draft(draft).map_err(failed)?;
    let Some(existing) = entries.iter().find(|e| e.term.eq_ignore_ascii_case(&draft.term)) else { return Ok(EntryAdd::Create(draft)) };
    let added: Vec<String> = draft.heard_as.iter().filter(|h| !existing.heard_as.iter().any(|e| e.eq_ignore_ascii_case(h))).cloned().collect();
    if added.is_empty() {
        return Ok(EntryAdd::Unchanged { id: existing.id });
    }
    let mut merged = draft_of(existing);
    merged.heard_as.extend(added.iter().cloned());
    Ok(EntryAdd::Merge { id: existing.id, draft: merged, added })
}

/// Whether a rule an AI tool creates waits for the user's approval.
pub fn new_rule_waits(settings: &McpSettings) -> bool {
    settings.write < McpWrite::EnableRules
}

/// A rule's draft as it is, with the flag the user set (a waiting rule is off).
pub fn rule_draft_of(rule: &ReplacementRule) -> RuleDraft {
    RuleDraft::from(rule)
}

/// How `rules_update` changes `rule` for `client`: the new draft, and whether the rule keeps
/// waiting (`Some(true)`: changed by its own tool below [`McpWrite::EnableRules`]; `None`: as the
/// interface's change would, turning a waiting rule on approves it). Refused when the client may
/// not change this rule; `Ok(None)` when the patch changes nothing.
pub fn plan_rule_update(
    settings: &McpSettings,
    client: &str,
    rule: &ReplacementRule,
    patch: &RulePatch,
) -> Result<Option<(RuleDraft, Option<bool>)>, ToolError> {
    let own_waiting = rule.pending && matches!(&rule.source, EntrySource::Mcp { client: c } if c == client);
    access::check_rule_change(settings, own_waiting)?;
    let mut draft = validate_rule_draft(&patch.apply(rule_draft_of(rule))).map_err(failed)?;
    let keeps_waiting = settings.write < McpWrite::EnableRules;
    if keeps_waiting {
        draft.enabled = false;
    }
    if draft == rule_draft_of(rule) && (!keeps_waiting || rule.pending) {
        return Ok(None);
    }
    Ok(Some((draft, keeps_waiting.then_some(true))))
}

/// The note a result carries when a tool asked for a rule to be on and it waits for approval
/// instead.
pub fn waiting_note() -> String {
    access::bilingual(
        "这条规则需要用户在 Voltip 的规则页确认后才会生效。",
        "The rule waits for the user to approve it on Voltip's rules page before it takes effect.",
    )
}
