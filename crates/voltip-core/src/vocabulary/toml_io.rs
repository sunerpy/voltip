//! The TOML exchange format of the replacement rules (docs/dictation.md §16.5): `version = 1`
//! plus one `[[rule]]` table per rule, in execution order, without ids or timestamps.

use serde::{Deserialize, Serialize};

use super::{MAX_RULES, MAX_TOML_BYTES, ReplacementRule, RuleDraft, VocabularyError, validate_rule_draft};

/// `version` of the format.
pub const RULES_TOML_VERSION: u32 = 1;

/// First line of an export (a comment; imports ignore it).
const HEADER: &str = "# Voltip 替换规则 · docs/dictation.md §16\n";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RulesToml {
    version: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    rule: Vec<RuleDraft>,
}

/// The rules as TOML text (what `rules_export` returns).
pub fn export_rules_toml(rules: &[ReplacementRule]) -> Result<String, VocabularyError> {
    let file = RulesToml { version: RULES_TOML_VERSION, rule: rules.iter().map(RuleDraft::from).collect() };
    let body = toml::to_string(&file).map_err(|e| VocabularyError::Rules(format!("无法导出 TOML：{e}")))?;
    Ok(format!("{HEADER}{body}"))
}

/// Parse and validate a whole import (docs/dictation.md §16.5): the text, the version, the count,
/// every rule on its own (a regex is compiled) and the uniqueness of names inside the file. The
/// first problem is reported with its position; nothing is partially accepted.
pub fn parse_rules_toml(text: &str) -> Result<Vec<RuleDraft>, VocabularyError> {
    let err = |m: String| VocabularyError::Rules(m);
    if text.len() > MAX_TOML_BYTES {
        return Err(err(format!("TOML 文本最多 {} KiB", MAX_TOML_BYTES / 1024)));
    }
    let file: RulesToml = toml::from_str(text).map_err(|e| err(format!("TOML 无法解析：{}", e.to_string().trim_end())))?;
    if file.version != RULES_TOML_VERSION {
        return Err(err(format!("不支持的 version = {}（应为 {RULES_TOML_VERSION}）", file.version)));
    }
    if file.rule.len() > MAX_RULES {
        return Err(err(format!("文件里有 {} 条规则，最多 {MAX_RULES} 条", file.rule.len())));
    }
    let mut drafts: Vec<RuleDraft> = Vec::with_capacity(file.rule.len());
    for (i, rule) in file.rule.iter().enumerate() {
        let n = i + 1;
        let draft = validate_rule_draft(rule).map_err(|e| {
            let detail = e.to_string();
            err(format!("第 {n} 条（{}）：{}", rule.name.trim(), detail.strip_prefix("rules: ").unwrap_or(&detail)))
        })?;
        if drafts.iter().any(|d| d.name == draft.name) {
            return Err(err(format!("第 {n} 条：名称「{}」在文件里重复", draft.name)));
        }
        drafts.push(draft);
    }
    Ok(drafts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vocabulary::RuleKind;
    use uuid::Uuid;

    fn stored(name: &str, kind: RuleKind, pattern: &str, replacement: &str, case_sensitive: bool, enabled: bool) -> ReplacementRule {
        ReplacementRule {
            id: Uuid::new_v4(),
            name: name.into(),
            kind,
            pattern: pattern.into(),
            replacement: replacement.into(),
            case_sensitive,
            enabled,
            created_at_ms: 1,
            updated_at_ms: 2,
        }
    }

    /// Export then import gives the same rules (as drafts) in the same order; ids and timestamps
    /// stay out of the text; quotes, backslashes and `$1` survive.
    #[test]
    fn export_then_import_round_trips() {
        let rules = vec![
            stored("git push", RuleKind::Literal, "给他push", "git push", true, true),
            stored("PR 编号", RuleKind::Regex, r"\bpr (\d+)", "PR #$1", false, true),
            stored("quote \"x\"", RuleKind::Literal, "it's", "it is", true, false),
            stored("删除填充词", RuleKind::Regex, "(嗯|啊)+[，,]?", "", true, true),
        ];
        let text = export_rules_toml(&rules).unwrap();
        assert!(text.starts_with("# Voltip 替换规则"), "{text}");
        assert!(text.contains("version = 1") && text.contains("[[rule]]"), "{text}");
        assert!(!text.contains("created_at_ms") && !text.contains(&rules[0].id.to_string()), "{text}");
        let back = parse_rules_toml(&text).unwrap();
        assert_eq!(back, rules.iter().map(RuleDraft::from).collect::<Vec<_>>());
        assert_eq!(export_rules_toml(&[]).unwrap(), format!("{HEADER}version = 1\n"));
        assert!(parse_rules_toml(&export_rules_toml(&[]).unwrap()).unwrap().is_empty());
    }

    #[test]
    fn missing_keys_take_the_documented_defaults() {
        let drafts = parse_rules_toml("version = 1\n[[rule]]\nname = \" a \"\npattern = \"x\"\n").unwrap();
        assert_eq!(
            drafts,
            vec![RuleDraft { name: "a".into(), kind: RuleKind::Literal, pattern: "x".into(), replacement: String::new(), case_sensitive: true, enabled: true }]
        );
    }

    /// Every refusal names where the problem is; nothing is half-accepted.
    #[test]
    fn invalid_imports_are_refused_with_their_position() {
        let cases: [(&str, &str); 7] = [
            ("version = 1\n[[rule]]\nname = \"a\"\npattern = \"x\n", "TOML 无法解析"),
            ("version = 1\n[[rule]]\nname = \"a\"\npatern = \"x\"\n", "unknown field"),
            ("version = 2\n", "不支持的 version = 2"),
            ("[[rule]]\nname = \"a\"\npattern = \"x\"\n", "missing field"),
            (
                "version = 1\n[[rule]]\nname = \"ok\"\npattern = \"x\"\n[[rule]]\nname = \"bad\"\nkind = \"regex\"\npattern = \"(\"\n",
                "第 2 条（bad）：规则「bad」：正则无法编译",
            ),
            ("version = 1\n[[rule]]\nname = \"a\"\npattern = \"x\"\n[[rule]]\nname = \"a\"\npattern = \"y\"\n", "第 2 条：名称「a」在文件里重复"),
            ("version = 1\n[[rule]]\nname = \"a\"\nkind = \"glob\"\npattern = \"x\"\n", "unknown variant"),
        ];
        for (text, needle) in cases {
            let err = parse_rules_toml(text).unwrap_err().to_string();
            assert!(err.starts_with("rules: ") && err.contains(needle), "{text:?} → {err}");
        }
        let syntax = parse_rules_toml("version = 1\n[[rule]]\nname = \"a\"\npattern = \"x\n").unwrap_err().to_string();
        assert!(syntax.contains("line 4"), "TOML errors carry the line: {syntax}");
        let many: String =
            std::iter::once("version = 1\n".to_owned()).chain((0..=MAX_RULES).map(|i| format!("[[rule]]\nname = \"r{i}\"\npattern = \"p\"\n"))).collect();
        assert!(parse_rules_toml(&many).unwrap_err().to_string().contains("最多 200 条"));
        assert!(parse_rules_toml(&"#".repeat(MAX_TOML_BYTES + 1)).unwrap_err().to_string().contains("256 KiB"));
    }
}
