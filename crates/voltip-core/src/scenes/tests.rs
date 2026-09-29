use super::*;
use crate::dictation::Via;
use crate::history::Outcome;

fn draft(name: &str, apps: &[&str], keywords: &[&str]) -> SceneDraft {
    SceneDraft {
        name: name.into(),
        enabled: true,
        matching: SceneMatch { apps: apps.iter().map(|a| (*a).to_owned()).collect(), title_contains: keywords.iter().map(|k| (*k).to_owned()).collect() },
        overrides: SceneOverrides::default(),
    }
}

fn scene(name: &str, apps: &[&str], keywords: &[&str]) -> Scene {
    let d = validate_scene_draft(&draft(name, apps, keywords)).unwrap();
    Scene { id: Uuid::new_v4(), name: d.name, enabled: d.enabled, matching: d.matching, overrides: d.overrides, created_at_ms: 1, updated_at_ms: 1 }
}

fn app(id: &str, title: Option<&str>) -> ForegroundApp {
    ForegroundApp { app_id: id.into(), name: id.into(), title: title.map(str::to_owned), window: None }
}

fn err(d: &SceneDraft) -> String {
    validate_scene_draft(d).unwrap_err().to_string()
}

/// A draft is normalised on its own: name trimmed, apps normalised / de-duplicated / blanks
/// dropped, keywords trimmed and de-duplicated ignoring case, the language lower-cased, the prompt's
/// line endings normalised; blank language / prompt mean "follow the global setting".
#[test]
fn drafts_are_normalised() {
    let mut d = draft(" 聊天 ", &[" Slack.EXE", "slack", "  ", "WeChat.exe", "com.Tinyspeck.SlackMacGap"], &[" GitHub ", "github", "", "拉取请求"]);
    d.overrides = SceneOverrides {
        refine_enabled: Some(true),
        refine_style: Some(RefineStyle::Punctuation),
        output_mode: Some(OutputMode::StreamingFinal),
        language: Some(" ZH-Hans ".into()),
        chinese_script: Some(ChineseScript::Traditional),
        prompt: Some("  第一行\r\n第二行\r第三行\t。 \n".into()),
    };
    let v = validate_scene_draft(&d).unwrap();
    assert_eq!(v.name, "聊天");
    assert_eq!(v.matching.apps, ["slack", "wechat", "com.tinyspeck.slackmacgap"]);
    assert_eq!(v.matching.title_contains, ["GitHub", "拉取请求"]);
    assert_eq!(v.overrides.language.as_deref(), Some("zh-hans"));
    assert_eq!(v.overrides.prompt.as_deref(), Some("第一行\n第二行\n第三行\t。"));
    assert_eq!(
        (v.overrides.refine_enabled, v.overrides.refine_style, v.overrides.output_mode),
        (Some(true), Some(RefineStyle::Punctuation), Some(OutputMode::StreamingFinal))
    );
    assert_eq!(v.overrides.chinese_script, Some(ChineseScript::Traditional));
    assert_eq!(validate_scene_draft(&v).unwrap(), v, "the normalised form is a fixed point");
    let mut blank = draft("a", &["x"], &[]);
    blank.overrides.language = Some("  ".into());
    blank.overrides.prompt = Some(" \r\n ".into());
    let v = validate_scene_draft(&blank).unwrap();
    assert!(v.overrides.is_empty(), "blank language and prompt follow the global setting: {:?}", v.overrides);
    let mut auto = draft("a", &["x"], &[]);
    auto.overrides.language = Some("AUTO".into());
    assert_eq!(validate_scene_draft(&auto).unwrap().overrides.language.as_deref(), Some(LANGUAGE_AUTO));
}

/// Every limit of §18.1 refuses with a Chinese reason behind the `scenes:` prefix.
#[test]
fn drafts_past_the_limits_are_refused() {
    assert!(err(&draft("  ", &["x"], &[])).starts_with("scenes: 场景名称不能为空"));
    assert!(err(&draft(&"名".repeat(33), &["x"], &[])).contains("最多 32 个字符（当前 33）"));
    assert!(validate_scene_draft(&draft(&"名".repeat(32), &["x"], &[])).is_ok());
    assert!(err(&draft("a\nb", &["x"], &[])).contains("不能包含换行"));
    assert!(err(&draft("聊天", &[], &[])).contains("场景「聊天」至少要有一个应用"));
    assert!(err(&draft("聊天", &["  ", ""], &[])).contains("至少要有一个应用"), "only blanks = no app");
    assert!(err(&draft("a", &[".EXE"], &[])).contains("应用 id「.EXE」无效"));
    assert!(err(&draft("a", &[&"x".repeat(129)], &[])).contains("应用 id最多 128 个字符"));
    assert!(err(&draft("a", &["sl\u{7}ack"], &[])).contains("应用 id不能包含换行或控制字符"));
    let many: Vec<String> = (0..21).map(|i| format!("app{i}")).collect();
    let many: Vec<&str> = many.iter().map(String::as_str).collect();
    assert!(err(&draft("a", &many, &[])).contains("最多 20 个应用（当前 21）"));
    assert!(validate_scene_draft(&draft("a", &many[..20], &[])).is_ok());
    let keywords: Vec<String> = (0..11).map(|i| format!("k{i}")).collect();
    let keywords: Vec<&str> = keywords.iter().map(String::as_str).collect();
    assert!(err(&draft("a", &["x"], &keywords)).contains("最多 10 个窗口标题关键词（当前 11）"));
    assert!(err(&draft("a", &["x"], &[&"k".repeat(65)])).contains("窗口标题关键词最多 64 个字符"));
    for bad in ["中文", "zh_CN", "-x", "zh cn", &"a".repeat(17)] {
        let mut d = draft("a", &["x"], &[]);
        d.overrides.language = Some(bad.to_owned());
        assert!(err(&d).contains("语言代码"), "{bad:?}");
    }
    let mut d = draft("a", &["x"], &[]);
    d.overrides.prompt = Some("长".repeat(501));
    assert!(err(&d).contains("补充要求最多 500 个字符（当前 501）"));
    d.overrides.prompt = Some("长".repeat(500));
    assert!(validate_scene_draft(&d).is_ok());
    d.overrides.prompt = Some("a\u{1b}[31m".into());
    assert!(err(&d).contains("补充要求不能包含控制字符"));
}

/// The list: at most 50 scenes, names unique ignoring ASCII case; the same app in two scenes is fine.
#[test]
fn the_list_caps_the_count_and_keeps_names_unique() {
    let list: Vec<Scene> = (0..MAX_SCENES).map(|i| scene(&format!("s{i}"), &["x"], &[])).collect();
    assert!(check_scenes(&list).is_ok(), "the same app in every scene is allowed");
    let mut over = list.clone();
    over.push(scene("one more", &["y"], &[]));
    assert!(check_scenes(&over).unwrap_err().to_string().contains("场景最多 50 个"));
    let dup = vec![scene("Chat", &["a"], &[]), scene("chat", &["b"], &[])];
    assert_eq!(check_scenes(&dup).unwrap_err().to_string(), "scenes: 已有名为「Chat」的场景");
}

/// §18.3: the first enabled scene in list order whose apps contain the (normalised) id and whose
/// title keywords — if any — occur in the window title, ignoring case.
#[test]
fn matching_takes_the_first_enabled_scene_by_app_and_title() {
    let github = scene("GitHub", &["chrome", "firefox"], &["GitHub", "拉取请求"]);
    let browser = scene("浏览器", &["chrome.exe", "Firefox"], &[]);
    let mut off = scene("停用", &["slack"], &[]);
    off.enabled = false;
    let chat = scene("聊天", &["slack", "wechat"], &[]);
    let scenes = vec![github.clone(), browser.clone(), off, chat.clone()];
    let hit = |a: &ForegroundApp| match_scene(&scenes, a).map(|s| s.name.clone());
    let table: [(ForegroundApp, Option<&str>); 12] = [
        (app("chrome", Some("voltip/pull/12 · GitHub")), Some("GitHub")),
        (app("CHROME.EXE", Some("Pull requests · GITHUB")), Some("GitHub")),
        (app("firefox", Some("审阅拉取请求")), Some("GitHub")),
        (app("chrome", Some("Inbox")), Some("浏览器")),
        (app("chrome", None), Some("浏览器")),
        (app("Firefox.exe", Some("")), Some("浏览器")),
        (app("slack", Some("#dev")), Some("聊天")),
        (app("  WeChat.EXE ", None), Some("聊天")),
        (app("code", Some("GitHub")), None),
        (app("", Some("GitHub")), None),
        (app(".exe", None), None),
        (app("chromium", Some("GitHub")), None),
    ];
    for (a, want) in table {
        assert_eq!(hit(&a).as_deref(), want, "{a:?} {:?}", a.title);
    }
    // Order decides: the generic browser scene first shadows the GitHub one.
    let reordered = vec![browser, github, chat];
    assert_eq!(match_scene(&reordered, &app("chrome", Some("GitHub"))).map(|s| s.name.as_str()), Some("浏览器"));
    assert!(match_scene(&[], &app("chrome", None)).is_none());
}

fn entry(app: Option<(&str, &str)>) -> HistoryEntry {
    HistoryEntry {
        id: Uuid::new_v4(),
        at_ms: 1,
        raw_text: "x".into(),
        text: "x".into(),
        refined: false,
        asr_model: "m".into(),
        refine_model: None,
        duration_ms: 1,
        asr_ms: 1,
        refine_ms: None,
        outcome: Outcome::Inserted { via: Via::Paste },
        starred: false,
        mode: OutputMode::WholeTake,
        segments: None,
        live_error: None,
        vocabulary: None,
        kind: crate::TakeKind::Dictation,
        edit: None,
        app: app.map(|(id, name)| AppRef { id: id.into(), name: name.into() }),
        scene: None,
        origin: None,
    }
}

/// `recent_apps`: newest first (the history is newest first), one per id with the newest name,
/// rows without an app skipped, capped.
#[test]
fn recent_apps_are_the_newest_distinct_ones() {
    let history = vec![
        entry(Some(("slack", "Slack 4"))),
        entry(None),
        entry(Some(("code", "Code"))),
        entry(Some(("slack", "Slack 3"))),
        entry(Some(("winword", "WINWORD"))),
    ];
    let ids: Vec<(String, String)> = recent_apps(&history, MAX_RECENT_APPS).into_iter().map(|a| (a.id, a.name)).collect();
    assert_eq!(ids, [("slack".to_owned(), "Slack 4".to_owned()), ("code".into(), "Code".into()), ("winword".into(), "WINWORD".into())]);
    assert_eq!(recent_apps(&history, 2).len(), 2);
    assert!(recent_apps(&[], 20).is_empty());
    let many: Vec<HistoryEntry> = (0..30).map(|i| entry(Some((&format!("app{i}"), "n")))).collect();
    assert_eq!(recent_apps(&many, MAX_RECENT_APPS).len(), MAX_RECENT_APPS);
}

/// Wire shapes: `match` (a Rust keyword) on the wire, unset overrides absent, drafts default to on
/// with nothing overridden, unknown fields refused, the sharing switches default app-name-on /
/// title-off, a take context without a scene omits it.
#[test]
fn wire_shapes() {
    let mut s = scene("聊天", &["slack"], &[]);
    s.id = Uuid::nil();
    s.overrides.refine_style = Some(RefineStyle::Punctuation);
    let json = serde_json::to_string(&s).unwrap();
    assert_eq!(
        json,
        r#"{"id":"00000000-0000-0000-0000-000000000000","name":"聊天","enabled":true,"match":{"apps":["slack"],"title_contains":[]},"overrides":{"refine_style":"punctuation"},"created_at_ms":1,"updated_at_ms":1}"#
    );
    assert_eq!(serde_json::from_str::<Scene>(&json).unwrap(), s);
    let d: SceneDraft = serde_json::from_str(r#"{"name":"a","match":{"apps":["x"]}}"#).unwrap();
    assert!(d.enabled && d.overrides.is_empty() && d.matching.title_contains.is_empty());
    let d: SceneDraft = serde_json::from_str(r#"{"name":"a","match":{"apps":["x"]},"overrides":{"refine_enabled":null,"output_mode":"live_inject"}}"#).unwrap();
    assert_eq!((d.overrides.refine_enabled, d.overrides.output_mode), (None, Some(OutputMode::LiveInject)), "null = follow the global setting");
    for bad in [
        r#"{"name":"a","match":{"apps":["x"]},"weight":1}"#,
        r#"{"name":"a","match":{"apps":["x"],"urls":["github.com"]}}"#,
        r#"{"name":"a","match":{"apps":["x"]},"overrides":{"model":"m"}}"#,
        r#"{"name":"a","match":{"apps":["x"]},"overrides":{"refine_style":"casual"}}"#,
        r#"{"name":"a"}"#,
    ] {
        assert!(serde_json::from_str::<SceneDraft>(bad).is_err(), "{bad}");
    }
    assert_eq!(ContextSharing::default(), ContextSharing { app_name: true, window_title: false });
    assert_eq!(serde_json::to_string(&ContextSharing::default()).unwrap(), r#"{"app_name":true,"window_title":false}"#);
    assert_eq!(serde_json::from_str::<ContextSharing>("{}").unwrap(), ContextSharing::default());
    assert_eq!(serde_json::from_str::<ContextSharing>(r#"{"window_title":true}"#).unwrap(), ContextSharing { app_name: true, window_title: true });
    let ctx = TakeContext { app: AppRef { id: "slack".into(), name: "Slack".into() }, scene: None };
    assert_eq!(serde_json::to_string(&ctx).unwrap(), r#"{"app":{"id":"slack","name":"Slack"}}"#);
    let with = TakeContext { scene: Some(s.to_ref()), ..ctx };
    assert!(serde_json::to_string(&with).unwrap().ends_with(r#""scene":{"id":"00000000-0000-0000-0000-000000000000","name":"聊天"}}"#));
    assert_eq!(SceneDraft::from(&s).overrides.refine_style, Some(RefineStyle::Punctuation));
}

fn corrupt_files(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> =
        std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()?.file_name().into_string().ok()).filter(|n| n.starts_with("scenes.json.corrupt-")).collect();
    names.sort();
    names
}

/// Add / update / remove / reorder round-trip through `scenes.json`; refusals touch neither the
/// file nor the list; the cap holds.
#[test]
fn the_store_round_trips_every_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, notice) = SceneStore::open(dir.path());
    assert!(notice.is_none() && store.scenes().is_empty());
    let a = store.add(&draft(" 聊天 ", &["Slack.exe"], &[]), 10).unwrap();
    let b = store.add(&draft("代码", &["code"], &["README"]), 20).unwrap();
    assert_eq!(store.scenes()[0].name, "聊天");
    assert_eq!(store.scenes()[0].matching.apps, ["slack"], "stored normalised");
    assert_eq!(SceneStore::open(dir.path()).0.scenes(), store.scenes());
    let mut changed = draft("聊天", &["slack", "wechat"], &[]);
    changed.enabled = false;
    changed.overrides.prompt = Some("口语化".into());
    store.update(a, &changed, 30).unwrap();
    let s = &store.scenes()[0];
    assert_eq!((s.enabled, s.matching.apps.len(), s.created_at_ms, s.updated_at_ms, s.overrides.prompt.as_deref()), (false, 2, 10, 30, Some("口语化")));
    store.reorder(&[b, a]).unwrap();
    assert_eq!(SceneStore::open(dir.path()).0.scenes()[0].id, b);
    for bad in [vec![a], vec![a, a], vec![a, Uuid::new_v4()], vec![b, a, a]] {
        assert!(store.reorder(&bad).unwrap_err().to_string().contains("全部场景"), "{bad:?}");
    }
    let before = std::fs::read_to_string(dir.path().join(SCENES_FILE_NAME)).unwrap();
    assert!(store.add(&draft("代码", &["x"], &[]), 40).unwrap_err().to_string().contains("已有名为「代码」"));
    assert!(store.add(&draft("", &["x"], &[]), 40).is_err());
    assert!(store.update(Uuid::new_v4(), &draft("z", &["x"], &[]), 40).unwrap_err().to_string().contains("没有 id"));
    assert!(store.remove(Uuid::new_v4()).unwrap_err().to_string().starts_with("scenes: 没有 id"));
    assert_eq!(std::fs::read_to_string(dir.path().join(SCENES_FILE_NAME)).unwrap(), before, "refusals leave the file alone");
    store.remove(a).unwrap();
    assert_eq!(SceneStore::open(dir.path()).0.scenes().len(), 1);
    assert!(!dir.path().join("scenes.json.tmp").exists());
    assert!(format!("{store:?}").contains("scenes.json"));
    for i in 1..MAX_SCENES {
        store.add(&draft(&format!("s{i}"), &["x"], &[]), 1).unwrap();
    }
    assert!(store.add(&draft("one more", &["x"], &[]), 1).unwrap_err().to_string().contains("场景最多 50 个"));
    assert_eq!(SceneStore::open(dir.path()).0.scenes().len(), MAX_SCENES);
}

/// A file that cannot be used never stops the store: moved aside to `.corrupt-<secs>` (kept, never
/// overwritten), the store starts empty with a notice and writes the path again afterwards; an
/// unreadable path makes the store read-only.
#[test]
fn unusable_scene_files_are_quarantined_not_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(SCENES_FILE_NAME);
    std::fs::write(&path, b"{not json").unwrap();
    let (mut store, notice) = SceneStore::open(dir.path());
    let notice = notice.unwrap();
    assert!(notice.contains("scenes.json 无法使用") && notice.contains(".corrupt-"), "{notice}");
    assert!(store.scenes().is_empty() && !path.exists());
    assert_eq!(std::fs::read(dir.path().join(&corrupt_files(dir.path())[0])).unwrap(), b"{not json");
    std::fs::write(&path, br#"{"schema":9,"scenes":[]}"#).unwrap();
    assert!(SceneStore::open(dir.path()).1.unwrap().contains("schema 不是 1"));
    let raw = |name: &str, apps: &str| {
        format!(
            r#"{{"id":"{}","name":"{name}","enabled":true,"match":{{"apps":{apps}}},"overrides":{{}},"created_at_ms":1,"updated_at_ms":1}}"#,
            Uuid::new_v4()
        )
    };
    std::fs::write(&path, format!(r#"{{"schema":1,"scenes":[{},{}]}}"#, raw("A", r#"["x"]"#), raw("a", r#"["y"]"#))).unwrap();
    assert!(SceneStore::open(dir.path()).1.unwrap().contains("已有名为「A」"));
    std::fs::write(&path, format!(r#"{{"schema":1,"scenes":[{}]}}"#, raw("A", r#"["Slack.exe"]"#))).unwrap();
    assert!(SceneStore::open(dir.path()).1.unwrap().contains("不是规范形式"), "an id that is not normalised");
    std::fs::write(&path, format!(r#"{{"schema":1,"scenes":[{}]}}"#, raw("A", "[]"))).unwrap();
    assert!(SceneStore::open(dir.path()).1.unwrap().contains("至少要有一个应用"));
    assert_eq!(corrupt_files(dir.path()).len(), 5, "every bad file is kept");
    store.add(&draft("fresh", &["x"], &[]), 1).unwrap();
    assert_eq!(SceneStore::open(dir.path()).0.scenes()[0].name, "fresh");
    // A directory in place of the file: neither moved nor overwritten, the store refuses to write.
    let other = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(other.path().join(SCENES_FILE_NAME)).unwrap();
    let (mut store, notice) = SceneStore::open(other.path());
    assert!(notice.unwrap().contains("scenes.json 无法读取"));
    let err = store.add(&draft("a", &["x"], &[]), 1).unwrap_err();
    assert!(matches!(err, SceneError::Store(_)) && err.to_string().contains("为避免覆盖不写入"), "{err}");
    assert!(other.path().join(SCENES_FILE_NAME).is_dir());
}

#[test]
fn normalisation_and_context_lines() {
    for (raw, want) in [("  Slack.EXE ", "slack"), ("notepad.exe.exe", "notepad"), ("com.Apple.Safari", "com.apple.safari"), (".exe", ""), ("微信.exe", "微信")]
    {
        assert_eq!(normalize_app_id(raw), want, "{raw:?}");
        assert_eq!(normalize_app_id(&normalize_app_id(raw)), want, "idempotent");
    }
    assert_eq!(clean_context_line(" a\u{7}\tb \n c ", 64).as_deref(), Some("a b c"));
    assert_eq!(clean_context_line(" \u{0} ", 64), None);
    assert_eq!(clean_context_line("abcdef", 5).as_deref(), Some("abcd…"));
}
