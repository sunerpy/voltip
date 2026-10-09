use super::*;
use crate::presets::{BuiltinPreset, PresetId};
use voltip_protocol::Platform;

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
    Scene {
        id: Uuid::new_v4(),
        name: d.name,
        enabled: d.enabled,
        matching: d.matching,
        overrides: d.overrides,
        created_at_ms: 1,
        updated_at_ms: 1,
        builtin: None,
    }
}

fn app(id: &str, title: Option<&str>) -> ForegroundApp {
    ForegroundApp { app_id: id.into(), name: id.into(), title: title.map(str::to_owned), window: None }
}

fn err(d: &SceneDraft) -> String {
    validate_scene_draft(d).unwrap_err().to_string()
}

/// A store on a host that keeps no built-in scenes: the user's scenes alone (the built-in ones are
/// tested on the desktops below).
fn open_user(dir: &std::path::Path) -> (SceneStore, Option<String>) {
    SceneStore::open_on(dir, Platform::Other, 0)
}

/// A draft is normalised on its own: name trimmed, apps normalised / de-duplicated / blanks
/// dropped, keywords trimmed and de-duplicated ignoring case, the language lower-cased, the prompt's
/// line endings normalised; blank language / prompt mean "follow the global setting".
#[test]
fn drafts_are_normalised() {
    let mut d = draft(" 聊天 ", &[" Slack.EXE", "slack", "  ", "WeChat.exe", "com.Tinyspeck.SlackMacGap"], &[" GitHub ", "github", "", "拉取请求"]);
    d.overrides = SceneOverrides {
        refine_enabled: Some(true),
        refine_preset: Some(PresetId::Builtin(BuiltinPreset::Punctuation)),
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
        (v.overrides.refine_enabled, v.overrides.refine_preset, v.overrides.output_mode),
        (Some(true), Some(PresetId::Builtin(BuiltinPreset::Punctuation)), Some(OutputMode::StreamingFinal))
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

/// Wire shapes: `match` (a Rust keyword) on the wire, unset overrides absent, drafts default to on
/// with nothing overridden, unknown fields refused, the sharing switches default app-name-on /
/// title-off, a take context without a scene omits it.
#[test]
fn wire_shapes() {
    let mut s = scene("聊天", &["slack"], &[]);
    s.id = Uuid::nil();
    s.overrides.refine_preset = Some(PresetId::Builtin(BuiltinPreset::Punctuation));
    let json = serde_json::to_string(&s).unwrap();
    assert_eq!(
        json,
        r#"{"id":"00000000-0000-0000-0000-000000000000","name":"聊天","enabled":true,"match":{"apps":["slack"],"title_contains":[]},"overrides":{"refine_preset":"punctuation"},"created_at_ms":1,"updated_at_ms":1}"#
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
        r#"{"name":"a","match":{"apps":["x"]},"overrides":{"refine_preset":"casual"}}"#,
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
    assert_eq!(SceneDraft::from(&s).overrides.refine_preset, Some(PresetId::Builtin(BuiltinPreset::Punctuation)));
}

/// docs/dictation.md §21: `scenes.json` written before presets stored a refine style under
/// `refine_style` (`default` / `punctuation` / `formal`). Those files still load — as 校对, 只加标点
/// and 书面语 — instead of being set aside as unusable, and the next save writes `refine_preset`.
#[test]
fn regression_scenes_written_before_presets_still_load() {
    let dir = tempfile::tempdir().unwrap();
    let old = r#"{"schema":1,"scenes":[
        {"id":"00000000-0000-4000-8000-000000000001","name":"聊天","enabled":true,"match":{"apps":["slack"],"title_contains":[]},"overrides":{"refine_style":"default"},"created_at_ms":1,"updated_at_ms":1},
        {"id":"00000000-0000-4000-8000-000000000002","name":"代码","enabled":true,"match":{"apps":["code"],"title_contains":[]},"overrides":{"refine_style":"punctuation"},"created_at_ms":1,"updated_at_ms":1},
        {"id":"00000000-0000-4000-8000-000000000003","name":"邮件","enabled":false,"match":{"apps":["outlook"],"title_contains":[]},"overrides":{"refine_style":"formal","prompt":"正式"},"created_at_ms":1,"updated_at_ms":1}
    ]}"#;
    std::fs::write(dir.path().join(SCENES_FILE_NAME), old).unwrap();
    let (mut store, notice) = open_user(dir.path());
    assert!(notice.is_none(), "not set aside: {notice:?}");
    assert!(corrupt_files(dir.path()).is_empty());
    let presets: Vec<_> = store.scenes().iter().map(|s| s.overrides.refine_preset).collect();
    assert_eq!(
        presets,
        [
            Some(PresetId::Builtin(BuiltinPreset::Proofread)),
            Some(PresetId::Builtin(BuiltinPreset::Punctuation)),
            Some(PresetId::Builtin(BuiltinPreset::Formal))
        ]
    );
    // The next save writes the preset under its own name; a restart reads the same list.
    let id = store.scenes()[0].id;
    let mut draft = SceneDraft::from(&store.scenes()[0]);
    draft.enabled = false;
    store.update(id, &draft, 2).unwrap();
    let written = std::fs::read_to_string(dir.path().join(SCENES_FILE_NAME)).unwrap();
    assert!(written.contains(r#""refine_preset": "proofread""#) && !written.contains("refine_style"), "{written}");
    let (again, notice) = open_user(dir.path());
    assert!(notice.is_none());
    assert_eq!(again.scenes().iter().map(|s| s.overrides.refine_preset).collect::<Vec<_>>(), presets);
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
    let (mut store, notice) = open_user(dir.path());
    assert!(notice.is_none() && store.scenes().is_empty());
    let a = store.add(&draft(" 聊天 ", &["Slack.exe"], &[]), 10).unwrap();
    let b = store.add(&draft("代码", &["code"], &["README"]), 20).unwrap();
    assert_eq!(store.scenes()[0].name, "聊天");
    assert_eq!(store.scenes()[0].matching.apps, ["slack"], "stored normalised");
    assert_eq!(open_user(dir.path()).0.scenes(), store.scenes());
    let mut changed = draft("聊天", &["slack", "wechat"], &[]);
    changed.enabled = false;
    changed.overrides.prompt = Some("口语化".into());
    store.update(a, &changed, 30).unwrap();
    let s = &store.scenes()[0];
    assert_eq!((s.enabled, s.matching.apps.len(), s.created_at_ms, s.updated_at_ms, s.overrides.prompt.as_deref()), (false, 2, 10, 30, Some("口语化")));
    store.reorder(&[b, a]).unwrap();
    assert_eq!(open_user(dir.path()).0.scenes()[0].id, b);
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
    assert_eq!(open_user(dir.path()).0.scenes().len(), 1);
    assert!(!dir.path().join("scenes.json.tmp").exists());
    assert!(format!("{store:?}").contains("scenes.json"));
    for i in 1..MAX_SCENES {
        store.add(&draft(&format!("s{i}"), &["x"], &[]), 1).unwrap();
    }
    assert!(store.add(&draft("one more", &["x"], &[]), 1).unwrap_err().to_string().contains("场景最多 50 个"));
    assert_eq!(open_user(dir.path()).0.scenes().len(), MAX_SCENES);
}

/// A file that cannot be used never stops the store: moved aside to `.corrupt-<secs>` (kept, never
/// overwritten), the store starts empty with a notice and writes the path again afterwards; an
/// unreadable path makes the store read-only.
#[test]
fn unusable_scene_files_are_quarantined_not_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(SCENES_FILE_NAME);
    std::fs::write(&path, b"{not json").unwrap();
    let (mut store, notice) = open_user(dir.path());
    let notice = notice.unwrap();
    assert!(notice.contains("scenes.json 无法使用") && notice.contains(".corrupt-"), "{notice}");
    assert!(store.scenes().is_empty() && !path.exists());
    assert_eq!(std::fs::read(dir.path().join(&corrupt_files(dir.path())[0])).unwrap(), b"{not json");
    std::fs::write(&path, br#"{"schema":9,"scenes":[]}"#).unwrap();
    assert!(open_user(dir.path()).1.unwrap().contains("schema 不是 1"));
    let raw = |name: &str, apps: &str| {
        format!(
            r#"{{"id":"{}","name":"{name}","enabled":true,"match":{{"apps":{apps}}},"overrides":{{}},"created_at_ms":1,"updated_at_ms":1}}"#,
            Uuid::new_v4()
        )
    };
    std::fs::write(&path, format!(r#"{{"schema":1,"scenes":[{},{}]}}"#, raw("A", r#"["x"]"#), raw("a", r#"["y"]"#))).unwrap();
    assert!(open_user(dir.path()).1.unwrap().contains("已有名为「A」"));
    std::fs::write(&path, format!(r#"{{"schema":1,"scenes":[{}]}}"#, raw("A", r#"["Slack.exe"]"#))).unwrap();
    assert!(open_user(dir.path()).1.unwrap().contains("不是规范形式"), "an id that is not normalised");
    std::fs::write(&path, format!(r#"{{"schema":1,"scenes":[{}]}}"#, raw("A", "[]"))).unwrap();
    assert!(open_user(dir.path()).1.unwrap().contains("至少要有一个应用"));
    assert_eq!(corrupt_files(dir.path()).len(), 5, "every bad file is kept");
    store.add(&draft("fresh", &["x"], &[]), 1).unwrap();
    assert_eq!(open_user(dir.path()).0.scenes()[0].name, "fresh");
    // A directory in place of the file: neither moved nor overwritten, the store refuses to write.
    let other = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(other.path().join(SCENES_FILE_NAME)).unwrap();
    let (mut store, notice) = open_user(other.path());
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

fn builtin_ids(store: &SceneStore) -> Vec<(Uuid, Option<BuiltinScene>)> {
    store.scenes().iter().map(|s| (s.id, s.builtin)).collect()
}

/// §18.10: a desktop's list always holds the seven built-in scenes. Opening appends the missing ones,
/// switched off, after the user's scenes, with the host's default applications, and writes them, so
/// a restart finds the same ids and adds nothing twice.
#[test]
fn builtin_scenes_are_filled_in_switched_off_after_the_users_and_never_twice() {
    let dir = tempfile::tempdir().unwrap();
    let (store, notice) = SceneStore::open_on(dir.path(), Platform::Windows, 5);
    assert!(notice.is_none());
    let categories: Vec<_> = store.scenes().iter().map(|s| s.builtin).collect();
    assert_eq!(categories, BuiltinScene::ALL.map(Some));
    for scene in store.scenes() {
        let category = scene.builtin.unwrap();
        assert_eq!(SceneDraft::from(scene), category.template(Platform::Windows));
        assert!(!scene.enabled);
        assert_eq!((scene.name.as_str(), scene.created_at_ms), (category.as_str(), 5));
    }
    assert!(store.scenes()[0].matching.apps.contains(&"code".to_owned()));
    let (again, _) = SceneStore::open_on(dir.path(), Platform::Windows, 9);
    assert_eq!(builtin_ids(&again), builtin_ids(&store), "the same ids, nothing added twice");

    // An older file with the user's scenes: they stay first and unchanged; one category the file
    // already has is not added again.
    let dir = tempfile::tempdir().unwrap();
    let old = r#"{"schema":1,"scenes":[
        {"id":"00000000-0000-4000-8000-000000000001","name":"聊天","enabled":true,"match":{"apps":["slack"],"title_contains":[]},"overrides":{"refine_style":"punctuation"},"created_at_ms":1,"updated_at_ms":1},
        {"id":"00000000-0000-4000-8000-000000000002","name":"chat","enabled":true,"match":{"apps":[],"title_contains":[]},"overrides":{},"created_at_ms":1,"updated_at_ms":2,"builtin":"chat"}
    ]}"#;
    std::fs::write(dir.path().join(SCENES_FILE_NAME), old).unwrap();
    let (store, notice) = SceneStore::open_on(dir.path(), Platform::Macos, 7);
    assert!(notice.is_none(), "{notice:?}");
    assert_eq!(store.scenes().len(), 8);
    assert_eq!((store.scenes()[0].name.as_str(), store.scenes()[0].builtin), ("聊天", None));
    let chat = &store.scenes()[1];
    assert_eq!((chat.builtin, chat.enabled, chat.updated_at_ms, chat.matching.apps.len()), (Some(BuiltinScene::Chat), true, 2, 0), "kept as stored");
    assert_eq!(store.scenes().iter().filter(|s| s.builtin == Some(BuiltinScene::Chat)).count(), 1);
    assert_eq!(store.scenes()[2].builtin, Some(BuiltinScene::Coding));
    assert!(store.scenes()[2].matching.apps.contains(&"com.microsoft.vscode".to_owned()), "macOS ids on a Mac");
}

/// Regression (plan 2.4): a built-in scene saved without applications, and a user scene that has a
/// category's name, survive the fill-in and a restart; nothing is set aside as unusable.
#[test]
fn regression_a_builtin_scene_without_apps_and_a_same_named_user_scene_survive_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, _) = SceneStore::open_on(dir.path(), Platform::Linux, 1);
    let mine = store.add(&draft("编程开发", &["code"], &[]), 2).unwrap();
    let also = store.add(&draft("coding", &["vim"], &[]), 3).unwrap();
    let legal = store.scenes().iter().find(|s| s.builtin == Some(BuiltinScene::Legal)).unwrap().clone();
    let mut on = SceneDraft::from(&legal);
    on.enabled = true;
    store.update(legal.id, &on, 4).unwrap();
    let coding = store.scenes().iter().find(|s| s.builtin == Some(BuiltinScene::Coding)).unwrap().clone();
    let mut none = SceneDraft::from(&coding);
    none.matching.apps.clear();
    store.update(coding.id, &none, 5).unwrap();
    let (again, notice) = SceneStore::open_on(dir.path(), Platform::Linux, 6);
    assert!(notice.is_none(), "{notice:?}");
    assert!(corrupt_files(dir.path()).is_empty());
    assert_eq!(again.scenes(), store.scenes());
    let names: Vec<_> = again.scenes().iter().filter(|s| s.builtin.is_none()).map(|s| (s.id, s.name.as_str())).collect();
    assert_eq!(names, [(mine, "编程开发"), (also, "coding")]);
    let legal = again.scenes().iter().find(|s| s.id == legal.id).unwrap();
    assert!(legal.enabled && legal.matching.apps.is_empty());
}

/// A built-in scene can be switched off and edited, not deleted or renamed; its category stays
/// through an update; 恢复默认 puts its applications and overrides back (its switch stays). They
/// count toward neither the cap nor the name rule of the user's scenes.
#[test]
fn builtin_scenes_keep_their_category_and_cannot_be_deleted_or_renamed() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, _) = SceneStore::open_on(dir.path(), Platform::Windows, 1);
    let office = store.scenes().iter().find(|s| s.builtin == Some(BuiltinScene::Office)).unwrap().clone();
    assert_eq!(store.remove(office.id).unwrap_err().to_string(), "scenes: 内置场景不能删除，可以关闭");
    let mut renamed = SceneDraft::from(&office);
    renamed.name = "写作".into();
    assert_eq!(store.update(office.id, &renamed, 2).unwrap_err().to_string(), "scenes: 内置场景不能改名");
    let mut edited = SceneDraft::from(&office);
    edited.enabled = true;
    edited.matching.apps = vec!["notepad".into()];
    edited.overrides.prompt = None;
    edited.overrides.refine_preset = Some(PresetId::Builtin(BuiltinPreset::Notes));
    store.update(office.id, &edited, 3).unwrap();
    let now = store.scenes().iter().find(|s| s.id == office.id).unwrap();
    assert_eq!((now.builtin, now.enabled, now.matching.apps.as_slice()), (Some(BuiltinScene::Office), true, ["notepad".to_owned()].as_slice()));
    store.restore(office.id, 4).unwrap();
    let back = store.scenes().iter().find(|s| s.id == office.id).unwrap();
    let template = BuiltinScene::Office.template(Platform::Windows);
    assert_eq!((back.enabled, &back.matching, &back.overrides, back.updated_at_ms), (true, &template.matching, &template.overrides, 4));
    let user = store.add(&draft("办公", &["x"], &[]), 5).unwrap();
    assert_eq!(store.restore(user, 6).unwrap_err().to_string(), "scenes: 只有内置场景可以恢复默认");
    assert!(store.restore(Uuid::new_v4(), 6).unwrap_err().to_string().contains("没有 id"));
    // A user scene still needs an application; the cap counts the user's scenes only.
    assert!(store.add(&draft("空", &[], &[]), 7).unwrap_err().to_string().contains("至少要有一个应用"));
    let mut empty = draft("办公", &[], &[]);
    empty.matching.apps.clear();
    assert!(store.update(user, &empty, 7).unwrap_err().to_string().contains("至少要有一个应用"));
    for i in 1..MAX_SCENES {
        store.add(&draft(&format!("s{i}"), &["x"], &[]), 8).unwrap();
    }
    assert_eq!(store.scenes().len(), MAX_SCENES + BuiltinScene::ALL.len());
    assert!(store.add(&draft("one more", &["x"], &[]), 9).unwrap_err().to_string().contains("场景最多 50 个"));
    // Two scenes of one category, or a built-in scene under another name, make a file unusable.
    let raw = |name: &str, builtin: &str| {
        format!(
            r#"{{"id":"{}","name":"{name}","enabled":false,"match":{{"apps":[]}},"overrides":{{}},"created_at_ms":1,"updated_at_ms":1,"builtin":"{builtin}"}}"#,
            Uuid::new_v4()
        )
    };
    let path = dir.path().join(SCENES_FILE_NAME);
    std::fs::write(&path, format!(r#"{{"schema":1,"scenes":[{},{}]}}"#, raw("legal", "legal"), raw("legal", "legal"))).unwrap();
    assert!(SceneStore::open_on(dir.path(), Platform::Windows, 1).1.unwrap().contains("出现了两次"));
    std::fs::write(&path, format!(r#"{{"schema":1,"scenes":[{}]}}"#, raw("法律", "legal"))).unwrap();
    assert!(SceneStore::open_on(dir.path(), Platform::Windows, 1).1.unwrap().contains("名称必须是 legal"));
}

/// The user's scenes come first (the built-in ones are appended), so one of theirs for the same
/// application wins; a built-in scene that is off, or lists no application, never matches.
#[test]
fn user_scenes_come_first_and_an_off_or_empty_builtin_scene_never_matches() {
    let dir = tempfile::tempdir().unwrap();
    let (mut store, _) = SceneStore::open_on(dir.path(), Platform::Windows, 1);
    let coding = store.scenes().iter().find(|s| s.builtin == Some(BuiltinScene::Coding)).unwrap().clone();
    assert!(match_scene(store.scenes(), &app("Code.exe", None)).is_none(), "off by default");
    let mut on = SceneDraft::from(&coding);
    on.enabled = true;
    store.update(coding.id, &on, 2).unwrap();
    assert_eq!(match_scene(store.scenes(), &app("Code.exe", None)).map(|s| s.builtin), Some(Some(BuiltinScene::Coding)));
    // A new scene of the user's goes before the built-in ones, so it matches first.
    let mine = store.add(&draft("我的代码", &["code"], &[]), 3).unwrap();
    assert_eq!(store.scenes()[0].id, mine);
    assert_eq!(match_scene(store.scenes(), &app("code", None)).map(|s| s.id), Some(mine), "the user's scene first");
    // Moved after the built-in one, it no longer wins; the next new one still goes first.
    let ids: Vec<Uuid> = store.scenes().iter().map(|s| s.id).filter(|id| *id != mine).chain(std::iter::once(mine)).collect();
    store.reorder(&ids).unwrap();
    assert_eq!(match_scene(store.scenes(), &app("code", None)).map(|s| s.builtin), Some(Some(BuiltinScene::Coding)));
    let next = store.add(&draft("另一个", &["vim"], &[]), 4).unwrap();
    assert_eq!(store.scenes()[0].id, next);
    let legal = store.scenes().iter().find(|s| s.builtin == Some(BuiltinScene::Legal)).unwrap().clone();
    let mut legal_on = SceneDraft::from(&legal);
    legal_on.enabled = true;
    store.update(legal.id, &legal_on, 4).unwrap();
    assert!(
        store.scenes().iter().filter(|s| s.builtin == Some(BuiltinScene::Legal)).all(|s| match_scene(std::slice::from_ref(s), &app("winword", None)).is_none())
    );
    let to_ref = store.scenes().iter().find(|s| s.id == coding.id).unwrap().to_ref();
    assert_eq!(to_ref.builtin, Some(BuiltinScene::Coding));
    assert!(serde_json::to_string(&to_ref).unwrap().ends_with(r#""name":"coding","builtin":"coding"}"#));
}

/// The phone (user decision 2026-10-01): its list holds the built-in scenes without applications,
/// and a scene of the user's need not name one — the user picks a take's scene there. A desktop
/// still refuses a scene of the user's without an application.
#[test]
fn a_phone_keeps_the_builtin_scenes_and_scenes_without_applications() {
    let dir = tempfile::tempdir().unwrap();
    let (mut phone, notice) = SceneStore::open_on(dir.path(), Platform::Android, 5);
    assert!(notice.is_none());
    assert_eq!(phone.scenes().iter().map(|s| s.builtin).collect::<Vec<_>>(), BuiltinScene::ALL.map(Some));
    assert!(phone.scenes().iter().all(|s| s.matching.apps.is_empty()));
    let id = phone.add(&draft("会议", &[], &[]), 6).unwrap();
    assert!(phone.scenes().iter().any(|s| s.id == id && s.matching.apps.is_empty()));
    phone.update(id, &draft("会议纪要", &[], &[]), 7).unwrap();
    let (again, notice) = SceneStore::open_on(dir.path(), Platform::Android, 8);
    assert!(notice.is_none(), "the saved list is valid on the phone");
    assert!(again.scenes().iter().any(|s| s.id == id && s.name == "会议纪要"));
    let desktop = tempfile::tempdir().unwrap();
    let (mut store, _) = SceneStore::open_on(desktop.path(), Platform::Windows, 5);
    assert!(matches!(store.add(&draft("会议", &[], &[]), 6), Err(SceneError::Invalid(m)) if m.contains("至少要有一个应用")));
}

/// docs/dictation.md §23: the local speech service reads the app's `scenes.json` with the store's
/// checks and never touches it: a file without the built-in scenes gets them in memory only, and an
/// unusable file is reported, neither moved nor rewritten.
#[test]
fn reading_the_scenes_never_moves_or_writes_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(SCENES_FILE_NAME);
    assert_eq!(SceneStore::read_on(dir.path(), Platform::Other, 1).unwrap(), Vec::<Scene>::new(), "missing: nothing");
    assert!(!path.exists(), "a missing file is not created");
    let user = r#"{"schema":1,"scenes":[{"id":"00000000-0000-4000-8000-000000000001","name":"聊天","enabled":true,"match":{"apps":["slack"],"title_contains":[]},"overrides":{},"created_at_ms":1,"updated_at_ms":1}]}"#;
    std::fs::write(&path, user).unwrap();
    let scenes = SceneStore::read_on(dir.path(), Platform::Linux, 3).unwrap();
    assert_eq!(scenes.len(), 1 + BuiltinScene::ALL.len());
    assert_eq!((scenes[0].name.as_str(), scenes[0].builtin), ("聊天", None));
    assert!(scenes[1..].iter().all(|s| s.builtin.is_some() && !s.enabled), "built-ins after the user's, switched off");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), user, "the file is not rewritten");
    std::fs::write(&path, b"{not json").unwrap();
    let err = SceneStore::read_on(dir.path(), Platform::Linux, 3).unwrap_err();
    assert!(err.contains("scenes.json 无法使用"), "{err}");
    assert_eq!(std::fs::read(&path).unwrap(), b"{not json", "left where it is");
    assert!(corrupt_files(dir.path()).is_empty(), "nothing set aside");
}
