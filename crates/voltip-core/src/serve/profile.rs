//! How a request of the local speech service is processed (docs/dictation.md §23.3): the `model`
//! field's grammar, the defaults a host starts with, and the resolution of both against the
//! settings, presets and scenes into the [`Recipe`] a take runs with — the same overrides a take of
//! the app gets from its scene (§18) and preset (§21).

use std::sync::Arc;

use uuid::Uuid;

use crate::dictation::ports::{ForegroundApp, RefineContext, RefineHints};
use crate::engines::ChineseScript;
use crate::presets::{BuiltinPreset, CustomPreset, PresetId, PresetRef, TakePreset, resolve};
use crate::scenes::{LANGUAGE_AUTO, Scene, SceneRef, clean_language, match_scene, normalize_app_id};
use crate::settings::Settings;
use crate::vocabulary::Vocabulary;

/// The model name of the default processing.
pub const MODEL_DEFAULT: &str = "voltip";
/// What starts a request's own choice.
pub const MODEL_PREFIX: &str = "voltip:";
/// The item that switches the clean-up off.
pub const MODEL_RAW: &str = "raw";

/// Why a request or a host's defaults cannot be processed; the text is what the caller is told.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ProfileError(pub String);

fn invalid(message: impl Into<String>) -> ProfileError {
    ProfileError(message.into())
}

/// What a request asks for (`model`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Profile {
    /// `voltip`, no model, or any name that is not `voltip:…` (`whisper-1`): the host's defaults.
    Default,
    /// `voltip:…`: the request's own choice. Nothing of the defaults is inherited (§23.3).
    Request {
        /// `raw`: no clean-up.
        raw: bool,
        /// `scene=…`.
        scene: Option<String>,
        /// `preset=…`.
        preset: Option<String>,
    },
}

/// Parse a request's `model`: see [`Profile`]. Items are separated by `;`; each of `raw`,
/// `scene=` and `preset=` at most once, and `raw` never with `preset`.
pub fn parse_model(model: Option<&str>) -> Result<Profile, ProfileError> {
    let Some(model) = model.map(str::trim).filter(|m| !m.is_empty()) else { return Ok(Profile::Default) };
    let Some(items) = model.strip_prefix(MODEL_PREFIX) else { return Ok(Profile::Default) };
    let (mut raw, mut scene, mut preset) = (false, None, None);
    for item in items.split(';').map(str::trim).filter(|i| !i.is_empty()) {
        let duplicate = || invalid(format!("model「{model}」中「{}」重复出现", item.split('=').next().unwrap_or(item)));
        if item == MODEL_RAW {
            if raw {
                return Err(duplicate());
            }
            raw = true;
            continue;
        }
        let Some((key, value)) = item.split_once('=') else {
            return Err(invalid(format!("model「{model}」中的「{item}」无法识别；可用 raw、scene=…、preset=…")));
        };
        let value = value.trim();
        if value.is_empty() {
            return Err(invalid(format!("model「{model}」中「{}=」后缺少名称", key.trim())));
        }
        let slot = match key.trim() {
            "scene" => &mut scene,
            "preset" => &mut preset,
            other => return Err(invalid(format!("model「{model}」中的「{other}」无法识别；可用 raw、scene=…、preset=…"))),
        };
        if slot.is_some() {
            return Err(duplicate());
        }
        *slot = Some(value.to_owned());
    }
    if raw && preset.is_some() {
        return Err(invalid(format!("model「{model}」不能同时指定 raw 和 preset")));
    }
    Ok(Profile::Request { raw, scene, preset })
}

/// The preset `selector` names: a built-in's wire name, a custom preset's UUID, a custom preset's
/// name (ASCII case ignored), or a built-in's Chinese or English name.
pub fn find_preset(selector: &str, presets: &[CustomPreset]) -> Option<PresetId> {
    let selector = selector.trim();
    if let Some(builtin) = BuiltinPreset::ALL.into_iter().find(|p| p.as_str() == selector) {
        return Some(PresetId::Builtin(builtin));
    }
    if let Ok(id) = Uuid::parse_str(selector) {
        return presets.iter().any(|p| p.id == id).then_some(PresetId::Custom(id));
    }
    if let Some(custom) = presets.iter().find(|p| p.name.eq_ignore_ascii_case(selector)) {
        return Some(PresetId::Custom(custom.id));
    }
    BuiltinPreset::ALL.into_iter().find(|p| p.display_name() == selector || p.english_name().eq_ignore_ascii_case(selector)).map(PresetId::Builtin)
}

/// The scene `selector` names in `scenes`: a built-in category's wire name, a scene's UUID, a
/// user scene's name (ASCII case ignored), or a built-in category's Chinese or English name.
pub fn find_scene<'a>(selector: &str, scenes: &'a [Scene]) -> Option<&'a Scene> {
    let selector = selector.trim();
    if let Some(scene) = scenes.iter().find(|s| s.builtin.is_some_and(|c| c.as_str() == selector)) {
        return Some(scene);
    }
    if let Ok(id) = Uuid::parse_str(selector) {
        return scenes.iter().find(|s| s.id == id);
    }
    if let Some(scene) = scenes.iter().find(|s| s.builtin.is_none() && s.name.eq_ignore_ascii_case(selector)) {
        return Some(scene);
    }
    scenes.iter().find(|s| s.builtin.is_some_and(|c| c.display_name() == selector || c.english_name().eq_ignore_ascii_case(selector)))
}

/// The defaults a host starts with (§23.3): the processing every `voltip` request gets, and the
/// language and script every request gets. The scene and preset are kept as ids, resolved once when
/// the host starts; one that is gone later behaves as the app's pinned scene and preset do.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Defaults {
    /// `--scene` / the app's service scene.
    pub scene: Option<Uuid>,
    /// `--app` (normalised): matched like the application in front when no scene is set.
    pub app: Option<String>,
    /// `--preset` / the app's service preset.
    pub preset: Option<PresetId>,
    /// `--refine on|off`.
    pub refine: Option<bool>,
    /// `--language`: forced for every request; `auto` = no hint.
    pub language: Option<String>,
    /// `--script`.
    pub script: Option<ChineseScript>,
}

/// What a host was started with, before it was checked against the lists.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DefaultChoices {
    /// `--scene`.
    pub scene: Option<String>,
    /// `--app`.
    pub app: Option<String>,
    /// `--preset`.
    pub preset: Option<String>,
    /// `--refine`.
    pub refine: Option<bool>,
    /// `--language`.
    pub language: Option<String>,
    /// `--script`.
    pub script: Option<ChineseScript>,
}

impl DefaultChoices {
    /// Resolve the choices against the lists the host starts with: a scene or preset that is not
    /// there, or a language code that is not one, is an error the host stops with.
    pub fn resolve(&self, presets: &[CustomPreset], scenes: &[Scene]) -> Result<Defaults, ProfileError> {
        let scene = match &self.scene {
            None => None,
            Some(selector) => Some(find_scene(selector, scenes).ok_or_else(|| invalid(format!("没有名为「{selector}」的场景")))?.id),
        };
        let preset = match &self.preset {
            None => None,
            Some(selector) => Some(find_preset(selector, presets).ok_or_else(|| invalid(format!("没有名为「{selector}」的预设")))?),
        };
        let app = match self.app.as_deref().map(normalize_app_id) {
            Some(id) if id.is_empty() => return Err(invalid("应用 id 不能为空")),
            other => other,
        };
        let language = match &self.language {
            None => None,
            Some(raw) => clean_language(raw).map_err(ProfileError)?,
        };
        Ok(Defaults { scene, app, preset, refine: self.refine, language, script: self.script })
    }
}

/// The lists a request is resolved against: one snapshot of the settings, presets, scenes and the
/// compiled vocabulary.
#[derive(Clone, Copy, Debug)]
pub struct Catalog<'a> {
    /// `settings.json`.
    pub settings: &'a Settings,
    /// `presets.json`.
    pub presets: &'a [CustomPreset],
    /// `scenes.json` (with the built-in scenes).
    pub scenes: &'a [Scene],
    /// The dictionary and the rules.
    pub vocabulary: &'a Arc<Vocabulary>,
}

/// How one request is processed: what a take of the app would get from its scene and preset.
#[derive(Clone, Debug)]
pub struct Recipe {
    /// The language hint (`None` = auto-detect).
    pub language: Option<String>,
    /// The script the recogniser's text is brought to.
    pub script: ChineseScript,
    /// Whether the clean-up runs.
    pub refine: bool,
    /// What the clean-up does.
    pub preset: TakePreset,
    /// The scene applied, as the response names it.
    pub scene: Option<SceneRef>,
    /// The dictionary and rules, with a built-in scene's term pack.
    pub vocabulary: Arc<Vocabulary>,
    /// What the refiner is told besides the text.
    pub hints: RefineHints,
}

impl Recipe {
    /// The preset as the response names it (only when the clean-up was asked for).
    pub fn preset_ref(&self) -> Option<PresetRef> {
        self.refine.then(|| self.preset.to_ref())
    }
}

/// A language hint: `auto` is none, anything else the code.
fn hint(code: &str) -> Option<String> {
    (code != LANGUAGE_AUTO).then(|| code.to_owned())
}

/// Resolve one request (`profile`, its `language` field) under `defaults` against `catalog`
/// (§23.3). A request's own scene or preset that is not there is an error; a default one that went
/// away since the host started is not (no scene; a gone custom preset is 校对).
pub fn resolve_recipe(defaults: &Defaults, profile: &Profile, request_language: Option<&str>, catalog: Catalog<'_>) -> Result<Recipe, ProfileError> {
    let engines = &catalog.settings.engines;
    let (scene, app, preset, refine): (Option<&Scene>, Option<&str>, Option<PresetId>, Option<bool>) = match profile {
        Profile::Default => {
            let scene = match defaults.scene {
                Some(id) => {
                    let found = catalog.scenes.iter().find(|s| s.id == id);
                    if found.is_none() {
                        tracing::warn!(scene = %id, "the service's scene no longer exists; requests run without a scene");
                    }
                    found
                }
                None => defaults.app.as_deref().and_then(|app| {
                    let probe = ForegroundApp { app_id: app.to_owned(), name: app.to_owned(), title: None, window: None };
                    match_scene(catalog.scenes, &probe)
                }),
            };
            (scene, defaults.app.as_deref(), defaults.preset, defaults.refine)
        }
        Profile::Request { raw, scene, preset } => {
            let scene = match scene {
                Some(selector) => {
                    Some(find_scene(selector, catalog.scenes).ok_or_else(|| invalid(format!("没有名为「{selector}」的场景（可选值见 GET /v1/models）")))?)
                }
                None => None,
            };
            let preset = match preset {
                Some(selector) => {
                    Some(find_preset(selector, catalog.presets).ok_or_else(|| invalid(format!("没有名为「{selector}」的预设（可选值见 GET /v1/models）")))?)
                }
                None => None,
            };
            (scene, None, preset, raw.then_some(false))
        }
    };
    let overrides = scene.map(|s| &s.overrides);
    let refine = refine.or(overrides.and_then(|o| o.refine_enabled)).unwrap_or(engines.refine_enabled);
    let preset_id = preset.or(overrides.and_then(|o| o.refine_preset)).unwrap_or(engines.refine_preset);
    let (preset, missing) = resolve(preset_id, catalog.presets);
    if missing {
        tracing::info!("the service's custom preset no longer exists; refining with 校对");
    }
    let request_language = match request_language {
        None => None,
        Some(raw) => clean_language(raw).map_err(ProfileError)?,
    };
    let language = match (&defaults.language, overrides.and_then(|o| o.language.as_deref()), request_language) {
        (Some(forced), _, _) => hint(forced),
        (None, Some(scene_language), _) => hint(scene_language),
        (None, None, Some(asked)) => hint(&asked),
        (None, None, None) => engines.language.clone(),
    };
    let script = defaults.script.or(overrides.and_then(|o| o.chinese_script)).unwrap_or(engines.chinese_script);
    let vocabulary = match scene.and_then(|s| s.builtin) {
        Some(category) => {
            let pack = crate::vocabulary::packs::terms(category);
            if pack.is_empty() { catalog.vocabulary.clone() } else { Arc::new(catalog.vocabulary.with_terms(pack)) }
        }
        None => catalog.vocabulary.clone(),
    };
    let hints = RefineHints {
        glossary: vocabulary.glossary().to_vec(),
        language: language.clone(),
        preset: preset.clone(),
        context: RefineContext {
            app_name: app.filter(|_| catalog.settings.context_sharing.app_name).map(str::to_owned),
            window_title: None,
            instruction: overrides.and_then(|o| o.prompt.clone()),
        },
    };
    Ok(Recipe { language, script, refine, preset, scene: scene.map(Scene::to_ref), vocabulary, hints })
}

/// One entry of `GET /v1/models`: a `model` value that selects it, and its display name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelInfo {
    /// What a request sends as `model`.
    pub id: String,
    /// What a person reads.
    pub name: String,
}

/// Every processing a request can select, each id a `model` value [`parse_model`] and
/// [`resolve_recipe`] accept as it is: the default, `raw`, every preset and every scene (a
/// built-in one by its category, which is the same on every machine).
pub fn model_list(presets: &[CustomPreset], scenes: &[Scene]) -> Vec<ModelInfo> {
    let mut out = vec![
        ModelInfo { id: MODEL_DEFAULT.to_owned(), name: "默认处理方式".to_owned() },
        ModelInfo { id: format!("{MODEL_PREFIX}{MODEL_RAW}"), name: "不使用 AI 润色".to_owned() },
    ];
    out.extend(BuiltinPreset::ALL.into_iter().map(|p| ModelInfo { id: format!("{MODEL_PREFIX}preset={}", p.as_str()), name: p.display_name().to_owned() }));
    out.extend(presets.iter().map(|p| ModelInfo { id: format!("{MODEL_PREFIX}preset={}", p.id), name: p.name.clone() }));
    out.extend(scenes.iter().map(|s| match s.builtin {
        Some(category) => ModelInfo { id: format!("{MODEL_PREFIX}scene={}", category.as_str()), name: category.display_name().to_owned() },
        None => ModelInfo { id: format!("{MODEL_PREFIX}scene={}", s.id), name: s.name.clone() },
    }));
    out
}
