use super::*;

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

fn rule(name: &str, kind: RuleKind, pattern: &str, replacement: &str) -> ReplacementRule {
    ReplacementRule {
        id: Uuid::new_v4(),
        name: name.into(),
        kind,
        pattern: pattern.into(),
        replacement: replacement.into(),
        case_sensitive: true,
        enabled: true,
        created_at_ms: 1,
        updated_at_ms: 1,
    }
}

fn draft(term: &str, heard: &[&str]) -> DictionaryDraft {
    DictionaryDraft { term: term.into(), heard_as: heard.iter().map(|h| (*h).to_owned()).collect(), enabled: true }
}

fn rule_draft(kind: RuleKind, pattern: &str) -> RuleDraft {
    RuleDraft { name: "r".into(), kind, pattern: pattern.into(), replacement: String::new(), case_sensitive: true, enabled: true }
}

/// The sample take's two known mis-hearings (docs/dictation.md §16.8) are corrected before the
/// refiner sees the text.
#[test]
fn dictionary_corrects_the_sample_mis_hearings() {
    let good = entry("good idea", &["谷歌IDR", "谷歌 IDR"]);
    let teams = entry("Teams", &["teams"]);
    let vocab = Vocabulary::compile(&[good.clone(), teams.clone()], &[]);
    let raw = "我想创建一个谷歌IDR吧，集成在teams里面，还有Teams。";
    let step = vocab.correct(raw);
    assert_eq!(step.text, "我想创建一个good idea吧，集成在Teams里面，还有Teams。");
    assert_eq!(
        step.hits,
        vec![VocabularyHit { id: good.id, count: 1 }, VocabularyHit { id: teams.id, count: 1 }],
        "the casing fix counts, the already right Teams does not"
    );
    assert!(step.error.is_none());
    assert_eq!(vocab.glossary(), ["good idea", "Teams"]);
}

#[test]
fn disabled_entries_neither_correct_nor_reach_the_glossary() {
    let mut off = entry("Voltip", &["沃提普"]);
    off.enabled = false;
    let on = entry("Rust", &[]);
    let vocab = Vocabulary::compile(&[off, on], &[]);
    assert_eq!(vocab.correct("沃提普").text, "沃提普");
    assert_eq!(vocab.glossary(), ["Rust"], "a term without mis-hearings still belongs in the glossary");
    assert!(!vocab.is_empty());
    assert!(Vocabulary::empty().is_empty() && Vocabulary::default().glossary().is_empty());
    let step = Vocabulary::empty().correct("x");
    assert_eq!((step.text.as_str(), step.hits.len(), step.error), ("x", 0, None));
    assert!(format!("{vocab:?}").contains("terms: 1") && !format!("{vocab:?}").contains("Rust"), "Debug shows counts, not the user's words");
}

/// The glossary keeps dictionary order, drops duplicates (ASCII case), and stops at the term and
/// character caps; mis-hearings never enter it.
#[test]
fn glossary_is_ordered_deduplicated_and_capped() {
    let entries = vec![entry("Voltip", &["沃提普"]), entry("voltip", &[]), entry("sherpa-onnx", &[])];
    let vocab = Vocabulary::compile(&entries, &[]);
    assert_eq!(vocab.glossary(), ["Voltip", "sherpa-onnx"]);
    assert_eq!(glossary_prompt(vocab.glossary()).as_deref(), Some("Voltip, sherpa-onnx"));
    assert_eq!(glossary_prompt(&[]), None, "no prompt field for an empty glossary");
    // 3-character terms: 200 of them take 3 + 199 × 5 = 998 characters, so the term cap binds.
    let many: Vec<DictionaryEntry> = (0..MAX_DICTIONARY_ENTRIES).map(|i| entry(&format!("{i:03}"), &[])).collect();
    assert_eq!(Vocabulary::compile(&many, &[]).glossary().len(), MAX_GLOSSARY_TERMS);
    // 4-character terms: the character cap binds first (4 + 166 × 6 = 1000).
    let wider: Vec<DictionaryEntry> = (0..MAX_DICTIONARY_ENTRIES).map(|i| entry(&format!("t{i:03}"), &[])).collect();
    assert_eq!(Vocabulary::compile(&wider, &[]).glossary().len(), 167);
    let long: Vec<DictionaryEntry> = (0..40).map(|i| entry(&format!("{i:02}{}", "x".repeat(60)), &[])).collect();
    let glossary = Vocabulary::compile(&long, &[]).glossary().to_vec();
    let prompt = glossary_prompt(&glossary).unwrap();
    assert!(prompt.chars().count() <= MAX_GLOSSARY_CHARS, "{}", prompt.chars().count());
    assert_eq!(glossary.len(), 15, "15 × 62 + 14 × 2 = 958 ≤ 1000 < 1022");
    assert_eq!(glossary[0], long[0].term, "dictionary order is priority");
}

/// §18.10: a built-in scene's pack joins the glossary after the dictionary's terms, skipping the
/// ones already there (ASCII case), inside the same term and character caps; corrections and
/// rules are unchanged, and the dictionary's own vocabulary is not touched.
#[test]
fn pack_terms_follow_the_dictionary_inside_the_same_caps() {
    let entries = vec![entry("Kubernetes", &["酷伯内提斯"]), entry("Voltip", &[])];
    let vocab = Vocabulary::compile(&entries, &[]);
    let with = vocab.with_terms(&["kubernetes", "Docker", "Voltip", "gRPC"]);
    assert_eq!(with.glossary(), ["Kubernetes", "Voltip", "Docker", "gRPC"]);
    assert_eq!(vocab.glossary(), ["Kubernetes", "Voltip"], "the take's copy only");
    assert_eq!(with.correct("酷伯内提斯").text, "Kubernetes");
    // A full dictionary leaves no room: nothing is added past the caps.
    let many: Vec<DictionaryEntry> = (0..MAX_DICTIONARY_ENTRIES).map(|i| entry(&format!("{i:03}"), &[])).collect();
    let full = Vocabulary::compile(&many, &[]);
    assert_eq!(full.with_terms(&["Docker"]).glossary().len(), MAX_GLOSSARY_TERMS);
    let wider: Vec<DictionaryEntry> = (0..166).map(|i| entry(&format!("t{i:03}"), &[])).collect();
    let near = Vocabulary::compile(&wider, &[]);
    // 4 + 165 × 6 = 994 characters: ", Go" fits (998); ", Docker" would not (1006) and ends the list.
    assert_eq!(near.with_terms(&["Go", "Docker", "C"]).glossary().len(), 167);
    assert_eq!(near.with_terms(&["Docker", "Go"]).glossary().len(), 166, "the first term that does not fit ends the pack");
    let prompt = glossary_prompt(near.with_terms(&["Go"]).glossary()).unwrap();
    assert!(prompt.chars().count() <= MAX_GLOSSARY_CHARS);
    assert_eq!(Vocabulary::empty().with_terms(&["API", "SDK"]).glossary(), ["API", "SDK"]);
}

/// Rules run in list order, each on the previous output (chaining); literal and regex; case.
#[test]
fn rules_run_in_order_and_chain() {
    let a = rule("a", RuleKind::Literal, "给他push", "git push");
    let b = rule("b", RuleKind::Regex, r"git (\w+)", "`git $1`");
    let c = rule("c", RuleKind::Literal, "voltip", "Voltip");
    let vocab = Vocabulary::compile(&[], &[a.clone(), b.clone(), c.clone()]);
    let step = vocab.apply_rules("先给他push，再说voltip和Voltip");
    assert_eq!(step.text, "先`git push`，再说Voltip和Voltip");
    assert_eq!(step.hits, vec![VocabularyHit { id: a.id, count: 1 }, VocabularyHit { id: b.id, count: 1 }, VocabularyHit { id: c.id, count: 1 }]);
    // Reversed order: `b` runs before `a` produced its text, so it never fires.
    let vocab = Vocabulary::compile(&[], &[b.clone(), a.clone()]);
    assert_eq!(vocab.apply_rules("给他push").text, "git push");
    // Case-insensitive literal (ASCII) and regex.
    let ci = ReplacementRule { case_sensitive: false, ..rule("ci", RuleKind::Literal, "voltip", "Voltip") };
    let re = ReplacementRule { case_sensitive: false, ..rule("re", RuleKind::Regex, r"\bpr (\d+)", "PR #$1") };
    let vocab = Vocabulary::compile(&[], &[ci, re]);
    assert_eq!(vocab.apply_rules("VOLTIP pr 12 and Pr 7").text, "Voltip PR #12 and PR #7");
    // Disabled rules are skipped.
    let off = ReplacementRule { enabled: false, ..a };
    assert_eq!(Vocabulary::compile(&[], &[off]).apply_rules("给他push").text, "给他push");
}

/// Regex specifics: named groups and `$$`, an empty match never inserts anything, and a rule that
/// deletes text works.
#[test]
fn regex_rules_expand_groups_and_ignore_empty_matches() {
    let named = rule("named", RuleKind::Regex, r"(?P<user>[a-z]+) at (?P<host>[a-z]+) dot com", "${user}@${host}.com costs $$1");
    assert_eq!(Vocabulary::compile(&[], &[named]).apply_rules("mail bob at example dot com").text, "mail bob@example.com costs $1");
    let filler = rule("filler", RuleKind::Regex, "(嗯|啊)*", "");
    let step = Vocabulary::compile(&[], std::slice::from_ref(&filler)).apply_rules("嗯嗯我们开会啊");
    assert_eq!(step.text, "我们开会", "the empty matches between characters are skipped");
    assert_eq!(step.hits, vec![VocabularyHit { id: filler.id, count: 2 }]);
    let step = Vocabulary::compile(&[], &[rule("star", RuleKind::Regex, "x*", "!")]).apply_rules("abc");
    assert_eq!((step.text.as_str(), step.hits.len()), ("abc", 0), "a rule that only matches empty changes nothing");
}

/// A rule that blows the text past the cap, or an input past the cap, falls back to the input with
/// the reason; the dictionary works the same way.
#[test]
fn oversized_input_or_output_falls_back_to_the_input() {
    let explode = rule("explode", RuleKind::Literal, "a", &"b".repeat(MAX_REPLACEMENT_CHARS));
    let text = "a ".repeat(400);
    let step = Vocabulary::compile(&[], &[explode]).apply_rules(&text);
    assert_eq!(step.text, text);
    assert!(step.hits.is_empty());
    assert!(step.error.as_deref().is_some_and(|e| e.contains("规则「explode」让文本超过 64 KiB")), "{:?}", step.error);
    let regex_explode = rule("rx", RuleKind::Regex, "a", &"b".repeat(MAX_REPLACEMENT_CHARS));
    assert!(Vocabulary::compile(&[], &[regex_explode]).apply_rules(&text).error.is_some());
    let huge = "x".repeat(MAX_TEXT_BYTES + 1);
    let vocab = Vocabulary::compile(&[entry("Y", &["x"])], &[rule("r", RuleKind::Literal, "x", "y")]);
    let step = vocab.apply_rules(&huge);
    assert_eq!(step.text.len(), huge.len());
    assert!(step.error.unwrap().contains("跳过替换规则"));
    assert!(vocab.correct(&huge).error.unwrap().contains("跳过词典纠正"));
    let growing = Vocabulary::compile(&[entry(&"长".repeat(MAX_TERM_CHARS), &["x"])], &[]);
    let step = growing.correct(&"x ".repeat(400));
    assert!(step.error.is_some() && step.text == "x ".repeat(400), "the dictionary's output is capped too");
}

#[test]
fn a_panicking_step_falls_back_to_its_input() {
    let step = guarded("原文", || panic!("boom"));
    assert_eq!(step, Step { text: "原文".into(), hits: Vec::new(), error: Some("内部错误（已回退到原文）".into()) });
}

/// Draft validation: trimming, limits, control characters, duplicates collapse, a variant equal to
/// the term is refused, and regex compile errors / size limits are reported at save time.
#[test]
fn drafts_are_validated_and_normalised() {
    let ok = validate_dictionary_draft(&draft("  Voltip ", &["沃提普", "", "  ", "VOLTIP ", "voltip"])).unwrap();
    assert_eq!(ok.term, "Voltip");
    assert_eq!(ok.heard_as, ["沃提普", "VOLTIP"], "blanks dropped, ASCII-case duplicates collapse");
    let cases: Vec<(DictionaryDraft, &str)> = vec![
        (draft(" ", &[]), "正确写法不能为空"),
        (draft(&"x".repeat(MAX_TERM_CHARS + 1), &[]), "最多 64 个字符（当前 65）"),
        (draft("a\nb", &[]), "换行或控制字符"),
        (draft("a", &[&"y".repeat(65)]), "误识别写法最多 64 个字符"),
        (draft("a", &["b\tc"]), "控制字符"),
        (draft("Voltip", &["Voltip"]), "和正确写法相同"),
        (draft("a", &["1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11"]), "最多 10 个误识别写法"),
    ];
    for (d, needle) in cases {
        let err = validate_dictionary_draft(&d).unwrap_err().to_string();
        assert!(err.starts_with("dictionary: ") && err.contains(needle), "{d:?} → {err}");
    }
    assert_eq!(validate_dictionary_draft(&draft(&"中".repeat(MAX_TERM_CHARS), &[])).unwrap().term.chars().count(), 64, "characters, not bytes");

    let r = validate_rule_draft(&RuleDraft { name: " n ".into(), replacement: " x ".into(), ..rule_draft(RuleKind::Literal, " ,") }).unwrap();
    assert_eq!((r.name.as_str(), r.pattern.as_str(), r.replacement.as_str()), ("n", " ,", " x "), "only the name is trimmed");
    let rule_cases: Vec<(RuleDraft, &str)> = vec![
        (RuleDraft { name: String::new(), ..rule_draft(RuleKind::Literal, "x") }, "规则名称不能为空"),
        (RuleDraft { name: "n".repeat(65), ..rule_draft(RuleKind::Literal, "x") }, "规则名称最多 64 个字符"),
        (rule_draft(RuleKind::Literal, ""), "匹配内容不能为空"),
        (rule_draft(RuleKind::Literal, &"p".repeat(MAX_PATTERN_CHARS + 1)), "匹配内容最多 256 个字符"),
        (RuleDraft { replacement: "r".repeat(MAX_REPLACEMENT_CHARS + 1), ..rule_draft(RuleKind::Literal, "x") }, "替换内容最多 256 个字符"),
        (rule_draft(RuleKind::Regex, "(unclosed"), "正则无法编译"),
        (rule_draft(RuleKind::Regex, r"(\w{100}){100}"), "正则过大"),
        (rule_draft(RuleKind::Regex, r"(a)\1"), "backreferences are not supported"),
        (rule_draft(RuleKind::Regex, r"(?<=a)b"), "look-around"),
    ];
    for (d, needle) in rule_cases {
        let err = validate_rule_draft(&d).unwrap_err().to_string();
        assert!(err.starts_with("rules: ") && err.contains(needle), "{d:?} → {err}");
    }
    assert!(validate_rule_draft(&rule_draft(RuleKind::Literal, "(unclosed")).is_ok(), "a literal is not a regex");
    assert!(validate_rule_draft(&rule_draft(RuleKind::Regex, r"\bpr (\d+)")).is_ok());
}

/// Whole-list checks: caps, unique terms / names, a mis-hearing belongs to one entry and is nobody's term.
#[test]
fn whole_lists_are_checked() {
    let a = entry("Voltip", &["沃提普"]);
    assert!(check_dictionary(&[a.clone(), entry("Rust", &["锈"])]).is_ok());
    assert!(check_dictionary(&[a.clone(), entry("voltip", &[])]).unwrap_err().to_string().contains("已有「Voltip」"));
    assert!(check_dictionary(&[a.clone(), entry("X", &["沃提普"])]).unwrap_err().to_string().contains("已是「Voltip」的误识别写法"));
    assert!(check_dictionary(&[a.clone(), entry("X", &["voltip"])]).unwrap_err().to_string().contains("是词条「Voltip」的正确写法"));
    assert!(check_dictionary(&[entry("OpenAI", &["openai"])]).is_ok(), "a casing fix of the entry's own term is fine");
    assert!(check_dictionary(&[a.clone(), entry("沃提普", &[])]).unwrap_err().to_string().contains("不能再作为正确写法"));
    let too_many: Vec<DictionaryEntry> = (0..=MAX_DICTIONARY_ENTRIES).map(|i| entry(&format!("t{i}"), &[])).collect();
    assert!(check_dictionary(&too_many).unwrap_err().to_string().contains("500"));
    let r = rule("same", RuleKind::Literal, "x", "y");
    assert!(check_rules(&[r.clone(), rule("other", RuleKind::Literal, "x", "y")]).is_ok());
    assert!(check_rules(&[r.clone(), rule("same", RuleKind::Literal, "z", "y")]).unwrap_err().to_string().contains("已有名为「same」"));
    let too_many: Vec<ReplacementRule> = (0..=MAX_RULES).map(|i| rule(&format!("r{i}"), RuleKind::Literal, "x", "")).collect();
    assert!(check_rules(&too_many).unwrap_err().to_string().contains("200"));
}

/// The preview runs the same code as the pipeline: dictionary, then rules; a draft stands in for
/// the rule it names or is appended (hits under the nil id); an invalid draft or an oversized text
/// is an error; a fallback shows in `error`.
#[test]
fn preview_uses_the_pipeline_semantics_and_drafts() {
    let dictionary = vec![entry("good idea", &["谷歌IDR"])];
    let saved = rule("app", RuleKind::Literal, "app", "App");
    let rules = vec![saved.clone()];
    let p = preview(&dictionary, &rules, "一个谷歌IDR的app", None).unwrap();
    assert_eq!(p.corrected, "一个good idea的app");
    assert_eq!(p.output, "一个good idea的App");
    assert_eq!(p.corrections, vec![VocabularyHit { id: dictionary[0].id, count: 1 }]);
    assert_eq!(p.rules, vec![VocabularyHit { id: saved.id, count: 1 }]);
    assert!(p.error.is_none());
    // Editing the saved rule: the draft replaces it (its hits keep the saved id).
    let edit = PreviewDraft { id: Some(saved.id), rule: RuleDraft { replacement: "APP".into(), ..RuleDraft::from(&saved) } };
    let p = preview(&dictionary, &rules, "一个app", Some(&edit)).unwrap();
    assert_eq!((p.output.as_str(), p.rules.clone()), ("一个APP", vec![VocabularyHit { id: saved.id, count: 1 }]));
    // A new rule: appended after the saved ones, reported under the nil id.
    let new = PreviewDraft { id: None, rule: RuleDraft { name: "new".into(), ..rule_draft(RuleKind::Regex, "App") } };
    let p = preview(&dictionary, &rules, "一个app", Some(&new)).unwrap();
    assert_eq!(p.output, "一个");
    assert_eq!(p.rules, vec![VocabularyHit { id: saved.id, count: 1 }, VocabularyHit { id: Uuid::nil(), count: 1 }]);
    // An unknown id is treated as a new rule (appended).
    let stray = PreviewDraft { id: Some(Uuid::new_v4()), rule: RuleDraft { name: "stray".into(), ..rule_draft(RuleKind::Literal, "一") } };
    assert_eq!(preview(&[], &rules, "一个app", Some(&stray)).unwrap().output, "个App");
    let bad = PreviewDraft { id: None, rule: rule_draft(RuleKind::Regex, "(") };
    assert!(preview(&dictionary, &rules, "x", Some(&bad)).unwrap_err().to_string().contains("正则无法编译"));
    assert!(preview(&dictionary, &rules, &"x".repeat(MAX_TEXT_BYTES + 1), None).unwrap_err().to_string().contains("64 KiB"));
    let explode = rule("explode", RuleKind::Literal, "a", &"b".repeat(MAX_REPLACEMENT_CHARS));
    let p = preview(&[], &[explode], &"a ".repeat(400), None).unwrap();
    assert!(p.error.is_some() && p.output == p.corrected);
    let json = serde_json::to_string(&preview(&[], &[], "x", None).unwrap()).unwrap();
    assert_eq!(json, r#"{"corrected":"x","output":"x","corrections":[],"rules":[]}"#, "no error key when nothing fell back");
}

/// Wire shapes (docs/dictation.md §16.1 / §16.4) and the defaults of the drafts.
#[test]
fn wire_shapes_and_draft_defaults() {
    let id = Uuid::nil();
    let e = DictionaryEntry { id, source: EntrySource::History { history_id: id }, ..entry("Voltip", &["沃提普"]) };
    let json = serde_json::to_string(&e).unwrap();
    assert_eq!(
        json,
        r#"{"id":"00000000-0000-0000-0000-000000000000","term":"Voltip","heard_as":["沃提普"],"enabled":true,"source":{"kind":"history","history_id":"00000000-0000-0000-0000-000000000000"},"created_at_ms":1,"updated_at_ms":1}"#
    );
    assert_eq!(serde_json::from_str::<DictionaryEntry>(&json).unwrap(), e);
    assert_eq!(serde_json::to_string(&EntrySource::Manual).unwrap(), r#"{"kind":"manual"}"#);
    let r = ReplacementRule { id, ..rule("PR", RuleKind::Regex, r"\bpr (\d+)", "PR #$1") };
    let json = serde_json::to_string(&r).unwrap();
    assert!(json.contains(r#""kind":"regex","pattern":"\\bpr (\\d+)","replacement":"PR #$1","case_sensitive":true"#), "{json}");
    assert_eq!(serde_json::from_str::<ReplacementRule>(&json).unwrap(), r);
    let d: DictionaryDraft = serde_json::from_str(r#"{"term":"x"}"#).unwrap();
    assert_eq!((d.heard_as.len(), d.enabled), (0, true));
    assert!(serde_json::from_str::<DictionaryDraft>(r#"{"term":"x","weight":2}"#).is_err(), "unknown fields are refused");
    let d: RuleDraft = serde_json::from_str(r#"{"name":"n","pattern":"p"}"#).unwrap();
    assert_eq!((d.kind, d.replacement.as_str(), d.case_sensitive, d.enabled), (RuleKind::Literal, "", true, true));
    assert!(serde_json::from_str::<RuleDraft>(r#"{"name":"n","pattern":"p","scope":"Code.exe"}"#).is_err());
    assert_eq!(serde_json::to_string(&ImportMode::Merge).unwrap(), r#""merge""#);
    assert_eq!(serde_json::from_str::<PreviewDraft>(r#"{"rule":{"name":"n","pattern":"p"}}"#).unwrap().id, None);
    let hits = VocabularyHits { corrections: vec![VocabularyHit { id, count: 2 }], rules: vec![] };
    assert_eq!(serde_json::to_string(&hits).unwrap(), r#"{"corrections":[{"id":"00000000-0000-0000-0000-000000000000","count":2}],"rules":[]}"#);
    assert_eq!(serde_json::from_str::<VocabularyHits>("{}").unwrap(), VocabularyHits::default());
    assert!(VocabularyHits::default().into_option().is_none());
    assert_eq!(hits.clone().into_option(), Some(hits.clone()));
    assert_eq!(serde_json::to_string(&RuleKind::default()).unwrap(), r#""literal""#);
}

#[test]
fn hits_merge_by_id_in_first_seen_order() {
    let (a, b, c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let mut total = VocabularyHits { corrections: vec![VocabularyHit { id: a, count: 1 }], rules: vec![VocabularyHit { id: c, count: u32::MAX }] };
    total.merge(VocabularyHits {
        corrections: vec![VocabularyHit { id: b, count: 2 }, VocabularyHit { id: a, count: 3 }],
        rules: vec![VocabularyHit { id: c, count: 1 }],
    });
    assert_eq!(total.corrections, vec![VocabularyHit { id: a, count: 4 }, VocabularyHit { id: b, count: 2 }]);
    assert_eq!(total.rules, vec![VocabularyHit { id: c, count: u32::MAX }], "saturating");
}

/// A rule that fails to compile at `compile` time (a store validated on save never has one) is
/// skipped, not fatal; the others still run.
#[test]
fn a_rule_that_does_not_compile_is_skipped() {
    let broken = rule("broken", RuleKind::Regex, "(", "x");
    let fine = rule("fine", RuleKind::Literal, "a", "b");
    let vocab = Vocabulary::compile(&[], &[broken, fine.clone()]);
    let step = vocab.apply_rules("a");
    assert_eq!((step.text.as_str(), step.hits), ("b", vec![VocabularyHit { id: fine.id, count: 1 }]));
}
