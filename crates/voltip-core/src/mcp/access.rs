//! Which level each tool needs (docs/dictation.md §25) and the refusal when the settings do not
//! grant it. The levels are read at every call, so taking a permission back takes effect at the
//! next call. A refusal says what is shared now and what the user would have to allow, in Chinese
//! and English, for the agent to pass on.

use super::tools::Tool;
use super::{HistoryRange, McpRead, McpSettings, McpWrite, ToolError};

/// What a tool needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    /// At least this read level.
    Read(McpRead),
    /// At least this write level.
    Write(McpWrite),
}

/// The level `tool` needs. `vocabulary_backtest` needs only the lists: its examples need
/// [`McpRead::Summary`] and are left out below it. `rules_update` needs [`McpWrite::Dictionary`]
/// for the waiting rules its own client created and [`McpWrite::EnableRules`] for any other
/// ([`check_rule_change`]).
pub const fn need(tool: Tool) -> Need {
    match tool {
        Tool::DictionaryList | Tool::RulesList | Tool::VocabularyPreview | Tool::VocabularyBacktest => Need::Read(McpRead::Rules),
        Tool::VocabularyCandidates => Need::Read(McpRead::Summary),
        Tool::HistorySearch | Tool::HistoryGet => Need::Read(McpRead::Full),
        Tool::DictionaryAdd | Tool::DictionaryUpdate | Tool::RulesAdd | Tool::RulesUpdate => Need::Write(McpWrite::Dictionary),
    }
}

/// A refusal in both languages, Chinese first.
pub fn bilingual(zh: &str, en: &str) -> String {
    format!("{zh}\n{en}")
}

/// What a read level shares, as a refusal names it.
const fn shared(read: McpRead) -> (&'static str, &'static str) {
    match read {
        McpRead::Rules => ("词典和规则", "the dictionary and rules"),
        McpRead::Summary => ("替换汇总", "the correction summary"),
        McpRead::Full => ("完整历史", "the full history"),
    }
}

/// What a read level adds, as a refusal names what the user would allow.
const fn wanted(read: McpRead) -> (&'static str, &'static str) {
    match read {
        McpRead::Rules => ("词典和规则", "the dictionary and rules"),
        McpRead::Summary => ("替换汇总", "the correction summary"),
        McpRead::Full => ("完整的历史记录", "full history entries"),
    }
}

/// A history range as a refusal names it.
pub const fn range_text(range: HistoryRange) -> (&'static str, &'static str) {
    match range {
        HistoryRange::Last7Days => ("最近 7 天", "the last 7 days"),
        HistoryRange::Last30Days => ("最近 30 天", "the last 30 days"),
        HistoryRange::All => ("全部记录", "all entries"),
    }
}

/// `Ok` when `settings` grant `need`; otherwise the refusal to pass on.
pub fn check(need: Need, settings: &McpSettings) -> Result<(), ToolError> {
    match need {
        Need::Read(level) if settings.read < level => {
            let (now_zh, now_en) = shared(settings.read);
            let (want_zh, want_en) = wanted(level);
            Err(ToolError::Denied(bilingual(
                &format!("当前只允许读取{now_zh}；{want_zh}需要在 Voltip 设置中授权。"),
                &format!("Voltip currently shares {now_en} only; {want_en} must be allowed in Voltip settings."),
            )))
        }
        Need::Write(level) if settings.write < level => Err(ToolError::Denied(bilingual(
            "当前只允许读取，不能修改；修改词典和规则需要在 Voltip 设置中授权。",
            "Voltip currently allows reading only; changing the dictionary and rules must be allowed in Voltip settings.",
        ))),
        _ => Ok(()),
    }
}

/// Whether a client may change a rule at `settings`' write level: any rule with
/// [`McpWrite::EnableRules`]; with [`McpWrite::Dictionary`] only a waiting rule `client` created.
pub fn check_rule_change(settings: &McpSettings, own_waiting_rule: bool) -> Result<(), ToolError> {
    check(Need::Write(McpWrite::Dictionary), settings)?;
    if settings.write >= McpWrite::EnableRules || own_waiting_rule {
        return Ok(());
    }
    Err(ToolError::Denied(bilingual(
        "当前只能修改本工具新建且仍待确认的规则；修改其他规则需要在 Voltip 设置中选择「也可以直接启用规则」。",
        "Voltip currently lets an AI tool change only the rules it created that still await approval; changing other rules needs 「Also turn rules on directly」 in Voltip settings.",
    )))
}

/// The refusal for an entry older than the full history's range.
pub fn out_of_range(range: HistoryRange) -> ToolError {
    let (zh, en) = range_text(range);
    ToolError::Denied(bilingual(
        &format!("这条记录不在允许读取的时间范围（{zh}）内；更早的记录需要在 Voltip 设置中授权。"),
        &format!("This entry is outside the period Voltip shares ({en}); older entries must be allowed in Voltip settings."),
    ))
}
