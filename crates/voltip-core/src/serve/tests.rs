#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::sync::Arc;

use uuid::Uuid;
use voltip_protocol::Platform;

use super::*;
use crate::dictation::fakes::{FAKE_REFINE_MODEL, FakeRefiner, FakeTranscriber};
use crate::dictation::ports::{Refiner, Transcriber};
use crate::engines::{BuiltIn, ChineseScript, ResolvedEngines, UserSecrets};
use crate::models::CancelToken;
use crate::presets::{BuiltinPreset, CustomPreset, PresetId, PresetStore, TakePreset};
use crate::scenes::{BuiltinScene, LANGUAGE_AUTO, Scene, SceneMatch, SceneOverrides};
use crate::settings::Settings;
use crate::vocabulary::{DictionaryEntry, EntrySource, ReplacementRule, RuleKind, Vocabulary};

const BUILT_IN: BuiltIn = BuiltIn { asr_url: Some("https://asr.test"), ..BuiltIn::EMPTY };

fn user_scene(name: &str, apps: &[&str], overrides: SceneOverrides) -> Scene {
    Scene {
        id: Uuid::new_v4(),
        name: name.into(),
        enabled: true,
        matching: SceneMatch { apps: apps.iter().map(|a| (*a).to_owned()).collect(), title_contains: Vec::new() },
        overrides,
        created_at_ms: 1,
        updated_at_ms: 1,
        builtin: None,
    }
}

fn builtin(category: BuiltinScene) -> Scene {
    let draft = category.template(Platform::Linux);
    Scene {
        id: Uuid::new_v4(),
        name: draft.name,
        enabled: false,
        matching: draft.matching,
        overrides: draft.overrides,
        created_at_ms: 1,
        updated_at_ms: 1,
        builtin: Some(category),
    }
}

fn custom(name: &str, prompt: &str) -> CustomPreset {
    CustomPreset { id: Uuid::new_v4(), name: name.into(), prompt: prompt.into(), created_at_ms: 1, updated_at_ms: 1 }
}

fn dict_entry(term: &str, heard: &[&str]) -> DictionaryEntry {
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

fn literal_rule(pattern: &str, replacement: &str) -> ReplacementRule {
    ReplacementRule {
        id: Uuid::new_v4(),
        name: format!("rule {pattern}"),
        kind: RuleKind::Literal,
        pattern: pattern.into(),
        replacement: replacement.into(),
        case_sensitive: true,
        enabled: true,
        created_at_ms: 1,
        updated_at_ms: 1,
    }
}

struct Lists {
    settings: Settings,
    presets: Vec<CustomPreset>,
    scenes: Vec<Scene>,
    vocabulary: Arc<Vocabulary>,
}

impl Lists {
    fn new() -> Self {
        Self { settings: Settings::default(), presets: Vec::new(), scenes: Vec::new(), vocabulary: Arc::new(Vocabulary::compile(&[], &[])) }
    }

    fn catalog(&self) -> Catalog<'_> {
        Catalog { settings: &self.settings, presets: &self.presets, scenes: &self.scenes, vocabulary: &self.vocabulary }
    }
}

// ---- the `model` grammar --------------------------------------------------------------------

#[test]
fn the_model_names_the_default_or_a_choice_of_its_own() {
    for default in [None, Some(""), Some("voltip"), Some(" voltip "), Some("whisper-1"), Some("gpt-4o-transcribe"), Some("voltipx")] {
        assert_eq!(parse_model(default).unwrap(), Profile::Default, "{default:?}");
    }
    assert_eq!(parse_model(Some("voltip:raw")).unwrap(), Profile::Request { raw: true, scene: None, preset: None });
    assert_eq!(
        parse_model(Some("voltip: scene = coding ; preset=prompt ;")).unwrap(),
        Profile::Request { raw: false, scene: Some("coding".into()), preset: Some("prompt".into()) }
    );
    assert_eq!(parse_model(Some("voltip:raw;scene=编程开发")).unwrap(), Profile::Request { raw: true, scene: Some("编程开发".into()), preset: None });
    assert_eq!(parse_model(Some("voltip:")).unwrap(), Profile::Request { raw: false, scene: None, preset: None }, "nothing chosen: the app's settings");
    for (bad, why) in [
        ("voltip:raw;raw", "重复"),
        ("voltip:scene=a;scene=b", "重复"),
        ("voltip:raw;preset=prompt", "不能同时"),
        ("voltip:preset=", "缺少名称"),
        ("voltip:fast", "无法识别"),
        ("voltip:speed=2", "无法识别"),
    ] {
        let err = parse_model(Some(bad)).unwrap_err();
        assert!(err.0.contains(why), "{bad}: {err}");
    }
}

// ---- precedence -----------------------------------------------------------------------------

#[test]
fn a_request_of_its_own_inherits_nothing_of_the_defaults() {
    let mut lists = Lists::new();
    let chat = user_scene("聊天", &["paseo"], SceneOverrides { refine_preset: Some(PresetId::Builtin(BuiltinPreset::Chat)), ..SceneOverrides::default() });
    lists.scenes = vec![chat.clone(), builtin(BuiltinScene::Coding)];
    let defaults = Defaults { app: Some("paseo".into()), preset: Some(PresetId::Builtin(BuiltinPreset::Notes)), refine: Some(false), ..Defaults::default() };
    // The defaults: the app matches the chat scene, the explicit preset and switch win over it.
    let recipe = resolve_recipe(&defaults, &Profile::Default, None, lists.catalog()).unwrap();
    assert_eq!(recipe.scene.as_ref().map(|s| s.id), Some(chat.id));
    assert_eq!((recipe.preset.clone(), recipe.refine), (TakePreset::Builtin(BuiltinPreset::Notes), false));
    assert_eq!(recipe.hints.context.app_name.as_deref(), Some("paseo"), "the app name is shared by default");
    // A request of its own: no app matching, no default preset, no default switch.
    let own = resolve_recipe(&defaults, &parse_model(Some("voltip:")).unwrap(), None, lists.catalog()).unwrap();
    assert_eq!(own.scene, None);
    assert_eq!((own.preset, own.refine), (TakePreset::Builtin(BuiltinPreset::Proofread), true), "the settings' preset and switch");
    assert_eq!(own.hints.context.app_name, None);
    // Its own scene brings that scene's preset.
    let coding = resolve_recipe(&defaults, &parse_model(Some("voltip:scene=coding")).unwrap(), None, lists.catalog()).unwrap();
    assert_eq!(coding.scene.as_ref().and_then(|s| s.builtin), Some(BuiltinScene::Coding));
    assert_eq!(coding.preset, TakePreset::Builtin(BuiltinScene::Coding.preset()));
    assert!(coding.hints.context.instruction.is_some(), "the scene's instruction goes to the clean-up");
    assert!(coding.vocabulary.glossary().len() > lists.vocabulary.glossary().len(), "a built-in scene adds its term pack");
    // raw switches the clean-up off whatever the scene says.
    let raw = resolve_recipe(&defaults, &parse_model(Some("voltip:raw;scene=聊天")).unwrap(), None, lists.catalog()).unwrap();
    assert!(!raw.refine);
    assert_eq!(raw.preset_ref(), None, "no preset is named when nothing is refined");
}

#[test]
fn an_explicit_scene_applies_even_when_switched_off_and_a_missing_one_is_an_error() {
    let mut lists = Lists::new();
    let mut off = user_scene("周报", &["app"], SceneOverrides { chinese_script: Some(ChineseScript::Traditional), ..SceneOverrides::default() });
    off.enabled = false;
    lists.scenes = vec![off.clone()];
    let defaults = Defaults { scene: Some(off.id), ..Defaults::default() };
    let recipe = resolve_recipe(&defaults, &Profile::Default, None, lists.catalog()).unwrap();
    assert_eq!((recipe.scene.map(|s| s.id), recipe.script), (Some(off.id), ChineseScript::Traditional), "as the app's pinned scene");
    // `--app` matches enabled scenes only, as the foreground probe does.
    let by_app = resolve_recipe(&Defaults { app: Some("app".into()), ..Defaults::default() }, &Profile::Default, None, lists.catalog()).unwrap();
    assert_eq!(by_app.scene, None);
    // A request naming what is not there is refused; a default that went away is not.
    let err = resolve_recipe(&Defaults::default(), &parse_model(Some("voltip:scene=nope")).unwrap(), None, lists.catalog()).unwrap_err();
    assert!(err.0.contains("没有名为「nope」的场景（可选值见 GET /v1/models）"), "{err}");
    assert!(resolve_recipe(&Defaults::default(), &parse_model(Some("voltip:preset=nope")).unwrap(), None, lists.catalog()).is_err());
    let gone = Defaults { scene: Some(Uuid::new_v4()), preset: Some(PresetId::Custom(Uuid::new_v4())), ..Defaults::default() };
    let recipe = resolve_recipe(&gone, &Profile::Default, None, lists.catalog()).unwrap();
    assert_eq!((recipe.scene, recipe.preset), (None, TakePreset::Builtin(BuiltinPreset::Proofread)), "no scene; a gone custom preset is 校对");
}

#[test]
fn the_language_comes_from_the_flag_the_scene_the_request_and_the_settings_in_that_order() {
    let mut lists = Lists::new();
    lists.settings.engines.language = Some("zh".into());
    let english = user_scene("English", &["x"], SceneOverrides { language: Some("en".into()), ..SceneOverrides::default() });
    let auto = user_scene("Auto", &["y"], SceneOverrides { language: Some(LANGUAGE_AUTO.into()), ..SceneOverrides::default() });
    lists.scenes = vec![english.clone(), auto.clone()];
    let language = |defaults: &Defaults, model: &str, asked: Option<&str>| {
        resolve_recipe(defaults, &parse_model(Some(model)).unwrap(), asked, lists.catalog()).unwrap().language
    };
    let none = Defaults::default();
    assert_eq!(language(&none, "voltip", None), Some("zh".into()), "the settings");
    assert_eq!(language(&none, "voltip", Some(" EN ")), Some("en".into()), "the request, lower-cased");
    assert_eq!(language(&none, "voltip", Some("auto")), None, "auto: no hint");
    assert_eq!(language(&none, "voltip:scene=English", Some("ja")), Some("en".into()), "the scene over the request");
    assert_eq!(language(&none, "voltip:scene=Auto", Some("ja")), None, "a scene's auto is no hint");
    let forced = Defaults { language: Some("yue".into()), ..Defaults::default() };
    assert_eq!(language(&forced, "voltip:scene=English", Some("ja")), Some("yue".into()), "the flag over everything");
    let forced_auto = Defaults { language: Some(LANGUAGE_AUTO.into()), ..Defaults::default() };
    assert_eq!(language(&forced_auto, "voltip", Some("ja")), None);
    let err = resolve_recipe(&none, &Profile::Default, Some("zh_CN!"), lists.catalog()).unwrap_err();
    assert!(err.0.contains("语言代码"), "{err}");
    // The script: the flag, then the scene, then the settings.
    lists.settings.engines.chinese_script = ChineseScript::AsIs;
    let script = |defaults: &Defaults| resolve_recipe(defaults, &Profile::Default, None, lists.catalog()).unwrap().script;
    assert_eq!(script(&Defaults::default()), ChineseScript::AsIs);
    assert_eq!(script(&Defaults { script: Some(ChineseScript::Traditional), ..Defaults::default() }), ChineseScript::Traditional);
}

#[test]
fn the_host_choices_are_checked_against_the_lists_it_starts_with() {
    let presets = vec![custom("周报", "整理成周报")];
    let scenes = vec![user_scene("会议", &["zoom"], SceneOverrides::default()), builtin(BuiltinScene::Legal)];
    let choices = DefaultChoices {
        scene: Some("法律".into()),
        app: Some(" Paseo.EXE ".into()),
        preset: Some("周报".into()),
        refine: Some(true),
        language: Some(" ZH ".into()),
        script: None,
    };
    let defaults = choices.resolve(&presets, &scenes).unwrap();
    assert_eq!(defaults.scene, Some(scenes[1].id), "a built-in category by its Chinese name");
    assert_eq!((defaults.app.as_deref(), defaults.preset), (Some("paseo"), Some(PresetId::Custom(presets[0].id))));
    assert_eq!(defaults.language.as_deref(), Some("zh"));
    for selector in ["legal", "Legal", "法律"] {
        assert_eq!(find_scene(selector, &scenes).map(|s| s.id), Some(scenes[1].id), "{selector}");
    }
    assert_eq!(find_scene(&scenes[0].id.to_string(), &scenes).map(|s| s.name.as_str()), Some("会议"));
    assert_eq!(find_preset("提示词优化", &presets), Some(PresetId::Builtin(BuiltinPreset::Prompt)));
    assert_eq!(find_preset(&Uuid::new_v4().to_string(), &presets), None, "a UUID that is not a preset");
    assert!(DefaultChoices { scene: Some("nope".into()), ..DefaultChoices::default() }.resolve(&presets, &scenes).unwrap_err().0.contains("场景"));
    assert!(DefaultChoices { preset: Some("nope".into()), ..DefaultChoices::default() }.resolve(&presets, &scenes).unwrap_err().0.contains("预设"));
    assert!(DefaultChoices { app: Some(" ".into()), ..DefaultChoices::default() }.resolve(&presets, &scenes).is_err());
    assert!(DefaultChoices { language: Some("!!".into()), ..DefaultChoices::default() }.resolve(&presets, &scenes).is_err());
}

#[test]
fn every_listed_model_selects_what_it_names() {
    let mut lists = Lists::new();
    lists.presets = vec![custom("周报", "整理成周报")];
    lists.scenes = vec![user_scene("会议", &["zoom"], SceneOverrides::default()), builtin(BuiltinScene::Coding)];
    let models = model_list(&lists.presets, &lists.scenes);
    assert_eq!(models.len(), 2 + BuiltinPreset::ALL.len() + 1 + 2);
    assert_eq!(models[0].id, "voltip");
    for model in &models {
        let profile = parse_model(Some(&model.id)).unwrap();
        let recipe = resolve_recipe(&Defaults::default(), &profile, None, lists.catalog()).unwrap();
        if let Some(preset) = model.id.strip_prefix("voltip:preset=") {
            assert_eq!(recipe.preset.to_ref().id.to_wire(), preset, "{model:?}");
        }
        if let Some(scene) = model.id.strip_prefix("voltip:scene=") {
            let applied = recipe.scene.expect("a scene");
            assert!(applied.id.to_string() == scene || applied.builtin.map(BuiltinScene::as_str) == Some(scene), "{model:?}");
        }
    }
    assert!(models.iter().any(|m| m.id == "voltip:scene=coding" && m.name == "编程开发"), "a built-in scene by its category");
}

// ---- processing -----------------------------------------------------------------------------

fn tone(seconds: f64, amplitude: f64) -> Vec<i16> {
    let n = (seconds * RATE as f64) as usize;
    (0..n).map(|i| (amplitude * (2.0 * std::f64::consts::PI * 440.0 * i as f64 / RATE as f64).sin()) as i16).collect()
}

fn pcm(dir: &Path, samples: &[i16]) -> PcmFile {
    let path = dir.join(format!("{}.pcm", Uuid::new_v4()));
    let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    std::fs::write(&path, bytes).unwrap();
    PcmFile { path, samples: samples.len() as u64 }
}

struct Rig {
    service: Service,
    transcriber: Arc<FakeTranscriber>,
    refiner: Option<Arc<FakeRefiner>>,
    dir: tempfile::TempDir,
}

fn rig_with(lists: Lists, transcriber: FakeTranscriber, refiner: Option<FakeRefiner>, defaults: Defaults) -> Rig {
    let transcriber = Arc::new(transcriber);
    let refiner = refiner.map(Arc::new);
    let engines = ResolvedEngines::resolve(&lists.settings.engines, &UserSecrets::default(), &BUILT_IN);
    let state = ServeState {
        settings: lists.settings,
        presets: Arc::new(lists.presets),
        scenes: Arc::new(lists.scenes),
        vocabulary: lists.vocabulary,
        engines,
        transcriber: transcriber.clone() as Arc<dyn Transcriber>,
        refiner: refiner.clone().map(|r| r as Arc<dyn Refiner>),
        segmenter: None,
    };
    Rig { service: Service::new(Arc::new(PushedState::new(state)), defaults), transcriber, refiner, dir: tempfile::tempdir().unwrap() }
}

fn rig(transcriber: FakeTranscriber, refiner: Option<FakeRefiner>) -> Rig {
    rig_with(Lists::new(), transcriber, refiner, Defaults::default())
}

fn request(model: &str) -> ServeRequest {
    ServeRequest { model: Some(model.into()), language: None }
}

#[tokio::test]
async fn a_take_goes_through_the_script_the_dictionary_the_clean_up_and_the_rules() {
    let mut lists = Lists::new();
    // The recogniser answers in Traditional; the dictionary only knows the Simplified mishearing:
    // it matches because the text is brought to the script first (§17).
    lists.vocabulary = Arc::new(Vocabulary::compile(&[dict_entry("Voltip", &["发音"])], &[literal_rule("世界", "World")]));
    let r = rig_with(lists, FakeTranscriber::ok("發音說你好世界"), Some(FakeRefiner::ok("Voltip 说：你好，世界。")), Defaults::default());
    let audio = pcm(r.dir.path(), &tone(3.0, 3000.0));
    let out = r.service.transcribe(&audio, &request("voltip"), &CancelToken::new()).await.unwrap();
    assert_eq!(out.raw_text, "发音说你好世界", "the recogniser's text in Simplified");
    let refiner = r.refiner.as_ref().unwrap();
    assert_eq!(refiner.inputs()[0].0, "Voltip说你好世界", "the refiner sees the dictionary's correction");
    assert_eq!(out.text, "Voltip 说：你好，World。", "the rules run after the clean-up");
    assert!(out.refined && out.refine_model.as_deref() == Some(FAKE_REFINE_MODEL));
    assert_eq!(out.preset.map(|p| p.id), Some(PresetId::Builtin(BuiltinPreset::Proofread)));
    assert_eq!((out.duration_ms, out.segments), (3000, 1));
    assert_eq!(r.transcriber.durations_ms(), vec![3000], "the whole take as one WAV");
}

#[tokio::test]
async fn with_the_script_as_is_the_dictionary_sees_the_recognisers_text() {
    let mut lists = Lists::new();
    lists.settings.engines.chinese_script = ChineseScript::AsIs;
    lists.settings.engines.refine_enabled = false;
    lists.vocabulary = Arc::new(Vocabulary::compile(&[dict_entry("Voltip", &["发音"])], &[]));
    let r = rig_with(lists, FakeTranscriber::ok("發音說"), None, Defaults::default());
    let out = r.service.transcribe(&pcm(r.dir.path(), &tone(2.0, 3000.0)), &request("voltip"), &CancelToken::new()).await.unwrap();
    assert_eq!(out.text, "發音說", "Traditional text does not match the Simplified mishearing");
    assert_eq!((out.refined, out.refine_error, out.preset), (false, None, None), "no clean-up asked for");
}

#[tokio::test]
async fn silence_and_short_takes_are_empty_and_never_sent() {
    let r = rig(FakeTranscriber::ok("不该出现"), Some(FakeRefiner::ok("不该出现")));
    for samples in [vec![0i16; RATE as usize * 3], tone(0.2, 3000.0), Vec::new()] {
        let out = r.service.transcribe(&pcm(r.dir.path(), &samples), &request("voltip"), &CancelToken::new()).await.unwrap();
        assert_eq!((out.text.as_str(), out.segments), ("", 0));
    }
    assert_eq!(r.transcriber.calls(), 0);
    assert_eq!(r.refiner.as_ref().unwrap().calls(), 0);
}

#[tokio::test]
async fn a_failed_clean_up_keeps_the_text_and_recognition_errors_are_typed() {
    let r = rig(FakeTranscriber::ok("你好"), Some(FakeRefiner::err("down")));
    let out = r.service.transcribe(&pcm(r.dir.path(), &tone(2.0, 3000.0)), &request("voltip"), &CancelToken::new()).await.unwrap();
    assert_eq!((out.text.as_str(), out.refined), ("你好", false));
    assert!(out.refine_error.is_some());
    let unconfigured = rig(FakeTranscriber::ok("你好"), None);
    let out = unconfigured.service.transcribe(&pcm(unconfigured.dir.path(), &tone(2.0, 3000.0)), &request("voltip"), &CancelToken::new()).await.unwrap();
    assert_eq!(out.refine_error.as_deref(), Some(crate::dictation::steps::REFINE_UNCONFIGURED));
    let quota = rig(FakeTranscriber::quota(), None);
    let err = quota.service.transcribe(&pcm(quota.dir.path(), &tone(2.0, 3000.0)), &request("voltip"), &CancelToken::new()).await.unwrap_err();
    assert!(matches!(err, ServeError::Quota(_)), "{err:?}");
    let down = rig(FakeTranscriber::err("503"), None);
    let err = down.service.transcribe(&pcm(down.dir.path(), &tone(2.0, 3000.0)), &request("voltip"), &CancelToken::new()).await.unwrap_err();
    assert!(matches!(err, ServeError::Upstream(_)), "{err:?}");
    let invalid = rig(FakeTranscriber::ok("你好"), None);
    let err = invalid.service.transcribe(&pcm(invalid.dir.path(), &tone(2.0, 3000.0)), &request("voltip:scene=nope"), &CancelToken::new()).await.unwrap_err();
    assert!(matches!(err, ServeError::Invalid(_)), "{err:?}");
    assert_eq!(invalid.transcriber.calls(), 0, "a refused request is not recognised");
}

#[tokio::test]
async fn recognition_that_is_not_configured_is_not_ready() {
    let mut lists = Lists::new();
    lists.settings.engines.asr_provider = crate::providers::ProviderId::Aliyun;
    let r = rig_with(lists, FakeTranscriber::ok("你好"), None, Defaults::default());
    let reason = r.service.ready().unwrap_err();
    assert!(reason.contains("识别服务未配置"), "{reason}");
    let err = r.service.transcribe(&pcm(r.dir.path(), &tone(2.0, 3000.0)), &request("voltip"), &CancelToken::new()).await.unwrap_err();
    assert!(matches!(err, ServeError::NotReady(_)));
    assert!(rig(FakeTranscriber::ok("x"), None).service.ready().is_ok());
}

/// 150 s: speech, a 40 s silence, speech again.
fn long_take() -> Vec<i16> {
    let mut samples = tone(50.0, 3000.0);
    samples.extend(vec![0i16; RATE as usize * 40]);
    samples.extend(tone(60.0, 3000.0));
    samples
}

#[tokio::test]
async fn a_long_take_is_recognised_in_segments_as_the_app_does() {
    let r = rig(FakeTranscriber::numbered(0, &[2, 3], std::time::Duration::ZERO), Some(FakeRefiner::ok("整理后的全文。")));
    let audio = pcm(r.dir.path(), &long_take());
    let out = r.service.transcribe(&audio, &request("voltip"), &CancelToken::new()).await.unwrap();
    let durations = r.transcriber.durations_ms();
    assert!(durations.iter().all(|d| *d <= 30_000), "segments of at most 30 s: {durations:?}");
    assert!(out.segments >= 5, "{out:?}");
    assert!(out.raw_text.contains("[未识别 00:00:"), "the second segment failed twice: {}", out.raw_text);
    assert!(out.raw_text.starts_with("第1段。"), "{}", out.raw_text);
    // Every segment once plus one retry, minus the silent ones: fewer calls than that means the
    // silence in the middle was never sent.
    assert!((r.transcriber.calls() as u32) < out.segments + 1, "a silent segment was skipped: {} calls, {} segments", r.transcriber.calls(), out.segments);
    assert_eq!(r.transcriber.calls(), durations.len());
    assert_eq!((out.text.as_str(), out.refined), ("整理后的全文。", true), "the whole text is cleaned up once");
    assert_eq!(out.duration_ms, 150_000);
}

#[tokio::test]
async fn a_long_text_past_two_thousand_characters_is_not_cleaned_up() {
    let r = rig(FakeTranscriber::numbered(500, &[], std::time::Duration::ZERO), Some(FakeRefiner::ok("不该出现")));
    let out = r.service.transcribe(&pcm(r.dir.path(), &tone(150.0, 3000.0)), &request("voltip"), &CancelToken::new()).await.unwrap();
    assert!(out.raw_text.chars().count() > crate::dictation::long::REFINE_MAX_CHARS);
    assert_eq!(out.refine_error.as_deref(), Some(crate::dictation::long::REFINE_SKIPPED));
    assert_eq!(r.refiner.as_ref().unwrap().calls(), 0);
}

#[tokio::test]
async fn a_cancelled_long_take_starts_no_further_segment() {
    let r = rig(FakeTranscriber::numbered(0, &[], std::time::Duration::ZERO), None);
    let cancel = CancelToken::new();
    cancel.cancel();
    let err = r.service.transcribe(&pcm(r.dir.path(), &tone(150.0, 3000.0)), &request("voltip"), &cancel).await.unwrap_err();
    assert_eq!(err, ServeError::Cancelled);
    assert_eq!(r.transcriber.calls(), 0);
}

#[tokio::test]
async fn a_long_take_with_nothing_recognised_reports_the_recognisers_error() {
    let r = rig(FakeTranscriber::err("down"), None);
    let err = r.service.transcribe(&pcm(r.dir.path(), &tone(130.0, 3000.0)), &request("voltip"), &CancelToken::new()).await.unwrap_err();
    assert!(matches!(err, ServeError::Upstream(_)), "{err:?}");
    let silent = rig(FakeTranscriber::ok("x"), None);
    let out = silent.service.transcribe(&pcm(silent.dir.path(), &vec![0i16; RATE as usize * 130]), &request("voltip"), &CancelToken::new()).await.unwrap();
    assert_eq!((out.text.as_str(), out.segments, silent.transcriber.calls()), ("", 0, 0), "a silent long take is empty");
}

// ---- the headless server's files ------------------------------------------------------------

fn file_source(dir: &Path, transcriber: Arc<FakeTranscriber>) -> (FileSource, Vec<String>) {
    let factory: crate::dictation::EngineFactory = Arc::new(move |_| (transcriber.clone() as Arc<dyn Transcriber>, None));
    FileSource::open(FileSourceConfig {
        data_dir: dir.to_path_buf(),
        platform: Platform::Linux,
        overrides: EngineOverrides::default(),
        secrets: UserSecrets::default(),
        built_in: BUILT_IN,
        models: None,
        factory,
        segmenter: None,
    })
    .unwrap()
}

#[test]
fn the_files_are_re_read_when_they_change_and_a_broken_one_keeps_its_last_content() {
    let dir = tempfile::tempdir().unwrap();
    let transcriber = Arc::new(FakeTranscriber::ok("x"));
    let (source, notices) = file_source(dir.path(), transcriber);
    assert!(notices.is_empty());
    assert_eq!(source.current().scenes.iter().filter(|s| s.builtin.is_some()).count(), BuiltinScene::ALL.len(), "built-ins in memory");
    assert!(!dir.path().join("scenes.json").exists(), "nothing written");
    let (mut presets, _) = PresetStore::open(dir.path());
    let id = presets.add(&crate::presets::PresetDraft { name: "周报".into(), prompt: "整理成周报".into() }, 1).unwrap();
    assert_eq!(source.current().presets.iter().map(|p| p.id).collect::<Vec<_>>(), [id], "the new preset is read");
    std::fs::write(dir.path().join("presets.json"), b"{broken").unwrap();
    assert_eq!(source.current().presets.iter().map(|p| p.id).collect::<Vec<_>>(), [id], "the last usable list stays");
    assert_eq!(std::fs::read(dir.path().join("presets.json")).unwrap(), b"{broken", "the broken file stays where it is");
    std::fs::write(dir.path().join("settings.json"), b"{broken").unwrap();
    assert!(source.current().settings.engines.refine_enabled, "the last usable settings stay");
    let ok = Settings { engines: crate::engines::EngineSettings { refine_enabled: false, ..Default::default() }, ..Settings::default() };
    std::fs::write(dir.path().join("settings.json"), serde_json::to_vec(&ok).unwrap()).unwrap();
    assert!(!source.current().settings.engines.refine_enabled, "a file that is usable again is read");
}

#[test]
fn an_unusable_settings_file_stops_the_server_and_a_broken_list_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("rules.json"), b"[").unwrap();
    let (source, notices) = file_source(dir.path(), Arc::new(FakeTranscriber::ok("x")));
    assert!(notices.iter().any(|n| n.contains("rules.json 无法使用")), "{notices:?}");
    assert!(source.current().vocabulary.glossary().is_empty());
    std::fs::write(dir.path().join("settings.json"), b"{").unwrap();
    let factory: crate::dictation::EngineFactory = Arc::new(|_| (Arc::new(FakeTranscriber::ok("x")) as Arc<dyn Transcriber>, None));
    let err = FileSource::open(FileSourceConfig {
        data_dir: dir.path().to_path_buf(),
        platform: Platform::Linux,
        overrides: EngineOverrides::default(),
        secrets: UserSecrets::default(),
        built_in: BUILT_IN,
        models: None,
        factory,
        segmenter: None,
    })
    .unwrap_err();
    assert!(err.contains("settings.json 无法使用"), "{err}");
    assert_eq!(std::fs::read(dir.path().join("settings.json")).unwrap(), b"{", "not moved aside");
}

#[test]
fn the_command_line_overrides_the_engines() {
    let mut engines = crate::engines::EngineSettings::default();
    EngineOverrides { local_model: Some("sense-voice-small".into()), threads: Some(4), ..EngineOverrides::default() }.apply(&mut engines);
    assert_eq!(
        (engines.asr_provider, engines.local_model.as_deref(), engines.local_threads),
        (crate::providers::ProviderId::Local, Some("sense-voice-small"), Some(4))
    );
    EngineOverrides { asr: Some(crate::providers::ProviderId::Aliyun), local_model: Some("x".into()), ..EngineOverrides::default() }.apply(&mut engines);
    assert_eq!(engines.asr_provider, crate::providers::ProviderId::Aliyun, "--asr wins over the implied local");
}

// ---- the token ------------------------------------------------------------------------------

#[test]
fn the_token_is_created_once_private_kept_and_replaced_on_request() {
    let dir = tempfile::tempdir().unwrap();
    let path = default_token_path(dir.path());
    let (token, warning) = load_or_create_token(&path).unwrap();
    assert_eq!((token.len(), warning), (64, None), "32 bytes as hexadecimal");
    assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(load_or_create_token(&path).unwrap().0, token, "kept across starts");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(path.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(load_or_create_token(&path).unwrap().1.unwrap().contains("chmod 600"));
    }
    let rotated = rotate_token(&path).unwrap();
    assert_ne!(rotated, token);
    assert_eq!(load_or_create_token(&path).unwrap().0, rotated);
    std::fs::write(&path, " \n").unwrap();
    assert!(load_or_create_token(&path).unwrap_err().contains("为空"));
}
