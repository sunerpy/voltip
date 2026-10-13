use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::access::{Need, check, check_rule_change, need, out_of_range};
use super::activity::{ActivityDraft, ActivityLog, MAX_ACTIVITY, McpAction, READ_MERGE_MS, Undo, UndoError};
use super::candidates::{Candidates, MAX_DIFF_CELLS, Segment, SegmentKind, candidates, diff_segments, diff_tokens, is_mark, replacements};
use super::records::{HistoryRecord, fit_record, fit_records, json_len, page};
use super::tools::{HistoryCursor, McpRequest, REVIEW_PROMPT, Tool, parse_call, parse_instant, prompt_text, prompts, specs};
use super::writes::{EntryAdd, new_rule_waits, plan_entry_add, plan_rule_update};
use super::*;
use crate::history::{HistoryEntry, HistoryText};
use crate::vocabulary::{DictionaryDraft, DictionaryEntry, EntrySource, ReplacementRule, RuleDraft, RuleKind, VocabularyHit, VocabularyHits};

fn settings(read: McpRead, write: McpWrite) -> McpSettings {
    McpSettings { enabled: true, read, write, ..McpSettings::default() }
}

fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        other => panic!("not an object: {other}"),
    }
}

fn entry(term: &str, heard: &[&str]) -> DictionaryEntry {
    DictionaryEntry {
        id: Uuid::new_v4(),
        term: term.into(),
        heard_as: heard.iter().map(|h| (*h).to_owned()).collect(),
        enabled: true,
        source: EntrySource::Manual,
        created_at_ms: 1,
        updated_at_ms: 1,
    }
}

fn rule(name: &str, source: EntrySource, pending: bool) -> ReplacementRule {
    ReplacementRule {
        id: Uuid::new_v4(),
        name: name.into(),
        kind: RuleKind::Literal,
        pattern: "非书".into(),
        replacement: "飞书".into(),
        case_sensitive: true,
        enabled: !pending,
        created_at_ms: 1,
        updated_at_ms: 1,
        source,
        pending,
    }
}

fn take(raw: &str, text: &str, at_ms: u64) -> HistoryText {
    HistoryText { id: Uuid::new_v4(), at_ms, raw_text: raw.into(), text: text.into(), refined: true, vocabulary: VocabularyHits::default() }
}

#[test]
fn settings_default_to_off_on_their_own_port_and_old_files_read_as_defaults() {
    let defaults = McpSettings::default();
    assert_eq!(
        (defaults.enabled, defaults.port, defaults.read, defaults.history_range, defaults.write),
        (false, 47841, McpRead::Summary, HistoryRange::Last30Days, McpWrite::Dictionary)
    );
    assert_eq!(
        serde_json::to_value(&defaults).unwrap(),
        json!({ "enabled": false, "port": 47841, "read": "summary", "history_range": "last_30_days", "write": "dictionary" })
    );
    let partial: McpSettings = serde_json::from_value(json!({ "enabled": true, "read": "full", "history_range": "last_7_days" })).unwrap();
    assert_eq!(
        (partial.enabled, partial.port, partial.read, partial.history_range, partial.write),
        (true, 47841, McpRead::Full, HistoryRange::Last7Days, McpWrite::Dictionary)
    );
    for (range, wire) in [(HistoryRange::Last7Days, "last_7_days"), (HistoryRange::Last30Days, "last_30_days"), (HistoryRange::All, "all")] {
        assert_eq!(serde_json::to_value(range).unwrap(), wire);
        assert_eq!(range.as_str(), wire);
    }
    for (write, wire) in [(McpWrite::ReadOnly, "read_only"), (McpWrite::Dictionary, "dictionary"), (McpWrite::EnableRules, "enable_rules")] {
        assert_eq!(serde_json::to_value(write).unwrap(), wire);
    }
    // A settings file from before the service has no `mcp`: it reads as off.
    let old: crate::Settings = serde_json::from_value(json!({ "schema": 1, "theme": "glass", "follow_system_theme": false, "relay_enabled": true })).unwrap();
    assert_eq!(old.mcp, McpSettings::default());
    assert_eq!(address(47841), "http://127.0.0.1:47841/mcp");
}

#[test]
fn the_range_reaches_back_seven_or_thirty_days_or_all_the_way() {
    let day = 24 * 60 * 60 * 1000;
    let now = 100 * day;
    assert_eq!(HistoryRange::Last7Days.start(now), Some(93 * day));
    assert_eq!(HistoryRange::Last30Days.start(now), Some(70 * day));
    assert_eq!(HistoryRange::All.start(now), None);
    assert_eq!(HistoryRange::Last30Days.start(day), Some(0), "never before 1970");
}

#[test]
fn client_names_are_cleaned_and_capped() {
    assert_eq!(clean_client_name("  claude-code  "), "claude-code");
    assert_eq!(clean_client_name("Claude\tCode\n"), "Claude Code");
    assert_eq!(clean_client_name("a\u{202e}b\u{200b}c\u{7}"), "abc", "no direction overrides, zero-width or control characters");
    assert_eq!(clean_client_name(""), UNNAMED_CLIENT);
    assert_eq!(clean_client_name("\u{200b} \t"), UNNAMED_CLIENT);
    let long = "名".repeat(MAX_CLIENT_CHARS + 5);
    assert_eq!(clean_client_name(&long).chars().count(), MAX_CLIENT_CHARS);
    assert_eq!(clean_client_name(&format!("{} x", "a".repeat(MAX_CLIENT_CHARS - 1))), "a".repeat(MAX_CLIENT_CHARS - 1), "no space left at the end of a cut");
}

/// Every tool needs one level; a refusal says what is shared and what to allow, in both
/// languages.
#[test]
fn each_tool_needs_its_level_and_a_refusal_says_what_to_allow() {
    let lists_only = settings(McpRead::Rules, McpWrite::ReadOnly);
    let summary = settings(McpRead::Summary, McpWrite::Dictionary);
    let full = settings(McpRead::Full, McpWrite::EnableRules);
    for tool in Tool::ALL {
        assert!(check(need(tool), &full).is_ok(), "{tool:?} at the top level");
        let at_lists = check(need(tool), &lists_only);
        match tool {
            Tool::DictionaryList | Tool::RulesList | Tool::VocabularyPreview | Tool::VocabularyBacktest => assert!(at_lists.is_ok(), "{tool:?}"),
            _ => assert!(matches!(at_lists, Err(ToolError::Denied(_))), "{tool:?}"),
        }
        let at_summary = check(need(tool), &summary);
        assert_eq!(at_summary.is_ok(), !matches!(tool, Tool::HistorySearch | Tool::HistoryGet), "{tool:?}");
    }
    let ToolError::Denied(text) = check(Need::Read(McpRead::Full), &summary).unwrap_err() else { panic!() };
    assert_eq!(
        text,
        "当前只允许读取替换汇总；完整的历史记录需要在 Voltip 设置中授权。\nVoltip currently shares the correction summary only; full history entries must be allowed in Voltip settings."
    );
    let ToolError::Denied(text) = check(Need::Read(McpRead::Summary), &lists_only).unwrap_err() else { panic!() };
    assert!(text.starts_with("当前只允许读取词典和规则；替换汇总需要在 Voltip 设置中授权。"), "{text}");
    let ToolError::Denied(text) = check(Need::Write(McpWrite::Dictionary), &lists_only).unwrap_err() else { panic!() };
    assert!(text.contains("只允许读取") && text.contains("reading only"), "{text}");
    let ToolError::Denied(text) = out_of_range(HistoryRange::Last7Days) else { panic!() };
    assert!(text.contains("最近 7 天") && text.contains("the last 7 days"), "{text}");
}

#[test]
fn only_a_waiting_rule_of_the_same_tool_may_change_below_enable_rules() {
    let dictionary = settings(McpRead::Summary, McpWrite::Dictionary);
    assert!(check_rule_change(&dictionary, true).is_ok());
    let ToolError::Denied(text) = check_rule_change(&dictionary, false).unwrap_err() else { panic!() };
    assert!(text.contains("也可以直接启用规则") && text.contains("Also turn rules on directly"), "{text}");
    assert!(check_rule_change(&settings(McpRead::Summary, McpWrite::EnableRules), false).is_ok());
    assert!(check_rule_change(&settings(McpRead::Summary, McpWrite::ReadOnly), true).is_err(), "read-only refuses even its own rule");
}

#[test]
fn the_tools_are_listed_once_each_with_an_object_schema_and_honest_hints() {
    let specs = specs();
    assert_eq!(specs.len(), Tool::ALL.len());
    let mut names: Vec<&str> = specs.iter().map(|s| s.name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), specs.len());
    assert_eq!(
        specs.iter().map(|s| s.name).collect::<Vec<_>>(),
        [
            "dictionary_list",
            "rules_list",
            "vocabulary_preview",
            "vocabulary_backtest",
            "vocabulary_candidates",
            "history_search",
            "history_get",
            "dictionary_add",
            "dictionary_update",
            "rules_add",
            "rules_update"
        ]
    );
    for spec in &specs {
        assert_eq!(spec.input_schema["type"], "object", "{}", spec.name);
        assert_eq!(spec.input_schema["additionalProperties"], false, "{}", spec.name);
        assert!(!spec.description.is_empty() && !spec.title.is_empty());
        let tool = Tool::named(spec.name).unwrap();
        assert_eq!(spec.read_only, tool.read_only());
        assert!(spec.idempotent || spec.name == "rules_add", "{}", spec.name);
    }
    assert!(!Tool::DictionaryAdd.read_only() && Tool::HistoryGet.read_only());
    assert_eq!(Tool::named("dictionary_delete"), None, "there is no deleting tool");
    assert_eq!(prompts()[0].name, REVIEW_PROMPT);
}

#[test]
fn arguments_parse_into_requests_with_defaults_and_bounds() {
    let call = |name: &str, args: Value| parse_call(name, object(args));
    assert_eq!(call("dictionary_list", json!({})).unwrap(), McpRequest::DictionaryList { offset: 0 });
    assert_eq!(call("rules_list", json!({ "cursor": "100" })).unwrap(), McpRequest::RulesList { offset: 100 });
    assert!(matches!(call("rules_list", json!({ "cursor": "x" })), Err(ToolError::InvalidArguments(_))));
    assert!(matches!(call("dictionary_list", json!({ "page": 2 })), Err(ToolError::InvalidArguments(m)) if m.contains("unknown field")));
    assert!(matches!(call("dictionary_remove", json!({})), Err(ToolError::Unknown(m)) if m.contains("dictionary_remove")));
    let McpRequest::Backtest { limit, exclude, .. } =
        call("vocabulary_backtest", json!({ "draft": { "kind": "dictionary", "entry": { "term": "Voltip", "heard_as": ["维他命"] } } })).unwrap()
    else {
        panic!()
    };
    assert_eq!((limit, exclude), (crate::vocabulary::BACKTEST_LIMIT, None));
    assert!(call("vocabulary_backtest", json!({ "draft": { "kind": "rule", "rule": { "name": "a", "pattern": "x" } }, "limit": 2001 })).is_err());
    assert_eq!(
        call("vocabulary_candidates", json!({ "since": "2026-10-01", "limit": 5 })).unwrap(),
        McpRequest::Candidates { since_ms: Some(1_790_812_800_000), limit: 5 }
    );
    assert!(call("vocabulary_candidates", json!({ "limit": 0 })).is_err());
    assert!(call("vocabulary_candidates", json!({ "since": "yesterday" })).is_err());
    let McpRequest::HistorySearch { query, since_ms, until_ms, app, cursor, limit } =
        call("history_search", json!({ "query": "  飞书 ", "until": 5, "app": " ", "cursor": "7:3" })).unwrap()
    else {
        panic!()
    };
    assert_eq!((query.as_deref(), since_ms, until_ms, app, cursor, limit), (Some("飞书"), None, Some(5), None, Some(HistoryCursor { at_ms: 7, row: 3 }), 20));
    assert!(call("history_search", json!({ "limit": 51 })).is_err(), "at most 50 entries a page");
    assert!(call("history_search", json!({ "cursor": "garbage" })).is_err());
    assert!(call("history_get", json!({ "id": "not a uuid" })).is_err());
    let McpRequest::DictionaryAdd { draft } = call("dictionary_add", json!({ "term": "Voltip", "heard_as": ["维他命"] })).unwrap() else { panic!() };
    assert!(draft.enabled, "on unless said otherwise");
    let McpRequest::RulesAdd { draft } = call("rules_add", json!({ "name": "飞书", "pattern": "非书", "replacement": "飞书" })).unwrap() else {
        panic!()
    };
    assert_eq!((draft.kind, draft.case_sensitive, draft.enabled), (RuleKind::Literal, true, true), "the rules page's defaults");
    let id = Uuid::new_v4();
    let McpRequest::RulesUpdate { patch, .. } = call("rules_update", json!({ "id": id, "enabled": true })).unwrap() else { panic!() };
    assert_eq!((patch.enabled, patch.name), (Some(true), None));
    assert_eq!(HistoryCursor { at_ms: 9, row: -1 }.encode(), "9:-1");
}

#[test]
fn instants_are_milliseconds_or_rfc_3339() {
    assert_eq!(parse_instant("1970-01-01"), Some(0));
    assert_eq!(parse_instant("2026-10-01"), Some(1_790_812_800_000));
    assert_eq!(parse_instant("2026-10-01T09:00:00+08:00"), Some(1_790_816_400_000));
    assert_eq!(parse_instant("2026-10-01T01:00:00Z"), Some(1_790_816_400_000));
    assert_eq!(parse_instant("2026-10-01 01:00:00.250z"), Some(1_790_816_400_250));
    assert_eq!(parse_instant("2026-10-01T01:00Z"), Some(1_790_816_400_000), "seconds may be left out");
    assert_eq!(parse_instant("2026-09-30T20:00:00-0500"), Some(1_790_816_400_000));
    assert_eq!(parse_instant("2024-02-29"), Some(1_709_164_800_000), "a leap day");
    for bad in [
        "",
        "2026-13-01",
        "2026-02-30",
        "2025-02-29",
        "2026-1-01",
        "2026-10-01T25:00:00Z",
        "2026-10-01T09:00:00",
        "1969-12-31",
        "2026-10-01T09:00:00+24:00",
        "x",
    ] {
        assert_eq!(parse_instant(bad), None, "{bad:?}");
    }
}

#[test]
fn the_review_prompt_takes_a_number_of_days() {
    let text = prompt_text(REVIEW_PROMPT, &Map::new()).unwrap();
    assert!(text.contains("last 30 days") && text.contains("vocabulary_candidates") && text.contains("vocabulary_backtest"), "{text}");
    assert!(prompt_text(REVIEW_PROMPT, &object(json!({ "days": "7" }))).unwrap().contains("last 7 days"));
    assert!(prompt_text(REVIEW_PROMPT, &object(json!({ "days": 90 }))).unwrap().contains("last 90 days"));
    assert!(matches!(prompt_text(REVIEW_PROMPT, &object(json!({ "days": 0 }))), Err(ToolError::InvalidArguments(_))));
    assert!(matches!(prompt_text("other", &Map::new()), Err(ToolError::Unknown(_))));
}

fn seg(kind: SegmentKind, text: &str) -> Segment {
    Segment { kind, text: text.into() }
}

/// The tokens and the diff of `refine.ts`: words with their trailing spaces, single CJK
/// characters, punctuation; ties go to the deletion.
#[test]
fn the_diff_is_the_history_pages() {
    assert_eq!(diff_tokens("用 fetchUser 改成async，好"), ["用", " ", "fetchUser ", "改", "成", "async", "，", "好"]);
    assert_eq!(diff_tokens("a  b\u{3000}c"), ["a  ", "b\u{3000}", "c"]);
    assert_eq!(diff_tokens("  x"), ["  ", "x"]);
    assert_eq!(diff_tokens("v1.2-rc/x"), ["v1.2-rc/x"]);
    assert!(diff_tokens("").is_empty());
    use SegmentKind::{Add, Del, Same};
    assert_eq!(
        diff_segments("今天用维他命测了", "今天用 Voltip 测了。").unwrap(),
        vec![seg(Same, "今天用"), seg(Del, "维他命"), seg(Add, " Voltip "), seg(Same, "测了"), seg(Add, "。")]
    );
    assert_eq!(diff_segments("甲乙", "丙乙").unwrap(), vec![seg(Del, "甲"), seg(Add, "丙"), seg(Same, "乙")]);
    assert_eq!(diff_segments("ab c", "xb c").unwrap(), vec![seg(Del, "ab "), seg(Add, "xb "), seg(Same, "c")], "a word is one token");
    assert_eq!(diff_segments("", "").unwrap(), Vec::<Segment>::new());
    let long = "字".repeat(1001);
    assert!(diff_segments(&long, &long).is_none(), "{} cells", 1002 * 1002 > MAX_DIFF_CELLS);
}

#[test]
fn replacements_are_trimmed_pairs_without_punctuation_only_changes() {
    let found = replacements("发到非书群，然后维他命，", "发到飞书群。然后 Voltip。").unwrap();
    let pairs: Vec<(&str, &str, usize)> = found.iter().map(|r| (r.heard.as_str(), r.polished.as_str(), r.at)).collect();
    assert_eq!(pairs, [("非", "飞", 2), ("维他命", "Voltip", 8)]);
    assert!(replacements("你好，", "你好。").unwrap().is_empty(), "punctuation only");
    assert!(replacements("嗯好的", "好的").unwrap().is_empty(), "a deletion is no pair");
    assert!(replacements("abc", "ABC").unwrap().iter().any(|r| r.heard == "abc" && r.polished == "ABC"), "a case change is a pair");
    let long = "长".repeat(70);
    assert!(replacements(&format!("说{long}了"), "说短了").unwrap().is_empty(), "longer than a dictionary form");
    for c in ['，', '。', '、', '「', '」', '…', '—', '!', '?', '（', '·'] {
        assert!(is_mark(c), "{c}");
    }
    for c in ['好', 'a', '1', ' '] {
        assert!(!is_mark(c), "{c:?}");
    }
}

#[test]
fn candidates_are_counted_known_and_sorted() {
    let dictionary = vec![entry("Voltip", &["维他命"])];
    // The diff works on single CJK characters, as the history page's: 非书 → 飞书 is 非 → 飞, and
    // the snippets show the word around it.
    let history = vec![
        take("我们用维他命开会", "我们用 Voltip 开会。", 50),
        take("发到非书群", "发到飞书群。", 40),
        take("又是非书", "又是飞书。", 30),
        take("维他命", "Voltip", 20),
        take("非书非书", "飞书飞书", 10),
    ];
    let out = candidates(&dictionary, &[], &history, None, 10);
    assert_eq!((out.scanned, out.skipped), (5, 0));
    let top: Vec<(&str, &str, u32, bool, u64)> = out.pairs.iter().map(|p| (p.heard.as_str(), p.polished.as_str(), p.count, p.known, p.last_at_ms)).collect();
    assert_eq!(top, [("非", "飞", 4, false, 40), ("维他命", "Voltip", 2, true, 50)], "every occurrence counts");
    let examples = &out.pairs[0].examples;
    assert_eq!(examples.iter().map(|e| e.history_id).collect::<Vec<_>>(), [history[1].id, history[2].id, history[4].id], "one per take, newest first");
    assert_eq!(examples[0].snippet, "发到非书群");
    assert_eq!(out.pairs[1].examples[0].snippet, "我们用维他命开会");
    let recent = candidates(&dictionary, &[], &history, Some(35), 10);
    assert_eq!(recent.scanned, 2, "since");
    assert_eq!(candidates(&dictionary, &[], &history, None, 1).pairs.len(), 1, "limit");
    let long = take(&"字".repeat(1001), &"子".repeat(1001), 5);
    let out = candidates(&dictionary, &[], &[long], None, 10);
    assert_eq!((out.scanned, out.skipped, out.pairs.len()), (1, 1, 0), "a take too long to diff is counted as skipped");
}

/// Codex review 2026-10-13: a take the clean-up did not polish, or a rule's change in one it did,
/// was summed up as a replacement the AI clean-up made.
#[test]
fn regression_candidates_leave_out_what_the_dictionary_and_rules_changed() {
    let feishu = rule("非书 → 飞书", EntrySource::Manual, false);
    let voltip = entry("Voltip", &["维他命"]);
    let hits = |corrections: &[Uuid], rules: &[Uuid]| VocabularyHits {
        corrections: corrections.iter().map(|&id| VocabularyHit { id, count: 1 }).collect(),
        rules: rules.iter().map(|&id| VocabularyHit { id, count: 1 }).collect(),
    };
    let unpolished = HistoryText { refined: false, vocabulary: hits(&[], &[feishu.id]), ..take("发到非书群", "发到飞书群", 30) };
    let polished = HistoryText { vocabulary: hits(&[], &[feishu.id]), ..take("发到非书群，用维他命测", "发到飞书群，用 Voltip 测。", 20) };
    let pairs = |out: Candidates| out.pairs.into_iter().map(|p| (p.heard, p.polished, p.count)).collect::<Vec<_>>();
    let both = [unpolished.clone(), polished.clone()];
    let out = candidates(&[], std::slice::from_ref(&feishu), &both, None, 10);
    assert_eq!(out.scanned, 2, "the unpolished take was looked at");
    assert_eq!(pairs(out), [("维他命".to_owned(), "Voltip".to_owned(), 1)], "only the polish's own replacement");
    // A rule that is off now but unchanged since the take still made that change; with nothing
    // fired the polish made both.
    let off = ReplacementRule { enabled: false, ..feishu.clone() };
    assert_eq!(pairs(candidates(&[], &[off], &both, None, 10)).len(), 1);
    let nothing = HistoryText { vocabulary: VocabularyHits::default(), ..polished };
    assert_eq!(pairs(candidates(&[], std::slice::from_ref(&feishu), &[nothing], None, 10)).len(), 2);
    // A correction's change, of an entry off now but unchanged since the take.
    let corrected = HistoryText { vocabulary: hits(&[voltip.id], &[]), ..take("用维他命测", "用 Voltip 测。", 10) };
    let off_entry = DictionaryEntry { enabled: false, ..voltip };
    assert!(candidates(&[off_entry], &[], &[corrected], None, 10).pairs.is_empty());
    assert!(candidates(&[], &[], &[unpolished], None, 10).pairs.is_empty(), "not polished: nothing, whatever fired");
}

/// Codex review 2026-10-13 (round 3): the fired entries and rules are replayed as they are now, so
/// one deleted or changed after the take would hand its change back to the polish; such a take
/// gives no pairs.
#[test]
fn regression_candidates_skip_a_take_whose_fired_vocabulary_changed_since() {
    let feishu = rule("非书 → 飞书", EntrySource::Manual, false);
    let voltip = entry("Voltip", &["维他命"]);
    let fired = VocabularyHits { corrections: vec![VocabularyHit { id: voltip.id, count: 1 }], rules: vec![VocabularyHit { id: feishu.id, count: 1 }] };
    let polished =
        HistoryText { vocabulary: fired, ..take("发到非书群，用维他命测，跑千问", "发到飞书群，用 Voltip 测，跑 Qwen。", 20) };
    let heard = |dictionary: &[DictionaryEntry], rules: &[ReplacementRule]| {
        candidates(dictionary, rules, std::slice::from_ref(&polished), None, 10).pairs.into_iter().map(|p| p.heard).collect::<Vec<_>>()
    };
    let (dictionary, rules) = (std::slice::from_ref(&voltip), std::slice::from_ref(&feishu));
    assert_eq!(heard(dictionary, rules), ["千问"], "as they stood at the take: only the polish's own");
    assert!(heard(dictionary, &[]).is_empty(), "the rule deleted since");
    assert!(heard(&[], rules).is_empty(), "the entry deleted since");
    let edited = ReplacementRule { pattern: "非输".into(), updated_at_ms: 21, ..feishu.clone() };
    assert!(heard(dictionary, &[edited]).is_empty(), "the rule edited since");
    let disabled = DictionaryEntry { enabled: false, updated_at_ms: 21, ..voltip.clone() };
    assert!(heard(&[disabled], rules).is_empty(), "the entry turned off since");
    let at_the_take = ReplacementRule { updated_at_ms: 20, ..feishu.clone() };
    assert_eq!(heard(dictionary, &[at_the_take]), ["千问"], "changed at the take's own instant: still as it fired");
}

fn history_entry(raw: &str, text: &str) -> HistoryEntry {
    serde_json::from_value(json!({
        "id": Uuid::new_v4(), "at_ms": 7, "raw_text": raw, "text": text, "refined": true, "asr_model": "m", "refine_model": "r",
        "duration_ms": 1200, "asr_ms": 1, "outcome": { "kind": "clipboard", "reason": "no window", "code": "other" },
        "kind": "edit", "edit": { "instruction": "改正式", "selection": "secret selection" },
        "app": { "id": "code", "name": "Code" }, "scene": { "id": Uuid::new_v4(), "name": "代码评审" },
        "processed": { "text": "处理后", "preset": { "id": "notes", "name": "笔记" }, "at_ms": 9 }
    }))
    .unwrap()
}

/// A full history entry is shared without the selection a voice edit rewrote and without the
/// failure's wording.
#[test]
fn a_history_record_shares_the_texts_and_names_only() {
    let entry = history_entry("原文", "终稿");
    let record = HistoryRecord::from(&entry);
    let json = serde_json::to_value(&record).unwrap();
    assert_eq!(json["raw_text"], "原文");
    assert_eq!(json["text"], "终稿");
    assert_eq!(json["processed_text"], "处理后");
    assert_eq!(json["outcome"], "clipboard");
    assert_eq!(json["app"], json!({ "id": "code", "name": "Code" }));
    assert_eq!(json["scene"], "代码评审");
    assert_eq!(json["kind"], "edit");
    let text = json.to_string();
    assert!(!text.contains("secret selection") && !text.contains("no window") && !text.contains("改正式"), "{text}");
    assert!(json.get("truncated").is_none());
}

#[test]
fn results_stay_under_the_size_limit() {
    let big = "长".repeat(60_000);
    let record = fit_record(HistoryRecord::from(&history_entry(&big, &big)));
    assert!(record.truncated && json_len(&record) <= MAX_RESULT_BYTES, "{}", json_len(&record));
    assert!(record.raw_text.ends_with('…'));
    let records = fit_records(vec![HistoryRecord::from(&history_entry(&big, &big)), HistoryRecord::from(&history_entry(&big, &big))]);
    assert_eq!(records.len(), 1, "the first is shortened, the rest wait for the next page");
    assert!(records[0].truncated);
    let records = fit_records(vec![HistoryRecord::from(&history_entry(&big, &big)), HistoryRecord::from(&history_entry("a", "b"))]);
    assert_eq!(records.len(), 2, "a small one still fits beside a shortened one");
    assert!(json_len(&records) <= MAX_RESULT_BYTES);
    let small = fit_records((0..5).map(|_| HistoryRecord::from(&history_entry("a", "b"))).collect());
    assert_eq!(small.len(), 5);
    assert!(!small[0].truncated);
    let items: Vec<String> = (0..10).map(|i| format!("{i}{}", "x".repeat(40_000))).collect();
    let (first, next) = page(&items, 0, 100);
    assert!(first.len() < 10 && next == Some(first.len()), "{} items", first.len());
    let (rest, end) = page(&items, first.len(), 100);
    assert!(!rest.is_empty() && (end.is_none() || end > Some(first.len())));
    assert_eq!(page(&items, 99, 100), (Vec::<String>::new(), None));
    assert_eq!(page(&items[..3], 0, 2).1, Some(2), "the page size");
}

fn activity(action: McpAction, client: &str, at_ms: u64) -> super::McpActivity {
    ActivityDraft::new(action).into_activity(client, at_ms)
}

#[test]
fn the_activity_keeps_fifty_lines_merges_repeated_reads_and_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let (mut log, notice) = ActivityLog::open(dir.path());
    assert!(notice.is_none() && log.activities().is_empty());
    let first = log.record(activity(McpAction::ListDictionary, "Claude Code", 1_000), None);
    let again = log.record(activity(McpAction::ListDictionary, "Claude Code", 2_000), None);
    assert_eq!(log.activities().len(), 1, "the same read again replaces the line");
    assert_eq!(log.activities()[0].id, again.id);
    assert_ne!(first.id, again.id);
    log.record(activity(McpAction::ListDictionary, "Codex", 2_500), None);
    log.record(activity(McpAction::ListDictionary, "Claude Code", 2_000 + READ_MERGE_MS), None);
    assert_eq!(log.activities().len(), 3, "another client, or long after, is a line of its own");
    let after = entry("Voltip", &["维他命"]);
    let write =
        log.record(ActivityDraft::new(McpAction::AddEntry).subject("Voltip").into_activity("Claude Code", 3_000), Some(Undo::Entry { before: None, after }));
    assert!(write.undoable && !write.undone);
    assert!(matches!(log.undo_of(write.id), Ok(Undo::Entry { before: None, .. })));
    assert_eq!(log.undo_of(first.id).unwrap_err(), UndoError::Unknown, "merged away");
    assert_eq!(log.undo_of(again.id).unwrap_err(), UndoError::NotUndoable, "a read");
    let read = log.activities()[1].id;
    assert_eq!(log.undo_of(read).unwrap_err(), UndoError::NotUndoable);
    // A restart reads the same lines, the undo included.
    let (reopened, _) = ActivityLog::open(dir.path());
    assert_eq!(reopened.activities(), log.activities());
    assert!(reopened.undo_of(write.id).is_ok());
    let text = std::fs::read_to_string(dir.path().join("mcp/activity.json")).unwrap();
    assert!(text.contains("\"schema\": 1") && text.contains("add_entry"), "{text}");
    log.mark_undone(write.id);
    assert_eq!(log.undo_of(write.id).unwrap_err(), UndoError::AlreadyUndone);
    assert!(log.activities()[0].undone && !log.activities()[0].undoable);
    for i in 0..(MAX_ACTIVITY + 5) {
        log.record(ActivityDraft::new(McpAction::Backtest).subject(format!("草稿 {i}")).into_activity("Codex", 10_000 + i as u64), None);
    }
    assert_eq!(log.activities().len(), MAX_ACTIVITY);
    assert_eq!(log.activities()[0].subject.as_deref(), Some(format!("草稿 {}", MAX_ACTIVITY + 4).as_str()), "newest first");
    // A broken file is moved aside, never deleted, and the log starts empty.
    std::fs::write(dir.path().join("mcp/activity.json"), b"{broken").unwrap();
    let (broken, notice) = ActivityLog::open(dir.path());
    assert!(broken.activities().is_empty() && notice.unwrap().contains("activity.json"));
    assert!(std::fs::read_dir(dir.path().join("mcp")).unwrap().flatten().any(|e| e.file_name().to_string_lossy().starts_with("activity.json.corrupt-")));
}

#[test]
fn the_activity_wire_names_names_and_counts_only() {
    let mut a = ActivityDraft::new(McpAction::SearchHistory).count(12).range(HistoryRange::Last30Days).into_activity("Codex", 5);
    a.id = Uuid::nil();
    assert_eq!(
        serde_json::to_value(&a).unwrap(),
        json!({ "id": Uuid::nil(), "at_ms": 5, "client": "Codex", "action": "search_history", "count": 12, "range": "last_30_days", "undoable": false, "undone": false })
    );
    let rule = ActivityDraft::new(McpAction::AddRule).subject("非书 → 飞书").pending(true).into_activity("Claude Code", 6);
    assert_eq!(serde_json::to_value(&rule).unwrap()["pending"], true);
    for action in [McpAction::AddEntry, McpAction::MergeEntry, McpAction::UpdateEntry, McpAction::AddRule, McpAction::UpdateRule] {
        assert!(action.writes());
    }
    for action in [
        McpAction::ListDictionary,
        McpAction::ListRules,
        McpAction::Preview,
        McpAction::Backtest,
        McpAction::ReadSummary,
        McpAction::SearchHistory,
        McpAction::ReadEntry,
    ] {
        assert!(!action.writes());
    }
}

#[test]
fn dictionary_add_creates_merges_or_changes_nothing() {
    let voltip = entry("Voltip", &["维他命"]);
    let draft = |term: &str, heard: &[&str]| DictionaryDraft { term: term.into(), heard_as: heard.iter().map(|h| (*h).to_owned()).collect(), enabled: true };
    assert!(matches!(plan_entry_add(std::slice::from_ref(&voltip), &draft("飞书", &["非书"])).unwrap(), EntryAdd::Create(d) if d.term == "飞书"));
    let EntryAdd::Merge { id, draft: merged, added } =
        plan_entry_add(std::slice::from_ref(&voltip), &draft("voltip", &["维他命", "沃提普", "VOLTIP1"])).unwrap()
    else {
        panic!()
    };
    assert_eq!(
        (id, merged.term.as_str(), merged.heard_as.as_slice(), added.as_slice()),
        (voltip.id, "Voltip", &["维他命".to_owned(), "沃提普".into(), "VOLTIP1".into()][..], &["沃提普".to_owned(), "VOLTIP1".into()][..])
    );
    assert_eq!(plan_entry_add(std::slice::from_ref(&voltip), &draft("Voltip", &["维他命"])).unwrap(), EntryAdd::Unchanged { id: voltip.id }, "idempotent");
    assert!(matches!(plan_entry_add(&[], &draft(" ", &[])), Err(ToolError::Failed(m)) if m.starts_with("dictionary:")));
}

#[test]
fn a_rule_change_keeps_waiting_below_enable_rules() {
    let dictionary = settings(McpRead::Summary, McpWrite::Dictionary);
    let enable = settings(McpRead::Summary, McpWrite::EnableRules);
    assert!(new_rule_waits(&dictionary) && !new_rule_waits(&enable));
    let mine = rule("非书 → 飞书", EntrySource::Mcp { client: "Claude Code".into() }, true);
    let patch = super::tools::RulePatch { replacement: Some("飞书 Lark".into()), enabled: Some(true), ..Default::default() };
    let (draft, pending) = plan_rule_update(&dictionary, "Claude Code", &mine, &patch).unwrap().unwrap();
    assert_eq!((draft.replacement.as_str(), draft.enabled, pending), ("飞书 Lark", false, Some(true)), "still waiting, still off");
    assert!(matches!(plan_rule_update(&dictionary, "Codex", &mine, &patch), Err(ToolError::Denied(_))), "another tool's rule");
    let manual = rule("git push", EntrySource::Manual, false);
    assert!(matches!(plan_rule_update(&dictionary, "Claude Code", &manual, &patch), Err(ToolError::Denied(_))), "the user's rule");
    let (draft, pending) = plan_rule_update(&enable, "Codex", &manual, &patch).unwrap().unwrap();
    assert_eq!((draft.enabled, pending), (true, None));
    let (draft, pending) =
        plan_rule_update(&enable, "Claude Code", &mine, &super::tools::RulePatch { enabled: Some(true), ..Default::default() }).unwrap().unwrap();
    assert_eq!((draft.enabled, pending), (true, None), "turning it on approves it");
    assert_eq!(plan_rule_update(&enable, "Codex", &manual, &super::tools::RulePatch::default()).unwrap(), None, "nothing to change");
    assert_eq!(plan_rule_update(&dictionary, "Claude Code", &mine, &super::tools::RulePatch { enabled: Some(true), ..Default::default() }).unwrap(), None);
    let bad = super::tools::RulePatch { kind: Some(RuleKind::Regex), pattern: Some("(".into()), ..Default::default() };
    assert!(matches!(plan_rule_update(&enable, "Codex", &manual, &bad), Err(ToolError::Failed(m)) if m.contains("正则无法编译")));
    let _: RuleDraft = super::writes::rule_draft_of(&mine);
}

#[test]
fn the_setup_commands_carry_the_address_and_the_token() {
    assert_eq!(
        setup_command(SetupClient::ClaudeCode, "http://127.0.0.1:47841/mcp", "tok", false),
        "claude mcp add --transport http voltip http://127.0.0.1:47841/mcp --header \"Authorization: Bearer tok\""
    );
    assert_eq!(
        setup_command(SetupClient::Codex, "http://127.0.0.1:47841/mcp", "tok", false),
        "export VOLTIP_MCP_TOKEN=tok\ncodex mcp add voltip --url http://127.0.0.1:47841/mcp --bearer-token-env-var VOLTIP_MCP_TOKEN"
    );
    assert!(setup_command(SetupClient::Codex, "http://127.0.0.1:1/mcp", "tok", true).starts_with("$env:VOLTIP_MCP_TOKEN = \"tok\"\n"));
    assert_eq!(serde_json::to_value(SetupClient::ClaudeCode).unwrap(), "claude_code");
}

/// A history that cannot be read (a file that is no database) is said plainly, without the
/// database's words or its path; the lists still answer, without hits.
#[test]
fn reads_say_plainly_when_the_history_cannot_be_read() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite3");
    std::fs::write(&path, "this is no database, only text long enough to be read as a header of one".repeat(4)).unwrap();
    let reader = crate::history::HistoryReader::at(&path);
    let lists = reads::Lists { dictionary: vec![entry("Voltip", &["沃提普"])], rules: vec![rule("飞书", EntrySource::Manual, false)] };
    let full = settings(McpRead::Full, McpWrite::Dictionary);
    let draft = crate::vocabulary::BacktestDraft::Dictionary {
        id: None,
        entry: DictionaryDraft { term: "Voltip".into(), heard_as: vec!["沃提普".into()], enabled: true },
    };
    for request in [
        McpRequest::Backtest { draft, exclude: None, limit: 10 },
        McpRequest::Candidates { since_ms: None, limit: 10 },
        McpRequest::HistorySearch { query: None, since_ms: None, until_ms: None, app: None, cursor: None, limit: 10 },
        McpRequest::HistoryGet { id: Uuid::new_v4() },
    ] {
        match reads::run(&request, &full, &lists, &reader, 1_000) {
            Err(ToolError::Failed(message)) => {
                assert!(message.starts_with("历史记录暂时无法读取，请稍后重试。"), "{message}");
                assert!(!message.contains(path.to_str().unwrap()) && !message.contains("database"), "{message}");
            }
            other => panic!("{request:?}: {other:?}"),
        }
    }
    let (listed, note) = reads::run(&McpRequest::DictionaryList { offset: 0 }, &full, &lists, &reader, 1_000).unwrap();
    assert_eq!((listed["entries"][0]["term"].clone(), listed["entries"][0]["hits"].clone(), note.count), (json!("Voltip"), json!(0), Some(1)));
    let (listed, _) = reads::run(&McpRequest::RulesList { offset: 0 }, &full, &lists, &reader, 1_000).unwrap();
    assert_eq!((listed["rules"][0]["name"].clone(), listed["rules"][0]["hits"].clone()), (json!("飞书"), json!(0)));
}

/// A rule's backtest is noted under the rule's name; a write never runs as a read.
#[test]
fn a_rule_backtest_is_noted_by_its_name_and_a_write_is_never_run_as_a_read() {
    let dir = tempfile::tempdir().unwrap();
    // No database yet: nothing to scan.
    let reader = crate::history::HistoryReader::new(dir.path());
    let lists = reads::Lists::default();
    let rule_draft =
        RuleDraft {
            name: " 飞书 ".into(), kind: RuleKind::Literal, pattern: "非书".into(), replacement: "飞书".into(), case_sensitive: true, enabled: true
        };
    let request = McpRequest::Backtest { draft: crate::vocabulary::BacktestDraft::Rule { id: None, rule: rule_draft.clone() }, exclude: None, limit: 10 };
    let (value, note) = reads::run(&request, &settings(McpRead::Rules, McpWrite::ReadOnly), &lists, &reader, 1).unwrap();
    assert_eq!((note.action, note.subject.as_deref(), note.count), (McpAction::Backtest, Some("飞书"), Some(0)));
    assert_eq!((value["scanned"].clone(), value["matched"].clone()), (json!(0), json!(0)));
    assert!(value.get("examples_withheld").is_none(), "no example to withhold");
    let write = McpRequest::RulesAdd { draft: rule_draft };
    assert_eq!(reads::run(&write, &settings(McpRead::Full, McpWrite::EnableRules), &lists, &reader, 1).unwrap_err(), ToolError::Failed("not a read".into()));
}

/// A page of the full history that the size limit cut short goes on after its last record, until
/// every entry was shown once.
#[test]
fn a_history_page_cut_by_the_size_limit_goes_on_after_its_last_record() {
    let dir = tempfile::tempdir().unwrap();
    let big = "长".repeat(40_000);
    let mut store = crate::history::HistoryStore::open(dir.path());
    let mut ids = Vec::new();
    for at_ms in [1_000, 2_000, 3_000] {
        let mut take = history_entry(&big, &big);
        take.at_ms = at_ms;
        ids.push(take.id.to_string());
        store.push(take, 100).unwrap();
    }
    let reader = crate::history::HistoryReader::new(dir.path());
    let full = McpSettings { history_range: HistoryRange::All, ..settings(McpRead::Full, McpWrite::Dictionary) };
    let mut cursor = None;
    let mut seen = Vec::new();
    for _ in 0..ids.len() + 1 {
        let request = McpRequest::HistorySearch { query: None, since_ms: None, until_ms: None, app: None, cursor, limit: 50 };
        let (page, note) = reads::run(&request, &full, &reads::Lists::default(), &reader, 10_000).unwrap();
        let entries = page["entries"].as_array().unwrap();
        assert!(!entries.is_empty() && json_len(&page) <= MAX_RESULT_BYTES + 1024, "{} bytes", json_len(&page));
        assert_eq!(note.count, Some(u32::try_from(entries.len()).unwrap()));
        seen.extend(entries.iter().map(|e| e["id"].as_str().unwrap().to_owned()));
        match page["next_cursor"].as_str() {
            Some(text) => {
                let (at_ms, row) = text.split_once(':').unwrap();
                cursor = Some(HistoryCursor { at_ms: at_ms.parse().unwrap(), row: row.parse().unwrap() });
            }
            None => break,
        }
    }
    ids.reverse();
    assert_eq!(seen, ids, "newest first, each entry once");
}

/// A call that comes when the core has stopped is answered: the service is off.
#[tokio::test]
async fn a_call_after_the_core_stopped_says_the_service_is_off() {
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    drop(rx);
    let service = service::CoreMcp::new(tx);
    service.connected("Claude Code");
    let refused = service.call("Claude Code", "rules_list", Map::new()).await.unwrap_err();
    assert_eq!(refused, ToolError::Unavailable(access::bilingual("Voltip 的 AI 工具接入已关闭。", "Voltip's AI tool access is off.")));
}
