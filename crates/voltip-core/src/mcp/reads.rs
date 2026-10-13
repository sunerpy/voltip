//! The tools that read (docs/dictation.md §25), run on a blocking thread with a copy of the lists
//! and the history's own read-only connection: what each returns and what the activity notes.
//! The settings were checked before; here they only decide how much a result carries (a
//! backtest's examples, the full history's time window).

use serde_json::{Value, json};

use super::activity::{ActivityDraft, McpAction};
use super::candidates::candidates;
use super::records::{EntryView, HistoryRecord, RuleView, fit_record, fit_records, json_len, page};
use super::tools::{HistoryCursor, LIST_PAGE, McpRequest};
use super::{MAX_RESULT_BYTES, McpRead, McpSettings, ToolError, access};
use crate::history::{HistoryReader, HistorySearch, MAX_RECENT_TEXTS};
use crate::vocabulary::{BacktestDraft, DictionaryEntry, ReplacementRule, backtest, preview};

/// The lists as the core had them when the call came in.
#[derive(Clone, Debug, Default)]
pub struct Lists {
    /// The dictionary, in order.
    pub dictionary: Vec<DictionaryEntry>,
    /// The rules, in order.
    pub rules: Vec<ReplacementRule>,
}

fn failed(e: impl std::fmt::Display) -> ToolError {
    ToolError::Failed(e.to_string())
}

/// The history could not be read: said plainly, without the database's own wording (which can
/// name paths).
fn history_unreadable(e: &crate::CoreError) -> ToolError {
    tracing::warn!(error = %e, "an MCP read of the history failed");
    ToolError::Failed(access::bilingual("历史记录暂时无法读取，请稍后重试。", "The history cannot be read right now; try again later."))
}

/// Run a read. `request` must be one of the reading tools.
pub fn run(request: &McpRequest, settings: &McpSettings, lists: &Lists, reader: &HistoryReader, now_ms: u64) -> Result<(Value, ActivityDraft), ToolError> {
    match request {
        McpRequest::DictionaryList { offset } => {
            let hits = reader.hits().unwrap_or_else(|e| {
                tracing::warn!(error = %e, "hits unavailable for an MCP list");
                crate::history::HistoryHits::default()
            });
            let views: Vec<EntryView> = lists.dictionary.iter().map(|e| EntryView::new(e, hits.dictionary.get(&e.id).copied().unwrap_or(0))).collect();
            let (items, next) = page(&views, *offset, LIST_PAGE);
            let count = items.len();
            Ok((
                json!({ "entries": items, "total": views.len(), "next_cursor": next.map(|n| n.to_string()) }),
                ActivityDraft::new(McpAction::ListDictionary).count(count),
            ))
        }
        McpRequest::RulesList { offset } => {
            let hits = reader.hits().unwrap_or_else(|e| {
                tracing::warn!(error = %e, "hits unavailable for an MCP list");
                crate::history::HistoryHits::default()
            });
            let views: Vec<RuleView> =
                lists.rules.iter().enumerate().map(|(i, r)| RuleView::new(r, i + 1, hits.rules.get(&r.id).copied().unwrap_or(0))).collect();
            let (items, next) = page(&views, *offset, LIST_PAGE);
            let count = items.len();
            Ok((
                json!({ "rules": items, "total": views.len(), "next_cursor": next.map(|n| n.to_string()) }),
                ActivityDraft::new(McpAction::ListRules).count(count),
            ))
        }
        McpRequest::Preview { text, draft } => {
            let answer = preview(&lists.dictionary, &lists.rules, text, draft.as_ref()).map_err(failed)?;
            Ok((serde_json::to_value(answer).map_err(failed)?, ActivityDraft::new(McpAction::Preview)))
        }
        McpRequest::Backtest { draft, exclude, limit } => {
            let texts = reader.recent_texts(*limit).map_err(|e| history_unreadable(&e))?;
            let mut answer = backtest(&lists.dictionary, &texts, draft, *exclude).map_err(failed)?;
            let withheld = settings.read < McpRead::Summary && !answer.examples.is_empty();
            if settings.read < McpRead::Summary {
                answer.examples.clear();
            }
            let subject = match draft {
                BacktestDraft::Dictionary { entry, .. } => entry.term.trim().to_owned(),
                BacktestDraft::Rule { rule, .. } => rule.name.trim().to_owned(),
            };
            let matched = answer.matched as usize;
            let mut value = serde_json::to_value(answer).map_err(failed)?;
            if withheld && let Some(object) = value.as_object_mut() {
                object.insert(
                    "examples_withheld".into(),
                    Value::String(access::bilingual(
                        "示例需要在 Voltip 设置中允许读取替换汇总。",
                        "Examples need the correction summary to be shared in Voltip settings.",
                    )),
                );
            }
            Ok((value, ActivityDraft::new(McpAction::Backtest).subject(subject).count(matched)))
        }
        McpRequest::Candidates { since_ms, limit } => {
            let texts = reader.recent_texts(MAX_RECENT_TEXTS).map_err(|e| history_unreadable(&e))?;
            let mut answer = candidates(&lists.dictionary, &texts, *since_ms, *limit);
            while !answer.pairs.is_empty() && json_len(&answer) > MAX_RESULT_BYTES {
                answer.pairs.pop();
            }
            let count = answer.pairs.len();
            Ok((serde_json::to_value(answer).map_err(failed)?, ActivityDraft::new(McpAction::ReadSummary).count(count)))
        }
        McpRequest::HistorySearch { query, since_ms, until_ms, app, cursor, limit } => {
            let range = settings.history_range;
            let start = range.start(now_ms);
            let since = match (start, *since_ms) {
                (Some(start), Some(since)) => Some(start.max(since)),
                (start, since) => start.or(since),
            };
            let search = HistorySearch {
                since_ms: since,
                until_ms: *until_ms,
                app: app.clone(),
                query: query.clone(),
                after: cursor.map(|c| (c.at_ms, c.row)),
                limit: u32::try_from(*limit).unwrap_or(1),
            };
            let found = reader.search(&search).map_err(|e| history_unreadable(&e))?;
            let records = fit_records(found.entries.iter().map(|hit| HistoryRecord::from(&hit.entry)).collect());
            // A page cut short by the size limit goes on after its last record.
            let next = if records.len() < found.entries.len() {
                found.entries.get(records.len().wrapping_sub(1)).map(|hit| (hit.entry.at_ms, hit.row))
            } else {
                found.next
            };
            let count = records.len();
            let value = json!({
                "entries": records,
                "next_cursor": next.map(|(at_ms, row)| HistoryCursor { at_ms, row }.encode()),
                "range": range.as_str(),
            });
            Ok((value, ActivityDraft::new(McpAction::SearchHistory).count(count).range(range)))
        }
        McpRequest::HistoryGet { id } => {
            let range = settings.history_range;
            let entry = reader
                .entry(*id)
                .map_err(|e| history_unreadable(&e))?
                .ok_or_else(|| ToolError::Failed(access::bilingual("没有这条历史记录（可能已删除）。", "No such history entry (it may have been deleted).")))?;
            if range.start(now_ms).is_some_and(|start| entry.at_ms < start) {
                return Err(access::out_of_range(range));
            }
            let record = fit_record(HistoryRecord::from(&entry));
            Ok((serde_json::to_value(record).map_err(failed)?, ActivityDraft::new(McpAction::ReadEntry).count(1).range(range)))
        }
        McpRequest::DictionaryAdd { .. } | McpRequest::DictionaryUpdate { .. } | McpRequest::RulesAdd { .. } | McpRequest::RulesUpdate { .. } => {
            Err(ToolError::Failed("not a read".into()))
        }
    }
}
