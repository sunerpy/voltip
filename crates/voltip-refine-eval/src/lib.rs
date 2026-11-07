//! The AI polish prompt evaluation (docs/refine-eval.md): what each sample's clean-up sends, and
//! whether the prompts are still the ones the last evaluation measured.
//!
//! * [`SampleSet`] — the evaluation's samples ([`SAMPLES_PATH`]): synthetic dictated texts, each for
//!   one [`Unit`] (a built-in preset, or a built-in scene that has an instruction), with the points
//!   the answer must keep. Only what the request is made of is read here; the points and the
//!   judging are the runner's (`scripts/refine-eval/refine_eval.py`).
//! * [`hints`] — the hints the core gives the clean-up of the sample's take with the default
//!   settings (docs/dictation.md §18.5, §18.10): the scene's preset, instruction and term pack, the
//!   dictionary's terms first, the application's name.
//! * [`request`] — the body the app posts for it to the built-in service
//!   ([`voltip_refine::refine_request`]): system prompt, text, temperature, `max_tokens`, effort.
//! * [`lengths`] — each preset's text and scene instruction against the app's limit.
//! * [`digests`] — one SHA-256 per unit over its samples' requests, the model left out; the
//!   [`Record`] of the last evaluation ([`RECORD_PATH`]) holds the ones it measured, and a test fails
//!   with [`stale_message`] when a prompt (or a sample) has changed since.
//!
//! The evaluation itself (Bedrock, the judge, the report) runs by hand: `make refine-eval`, never
//! in CI.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use voltip_core::dictation::{RefineContext, RefineHints};
use voltip_core::vocabulary::packs;
use voltip_core::{BuiltinPreset, BuiltinScene, ContextSharing, DictionaryEntry, EntrySource, PresetId, RemoteService, TakePreset, Vocabulary};
use voltip_net::Network;

/// The samples, from the repository's root.
pub const SAMPLES_PATH: &str = "scripts/refine-eval/samples.json";
/// The last evaluation's record, from the repository's root.
pub const RECORD_PATH: &str = "docs/refine-eval/prompt-digests.json";
/// The command that evaluates the prompts again and writes the record.
pub const RERUN_COMMAND: &str = "make refine-eval";
/// The built-in clean-up's model as the app names it: the evaluation builds the request for it and
/// then sends it to the same model on Bedrock (`openai.gpt-oss-120b-1:0`).
pub const PRIMARY_MODEL: &str = "openai/gpt-oss-120b";
/// The built-in clean-up's fallback model as the app names it (`openai.gpt-oss-20b-1:0` on Bedrock).
pub const FALLBACK_MODEL: &str = "openai/gpt-oss-20b";
/// Where the request would go: never contacted, only the configuration reads it.
const SERVICE_URL: &str = "https://builtin.invalid/v1";

/// What the evaluation measures: the prompt of one built-in preset, or of one built-in scene with
/// an instruction (即时聊天 has none: its takes are the 口语聊天 preset's).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize)]
#[serde(try_from = "String")]
pub enum Unit {
    /// A built-in preset, as the engines' preset (no scene).
    Preset(BuiltinPreset),
    /// A built-in scene with its default preset, instruction and term pack.
    Scene(BuiltinScene),
}

impl Unit {
    /// Every unit, in the report's order: the presets as the interface lists them, then the scenes
    /// that have an instruction.
    pub fn all() -> Vec<Self> {
        let presets = BuiltinPreset::ALL.into_iter().map(Self::Preset);
        let scenes = BuiltinScene::ALL.into_iter().filter(|scene| scene.instruction().is_some()).map(Self::Scene);
        presets.chain(scenes).collect()
    }

    /// `preset:proofread`, `scene:coding`, …
    pub fn name(self) -> String {
        match self {
            Self::Preset(preset) => format!("preset:{}", preset.as_str()),
            Self::Scene(scene) => format!("scene:{}", scene.as_str()),
        }
    }
}

impl fmt::Display for Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name())
    }
}

impl FromStr for Unit {
    type Err = Error;

    fn from_str(name: &str) -> Result<Self, Error> {
        Self::all().into_iter().find(|unit| unit.name() == name).ok_or_else(|| Error::Samples(format!("unknown unit {name:?}")))
    }
}

impl TryFrom<String> for Unit {
    type Error = Error;

    fn try_from(name: String) -> Result<Self, Error> {
        name.parse()
    }
}

/// One dictated text and the settings of its take.
#[derive(Clone, Debug, Deserialize)]
pub struct Sample {
    /// Unique id (`prompt-03`).
    pub id: String,
    /// What it measures.
    pub unit: Unit,
    /// The recogniser's text, after the dictionary's corrections: what the clean-up receives.
    pub text: String,
    /// The engines' language setting (`zh`, `en`); `None`: automatic, the default.
    #[serde(default)]
    pub language: Option<String>,
    /// The display name of the application in front (shared by default).
    #[serde(default)]
    pub app: Option<String>,
    /// Its window title: only sent when the user shares window titles (off by default).
    #[serde(default)]
    pub window_title: Option<String>,
    /// The user's enabled dictionary terms, in order.
    #[serde(default)]
    pub dictionary: Vec<String>,
    /// The points the answer must keep; the runner reads them, here they only must exist.
    #[serde(default)]
    pub points: Vec<Value>,
}

/// The samples file: the samples, and the runner's own fields.
#[derive(Clone, Debug, Deserialize)]
pub struct SampleSet {
    /// Every sample, in the file's order.
    pub samples: Vec<Sample>,
}

impl SampleSet {
    /// Parse the samples file's text and check it: unique ids, no blank text, a point for each.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let set: Self = serde_json::from_str(text).map_err(|e| Error::Samples(e.to_string()))?;
        let mut ids = BTreeSet::new();
        for sample in &set.samples {
            if !ids.insert(sample.id.as_str()) {
                return Err(Error::Samples(format!("sample id {:?} appears twice", sample.id)));
            }
            if sample.text.trim().is_empty() {
                return Err(Error::Samples(format!("sample {:?} has no text", sample.id)));
            }
            if sample.points.is_empty() {
                return Err(Error::Samples(format!("sample {:?} lists no point to keep", sample.id)));
            }
        }
        Ok(set)
    }

    /// Read and parse the samples file at `path`.
    pub fn load(path: &Path) -> Result<Self, Error> {
        Self::parse(&std::fs::read_to_string(path).map_err(|e| Error::Io(format!("{}: {e}", path.display())))?)
    }
}

/// Why the samples or the record cannot be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// A file could not be read.
    Io(String),
    /// The samples file is not acceptable.
    Samples(String),
    /// The record is not acceptable.
    Record(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(message) => write!(f, "cannot read {message}"),
            Self::Samples(message) => write!(f, "samples: {message}"),
            Self::Record(message) => write!(f, "record: {message}"),
        }
    }
}

impl std::error::Error for Error {}

/// The hints the core gives the clean-up of `sample`'s take (`DictationEngine::refine_hints`) with
/// the default settings: the application's name shared, its window title only when the sample
/// names one (a user who shares titles), the engines' language the sample's. A scene takes the
/// built-in template's preset and instruction, and its term pack after the dictionary's terms
/// (§18.10); a preset is the engines' preset, without a scene.
pub fn hints(sample: &Sample) -> RefineHints {
    let entries: Vec<DictionaryEntry> = sample
        .dictionary
        .iter()
        .map(|term| DictionaryEntry {
            id: Uuid::nil(),
            term: term.clone(),
            heard_as: Vec::new(),
            enabled: true,
            source: EntrySource::Manual,
            created_at_ms: 0,
            updated_at_ms: 0,
        })
        .collect();
    let vocabulary = Vocabulary::compile(&entries, &[]);
    let (preset, instruction, vocabulary, language) = match sample.unit {
        Unit::Preset(preset) => (preset, None, vocabulary, sample.language.clone()),
        Unit::Scene(scene) => {
            let overrides = scene.template(voltip_protocol::Platform::Windows).overrides;
            let preset = match overrides.refine_preset {
                Some(PresetId::Builtin(preset)) => preset,
                _ => BuiltinPreset::Proofread,
            };
            let language = overrides.language.or_else(|| sample.language.clone());
            (preset, overrides.prompt, vocabulary.with_terms(packs::terms(scene)), language)
        }
    };
    let sharing = ContextSharing::default();
    RefineHints {
        glossary: vocabulary.glossary().to_vec(),
        language,
        preset: TakePreset::Builtin(preset),
        context: RefineContext { app_name: sample.app.clone().filter(|_| sharing.app_name), window_title: sample.window_title.clone(), instruction },
        origin: None,
    }
}

/// Builds the requests the app would post for the samples.
pub struct Requests {
    network: Arc<Network>,
}

impl Default for Requests {
    fn default() -> Self {
        Self::new()
    }
}

impl Requests {
    /// One configuration for every request (the network is never used: nothing is sent).
    pub fn new() -> Self {
        Self { network: Network::new() }
    }

    /// The body the app posts for `sample` to the built-in service running `model` (the app's name
    /// for it, [`PRIMARY_MODEL`] or [`FALLBACK_MODEL`]): `voltip_cloud::refine_config` for the
    /// built-in service, the hints as `voltip_cloud::prompt_hints` hands them over.
    pub fn body(&self, sample: &Sample, model: &str) -> Result<Value, Error> {
        let remote = RemoteService { url: SERVICE_URL.to_owned(), model: model.to_owned(), ..RemoteService::default() };
        let config = voltip_cloud::refine_config(&remote, true, &self.network);
        let hints = hints(sample);
        voltip_refine::refine_request(&config, &sample.text, &voltip_cloud::prompt_hints(&hints))
            .ok_or_else(|| Error::Samples(format!("sample {:?} would send nothing", sample.id)))
    }
}

/// A unit's own text against the limit the app holds it to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Length {
    /// Which unit.
    pub unit: Unit,
    /// A preset's text (task, rules, examples: what 复制为自定义 copies) or a scene's instruction, in
    /// characters.
    pub chars: usize,
    /// `MAX_PRESET_PROMPT_CHARS` for a preset (a copy must still be a valid custom preset),
    /// `MAX_SCENE_PROMPT_CHARS` for a scene's instruction.
    pub limit: usize,
}

/// Every unit's [`Length`], in [`Unit::all`]'s order.
pub fn lengths() -> Vec<Length> {
    Unit::all()
        .into_iter()
        .map(|unit| match unit {
            Unit::Preset(preset) => {
                Length { unit, chars: voltip_cloud::builtin_preset_body(preset).chars().count(), limit: voltip_core::presets::MAX_PRESET_PROMPT_CHARS }
            }
            Unit::Scene(scene) => {
                Length { unit, chars: scene.instruction().unwrap_or_default().chars().count(), limit: voltip_core::scenes::MAX_SCENE_PROMPT_CHARS }
            }
        })
        .collect()
}

/// `value` as JSON with every object's keys sorted: the same bytes whichever map type serde_json
/// was built with.
pub fn canonical_json(value: &Value) -> String {
    fn write(value: &Value, out: &mut String) {
        match value {
            Value::Object(map) => {
                let sorted: BTreeMap<&String, &Value> = map.iter().collect();
                out.push('{');
                for (i, (key, item)) in sorted.into_iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&Value::String(key.clone()).to_string());
                    out.push(':');
                    write(item, out);
                }
                out.push('}');
            }
            Value::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write(item, out);
                }
                out.push(']');
            }
            scalar => out.push_str(&scalar.to_string()),
        }
    }
    let mut out = String::new();
    write(value, &mut out);
    out
}

/// One SHA-256 per unit that has samples: over each of its samples, in id order, the id and the
/// request's [`canonical_json`] without its `model` (the evaluation sends the same request to the
/// primary and the fallback model). A change to a preset, a scene's instruction, a shared clause, a
/// term pack, the request's parameters or a sample changes it.
pub fn digests(samples: &[Sample]) -> Result<BTreeMap<String, String>, Error> {
    let requests = Requests::new();
    let mut by_unit: BTreeMap<String, Vec<&Sample>> = BTreeMap::new();
    for sample in samples {
        by_unit.entry(sample.unit.name()).or_default().push(sample);
    }
    let mut out = BTreeMap::new();
    for (unit, mut members) in by_unit {
        members.sort_by(|a, b| a.id.cmp(&b.id));
        let mut hasher = Sha256::new();
        for sample in members {
            let mut body = requests.body(sample, PRIMARY_MODEL)?;
            if let Some(map) = body.as_object_mut() {
                map.remove("model");
            }
            hasher.update(sample.id.as_bytes());
            hasher.update(b"\n");
            hasher.update(canonical_json(&body).as_bytes());
            hasher.update(b"\n");
        }
        out.insert(unit, hex::encode(hasher.finalize()));
    }
    Ok(out)
}

/// The last evaluation's record ([`RECORD_PATH`]): when it ran, on which models, its report, and
/// the digest of every unit it measured.
#[derive(Clone, Debug, Deserialize)]
pub struct Record {
    /// The day it ran (`2026-11-07`).
    pub evaluated_at: String,
    /// The report, from the repository's root.
    pub report: String,
    /// Unit name → [`digests`] when it ran.
    pub units: BTreeMap<String, String>,
}

impl Record {
    /// Parse the record's text.
    pub fn parse(text: &str) -> Result<Self, Error> {
        serde_json::from_str(text).map_err(|e| Error::Record(e.to_string()))
    }

    /// Read and parse the record at `path`.
    pub fn load(path: &Path) -> Result<Self, Error> {
        Self::parse(&std::fs::read_to_string(path).map_err(|e| Error::Io(format!("{}: {e}", path.display())))?)
    }

    /// The units whose prompts are not the ones measured: a different digest, or a unit on one side
    /// only, in name order.
    pub fn stale(&self, current: &BTreeMap<String, String>) -> Vec<String> {
        let names: BTreeSet<&String> = self.units.keys().chain(current.keys()).collect();
        names.into_iter().filter(|name| self.units.get(*name) != current.get(*name)).cloned().collect()
    }
}

/// What a failing check says: which units changed since the record, and the command that measures
/// them again and updates the report and the record.
pub fn stale_message(stale: &[String], record: &Record) -> String {
    format!(
        "AI 润色的提示词（或评测样例）在最近一次评测（{date}，报告 {report}）之后改过：{units}。\n\
         请重新运行评测、更新报告和记录：{RERUN_COMMAND}（说明见 docs/refine-eval.md）。\n\
         The polish prompts (or the evaluation samples) changed after the last evaluation ({date}, {report}): {units}. \
         Run the evaluation again and commit the new report and record: {RERUN_COMMAND} (see docs/refine-eval.md).",
        date = record.evaluated_at,
        report = record.report,
        units = stale.join(", "),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(json: Value) -> Sample {
        serde_json::from_value(json).unwrap()
    }

    fn system(body: &Value) -> &str {
        body["messages"][0]["content"].as_str().unwrap()
    }

    #[test]
    fn units_are_every_preset_and_every_scene_with_an_instruction() {
        let names: Vec<String> = Unit::all().into_iter().map(Unit::name).collect();
        assert_eq!(names.len(), 14, "{names:?}");
        assert_eq!(names[0], "preset:proofread");
        assert!(names.contains(&"scene:coding".to_owned()) && names.contains(&"scene:academic".to_owned()));
        assert!(!names.contains(&"scene:chat".to_owned()), "即时聊天 has no instruction");
        for unit in Unit::all() {
            assert_eq!(unit.name().parse::<Unit>(), Ok(unit));
            assert_eq!(unit.to_string(), unit.name());
        }
        assert!(matches!("scene:chat".parse::<Unit>(), Err(Error::Samples(_))));
    }

    #[test]
    fn a_scene_sample_takes_the_template_its_pack_after_the_dictionary_and_the_app() {
        let s = sample(serde_json::json!({
            "id": "coding-x", "unit": "scene:coding", "text": "把 docker 镜像推上去", "app": "Visual Studio Code",
            "dictionary": ["Voltip", "docker"], "points": [{"id": "p1"}]
        }));
        let hints = hints(&s);
        assert_eq!(hints.preset, TakePreset::Builtin(BuiltinPreset::Proofread));
        assert_eq!(hints.context.instruction.as_deref(), BuiltinScene::Coding.instruction());
        assert_eq!(hints.context.app_name.as_deref(), Some("Visual Studio Code"));
        assert_eq!(hints.context.window_title, None);
        assert_eq!(&hints.glossary[..3], ["Voltip", "docker", "API"], "the dictionary first, then the pack");
        assert!(!hints.glossary.iter().any(|t| t == "Docker"), "a term already there is not repeated");
        assert_eq!(hints.language, None, "automatic, the default");
        let office = sample(serde_json::json!({"id": "o", "unit": "scene:office", "text": "x", "language": "zh", "points": [{}]}));
        let hints = super::hints(&office);
        assert_eq!(hints.preset, TakePreset::Builtin(BuiltinPreset::Formal));
        assert_eq!(hints.language.as_deref(), Some("zh"), "the scene sets none: the engines' language");
        assert_eq!(&hints.glossary[..], packs::terms(BuiltinScene::Office));
    }

    #[test]
    fn a_preset_sample_has_no_scene_no_pack() {
        let s = sample(serde_json::json!({
            "id": "t", "unit": "preset:translate", "text": "明天见", "language": "zh", "window_title": "周报.docx", "points": [{}]
        }));
        let hints = hints(&s);
        assert_eq!(hints.preset, TakePreset::Builtin(BuiltinPreset::Translate));
        assert_eq!((hints.context.instruction, hints.glossary.len()), (None, 0));
        assert_eq!(hints.context.window_title.as_deref(), Some("周报.docx"));
    }

    #[test]
    fn the_body_is_the_built_in_request() {
        let requests = Requests::new();
        let s = sample(serde_json::json!({
            "id": "c", "unit": "scene:legal", "text": " 依据民法典第五百条 ", "app": "WPS Office", "points": [{}]
        }));
        let body = requests.body(&s, PRIMARY_MODEL).unwrap();
        assert_eq!(body["model"], PRIMARY_MODEL);
        assert_eq!(body["temperature"], 0.2);
        assert_eq!(body["reasoning_effort"], "low");
        assert_eq!(body["max_tokens"], 128 + 256, "the shortest answer's budget and gpt-oss's room to reason");
        assert_eq!(body["messages"][1]["content"], "依据民法典第五百条", "the text as sent, trimmed");
        let system = system(&body);
        assert!(system.starts_with(voltip_refine::Preset::Proofread.prompt().as_str()), "{system}");
        assert!(system.contains(BuiltinScene::Legal.instruction().unwrap()));
        assert!(system.contains("\n当前应用：WPS Office"));
        assert!(system.contains(voltip_refine::GLOSSARY_CLAUSE));
        let fallback = requests.body(&s, FALLBACK_MODEL).unwrap();
        assert_eq!(fallback["model"], FALLBACK_MODEL);
        assert_eq!(fallback["max_tokens"], body["max_tokens"], "both gpt-oss models get the same request");
        let blank = sample(serde_json::json!({"id": "b", "unit": "preset:chat", "text": " \n ", "points": [{}]}));
        assert!(matches!(requests.body(&blank, PRIMARY_MODEL), Err(Error::Samples(_))));
    }

    #[test]
    fn every_prompt_is_within_the_limit_the_app_holds_it_to() {
        let lengths = lengths();
        assert_eq!(lengths.len(), Unit::all().len());
        for length in lengths {
            assert!(length.chars > 0 && length.chars <= length.limit, "{length:?}");
        }
    }

    #[test]
    fn canonical_json_sorts_keys_at_every_level() {
        let value = serde_json::json!({"b": 1, "a": [{"d": 0.2, "c": "引号\"和\n换行"}], "e": null});
        assert_eq!(canonical_json(&value), r#"{"a":[{"c":"引号\"和\n换行","d":0.2}],"b":1,"e":null}"#);
    }

    #[test]
    fn a_digest_follows_the_prompt_and_the_samples_but_not_the_model() {
        let base = vec![
            sample(serde_json::json!({"id": "p-1", "unit": "preset:prompt", "text": "帮我写个脚本", "points": [{}]})),
            sample(serde_json::json!({"id": "p-2", "unit": "preset:prompt", "text": "把超时改成六十秒", "points": [{}]})),
            sample(serde_json::json!({"id": "n-1", "unit": "preset:notes", "text": "今天讨论了发布", "points": [{}]})),
        ];
        let first = digests(&base).unwrap();
        assert_eq!(first.keys().collect::<Vec<_>>(), ["preset:notes", "preset:prompt"]);
        assert!(first.values().all(|d| d.len() == 64));
        let mut reordered = base.clone();
        reordered.reverse();
        assert_eq!(digests(&reordered).unwrap(), first, "the file's order does not matter");
        let mut edited = base.clone();
        edited[1].text = "把超时改成九十秒".into();
        let second = digests(&edited).unwrap();
        assert_ne!(second["preset:prompt"], first["preset:prompt"]);
        assert_eq!(second["preset:notes"], first["preset:notes"], "only the unit whose request changed");
        let mut hinted = base.clone();
        hinted[2].language = Some("zh".into());
        assert_ne!(digests(&hinted).unwrap()["preset:notes"], first["preset:notes"], "the language line is part of the prompt");
    }

    #[test]
    fn the_record_names_the_changed_units_and_the_command() {
        let record = Record::parse(
            r#"{"evaluated_at": "2026-11-07", "report": "docs/refine-eval/report.md", "models": ["m"], "units": {"preset:chat": "aa", "preset:notes": "bb", "scene:old": "cc"}}"#,
        )
        .unwrap();
        let current: BTreeMap<String, String> =
            [("preset:chat", "aa"), ("preset:notes", "b2"), ("preset:prompt", "dd")].into_iter().map(|(k, v)| (k.to_owned(), v.to_owned())).collect();
        let stale = record.stale(&current);
        assert_eq!(stale, ["preset:notes", "preset:prompt", "scene:old"]);
        let message = stale_message(&stale, &record);
        assert!(message.contains("preset:notes, preset:prompt, scene:old"), "{message}");
        assert!(message.contains(RERUN_COMMAND) && message.contains("docs/refine-eval/report.md") && message.contains("2026-11-07"), "{message}");
        assert!(record.stale(&record.units).is_empty());
        assert!(matches!(Record::parse("{}"), Err(Error::Record(_))));
    }

    #[test]
    fn a_samples_file_needs_unique_ids_text_and_points() {
        let ok = r#"{"units": {}, "samples": [{"id": "a", "unit": "preset:chat", "text": "好", "points": [{"id": "p1"}]}]}"#;
        assert_eq!(SampleSet::parse(ok).unwrap().samples.len(), 1);
        for (bad, why) in [
            (
                r#"{"samples": [{"id": "a", "unit": "preset:chat", "text": "好", "points": [{}]}, {"id": "a", "unit": "preset:chat", "text": "好", "points": [{}]}]}"#,
                "twice",
            ),
            (r#"{"samples": [{"id": "a", "unit": "preset:chat", "text": "  ", "points": [{}]}]}"#, "no text"),
            (r#"{"samples": [{"id": "a", "unit": "preset:chat", "text": "好"}]}"#, "no point"),
            (r#"{"samples": [{"id": "a", "unit": "scene:chat", "text": "好", "points": [{}]}]}"#, "unknown unit"),
        ] {
            let err = SampleSet::parse(bad).unwrap_err();
            assert!(err.to_string().contains(why), "{why}: {err}");
        }
        assert!(matches!(SampleSet::load(Path::new("/nonexistent/samples.json")), Err(Error::Io(_))));
        assert!(matches!(Record::load(Path::new("/nonexistent/record.json")), Err(Error::Io(_))));
    }
}
