//! Scenes and context (docs/dictation.md §18).
//!
//! A scene names applications (and, optionally, window-title keywords) and says how a take that
//! starts in one of them is processed: output mode, refine on / off and preset, language, Chinese
//! script, and an extra instruction for the LLM. The engine probes the foreground application when
//! a take starts, takes the first enabled scene that matches ([`match_scene`]) and applies its
//! overrides to that take only. This module owns the wire types, the validation, the matching, the
//! store (`scenes.json`) and [`recent_apps`] for the scene editor.
//!
//! Nothing in here may stop a take: no probe answer, no match or an override the take cannot
//! honour all mean "the global settings".
//!
//! The desktop's list also holds the built-in scenes ([`BuiltinScene`], §18.10): they carry their
//! category, may list no application, cannot be deleted or renamed, and count toward neither
//! [`MAX_SCENES`] nor the name rule of the user's scenes.
//!
//! On a phone (user decision 2026-10-01) there is no foreground probe: the user picks the scene a
//! take runs with (`Settings.pinned_scene`), the built-in scenes come without applications, and a
//! scene of the user's need not name one ([`scenes_need_apps`]).

mod builtin;
mod store;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::dictation::ports::ForegroundApp;
use crate::engines::{ChineseScript, OutputMode};
use crate::presets::PresetId;

pub use builtin::{BuiltinScene, has_builtin_scenes, scenes_need_apps};
pub use store::{SCENES_FILE_NAME, SCENES_SCHEMA, SceneStore};

/// Most scenes the user makes (the built-in ones come on top).
pub const MAX_SCENES: usize = 50;
/// Longest scene name, in characters (the pill shows it).
pub const MAX_SCENE_NAME_CHARS: usize = 32;
/// Most applications one scene lists.
pub const MAX_SCENE_APPS: usize = 20;
/// Longest application id, in characters.
pub const MAX_APP_ID_CHARS: usize = 128;
/// Most window-title keywords one scene lists.
pub const MAX_TITLE_KEYWORDS: usize = 10;
/// Longest window-title keyword, in characters.
pub const MAX_TITLE_KEYWORD_CHARS: usize = 64;
/// Longest extra instruction for the LLM, in characters.
pub const MAX_SCENE_PROMPT_CHARS: usize = 500;
/// Longest language code of an override, in characters.
pub const MAX_LANGUAGE_CHARS: usize = 16;
/// Longest application name the core keeps and sends (characters).
pub const MAX_CONTEXT_NAME_CHARS: usize = 64;
/// Longest window title the core keeps and sends (characters).
pub const MAX_CONTEXT_TITLE_CHARS: usize = 200;
/// Most entries `recent_apps` answers with (`HistoryReader::recent_apps`, docs/dictation.md §18.6).
pub const MAX_RECENT_APPS: usize = 20;
/// `SceneOverrides.language` value meaning "no language hint for this take" (auto-detect).
pub const LANGUAGE_AUTO: &str = "auto";

/// Which applications (and window titles) a scene applies to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneMatch {
    /// Normalised application ids ([`normalize_app_id`]), 1–[`MAX_SCENE_APPS`], no duplicates; a
    /// built-in scene may list none (it then never matches).
    pub apps: Vec<String>,
    /// Window-title keywords, 0–[`MAX_TITLE_KEYWORDS`]; empty = any window of those apps. Compared
    /// case-insensitively, as substrings, on this machine only.
    #[serde(default)]
    pub title_contains: Vec<String>,
}

/// What a scene changes for one take; every `None` follows the global setting.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneOverrides {
    /// Run the LLM clean-up (or not).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refine_enabled: Option<bool>,
    /// What the clean-up does (docs/dictation.md §21). Scenes written before presets stored a
    /// refine style under `refine_style`; it still reads (`default` = 校对).
    #[serde(default, alias = "refine_style", skip_serializing_if = "Option::is_none")]
    pub refine_preset: Option<PresetId>,
    /// Output mode (docs/dictation.md §12); a streaming mode needs the live preview like the
    /// global setting does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_mode: Option<OutputMode>,
    /// [`LANGUAGE_AUTO`] (no hint) or a language code (`zh`, `en`, `yue`), lower-case.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Script of the recogniser's Chinese (docs/dictation.md §17).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chinese_script: Option<ChineseScript>,
    /// Extra instruction for the LLM (1–[`MAX_SCENE_PROMPT_CHARS`] characters, newlines allowed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

impl SceneOverrides {
    /// Nothing overridden.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// One scene; the list order is the matching order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scene {
    /// Stable id.
    pub id: Uuid,
    /// Display name, unique among the user's scenes ignoring ASCII case (1–[`MAX_SCENE_NAME_CHARS`]
    /// characters); a built-in scene's is its category's wire name.
    pub name: String,
    /// Off: never matches.
    pub enabled: bool,
    /// Which applications / windows.
    #[serde(rename = "match")]
    pub matching: SceneMatch,
    /// What changes for a take in them.
    #[serde(default)]
    pub overrides: SceneOverrides,
    /// Unix milliseconds.
    pub created_at_ms: u64,
    /// Unix milliseconds of the last change.
    pub updated_at_ms: u64,
    /// The built-in category, for a built-in scene (§18.10). Kept by the store: a draft never sets
    /// or clears it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builtin: Option<BuiltinScene>,
}

impl Scene {
    /// The id and name the status and the history carry.
    pub fn to_ref(&self) -> SceneRef {
        SceneRef { id: self.id, name: self.name.clone(), builtin: self.builtin }
    }
}

/// What the UI sends to create or change a scene (`scenes_add` / `scenes_update`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneDraft {
    /// Display name.
    pub name: String,
    /// Defaults to on.
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    /// Which applications / windows.
    #[serde(rename = "match")]
    pub matching: SceneMatch,
    /// Defaults to nothing overridden.
    #[serde(default)]
    pub overrides: SceneOverrides,
}

fn enabled_by_default() -> bool {
    true
}

impl From<&Scene> for SceneDraft {
    fn from(scene: &Scene) -> Self {
        Self { name: scene.name.clone(), enabled: scene.enabled, matching: scene.matching.clone(), overrides: scene.overrides.clone() }
    }
}

/// An application as the status, the history and `recent_apps` name it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppRef {
    /// Normalised application id.
    pub id: String,
    /// Display name.
    pub name: String,
}

/// A scene as the status and the history name it (the name as it was at the time).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneRef {
    /// Scene id (the scene may have been deleted since).
    pub id: Uuid,
    /// Scene name.
    pub name: String,
    /// A built-in scene's category: the interface names it in its own language.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builtin: Option<BuiltinScene>,
}

/// The take's context (`DictationStatus.context`): the application in front when it started and
/// the scene that matched, if any. The window title is never part of it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TakeContext {
    /// The foreground application.
    pub app: AppRef,
    /// The matched scene.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene: Option<SceneRef>,
}

/// Which parts of the take's context may go to the LLM (`Settings.context_sharing`,
/// docs/dictation.md §18.5). The scene's own instruction always goes with a refine request.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ContextSharing {
    /// Send the application's display name (on by default).
    pub app_name: bool,
    /// Send the window title (off by default).
    pub window_title: bool,
}

impl Default for ContextSharing {
    fn default() -> Self {
        Self { app_name: true, window_title: false }
    }
}

/// Why a scene command, the store or a draft was refused. The text is what the UI shows: the
/// `scenes:` prefix and a Chinese detail.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SceneError {
    /// A scene or the list is not acceptable.
    #[error("scenes: {0}")]
    Invalid(String),
    /// `scenes.json` could not be written.
    #[error("scenes: {0}")]
    Store(String),
}

fn invalid(message: impl Into<String>) -> SceneError {
    SceneError::Invalid(message.into())
}

/// The id normalisation of docs/dictation.md §18.3 (`voltip_platform::foreground` applies the same
/// rule): trim → lower-case → strip trailing `.exe` (repeatedly, so it is idempotent) → trim.
pub fn normalize_app_id(raw: &str) -> String {
    let lower = raw.trim().to_lowercase();
    let mut id = lower.as_str();
    while let Some(stem) = id.strip_suffix(".exe") {
        id = stem.trim_end();
    }
    id.trim().to_owned()
}

/// One line of reference text (an app name, a window title): control characters become spaces,
/// runs of whitespace collapse, the ends are trimmed and anything past `max` characters is cut
/// with `…`. `None` when nothing is left.
pub fn clean_context_line(raw: &str, max: usize) -> Option<String> {
    let mut out = String::new();
    for word in raw.split(|c: char| c.is_whitespace() || c.is_control()).filter(|w| !w.is_empty()) {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    if out.is_empty() {
        return None;
    }
    if out.chars().count() > max {
        out = out.chars().take(max.saturating_sub(1)).collect::<String>().trim_end().to_owned();
        out.push('…');
    }
    Some(out)
}

/// A trimmed single-line value of 1..=`max` characters, or why not.
fn clean_line(value: &str, what: &str, max: usize) -> Result<String, SceneError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(invalid(format!("{what}不能为空")));
    }
    let chars = value.chars().count();
    if chars > max {
        return Err(invalid(format!("{what}最多 {max} 个字符（当前 {chars}）")));
    }
    if value.chars().any(char::is_control) {
        return Err(invalid(format!("{what}不能包含换行或控制字符")));
    }
    Ok(value.to_owned())
}

/// Validate and normalise a scene draft on its own: name trimmed; apps normalised, blanks dropped,
/// duplicates collapsed, 1–[`MAX_SCENE_APPS`]; keywords trimmed, blanks dropped, duplicates
/// (ignoring case) collapsed; the language lower-cased (blank = follow the global setting); the
/// prompt's line endings normalised and its ends trimmed (blank = none). Cross-scene checks happen
/// in the store.
pub fn validate_scene_draft(draft: &SceneDraft) -> Result<SceneDraft, SceneError> {
    validate_scene_draft_with(draft, true)
}

/// [`validate_scene_draft`], with `require_apps` off for a built-in scene, which may list no
/// application (the store decides which rule a scene gets; the bridge checks an update without it).
pub fn validate_scene_draft_with(draft: &SceneDraft, require_apps: bool) -> Result<SceneDraft, SceneError> {
    let name = clean_line(&draft.name, "场景名称", MAX_SCENE_NAME_CHARS)?;
    let mut apps: Vec<String> = Vec::new();
    for raw in &draft.matching.apps {
        if raw.trim().is_empty() {
            continue;
        }
        let normalized = normalize_app_id(raw);
        if normalized.is_empty() {
            return Err(invalid(format!("应用 id「{}」无效", raw.trim())));
        }
        let id = clean_line(&normalized, "应用 id", MAX_APP_ID_CHARS)?;
        if !apps.contains(&id) {
            apps.push(id);
        }
    }
    if apps.is_empty() && require_apps {
        return Err(invalid(format!("场景「{name}」至少要有一个应用")));
    }
    if apps.len() > MAX_SCENE_APPS {
        return Err(invalid(format!("一个场景最多 {MAX_SCENE_APPS} 个应用（当前 {}）", apps.len())));
    }
    let mut title_contains: Vec<String> = Vec::new();
    for raw in &draft.matching.title_contains {
        if raw.trim().is_empty() {
            continue;
        }
        let keyword = clean_line(raw, "窗口标题关键词", MAX_TITLE_KEYWORD_CHARS)?;
        if !title_contains.iter().any(|k| k.to_lowercase() == keyword.to_lowercase()) {
            title_contains.push(keyword);
        }
    }
    if title_contains.len() > MAX_TITLE_KEYWORDS {
        return Err(invalid(format!("一个场景最多 {MAX_TITLE_KEYWORDS} 个窗口标题关键词（当前 {}）", title_contains.len())));
    }
    let language = match draft.overrides.language.as_deref().map(str::trim).filter(|l| !l.is_empty()) {
        None => None,
        Some(raw) => {
            let code = raw.to_ascii_lowercase();
            let valid = code.len() <= MAX_LANGUAGE_CHARS && code.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && !code.starts_with('-');
            if !valid {
                return Err(invalid(format!("语言代码「{raw}」无效（字母、数字或 -，最多 {MAX_LANGUAGE_CHARS} 个字符；auto = 自动识别）")));
            }
            Some(code)
        }
    };
    let prompt = match draft.overrides.prompt.as_deref() {
        None => None,
        Some(raw) => {
            let text = raw.replace("\r\n", "\n").replace('\r', "\n");
            let text = text.trim();
            if text.is_empty() {
                None
            } else {
                let chars = text.chars().count();
                if chars > MAX_SCENE_PROMPT_CHARS {
                    return Err(invalid(format!("补充要求最多 {MAX_SCENE_PROMPT_CHARS} 个字符（当前 {chars}）")));
                }
                if text.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
                    return Err(invalid("补充要求不能包含控制字符"));
                }
                Some(text.to_owned())
            }
        }
    };
    Ok(SceneDraft {
        name,
        enabled: draft.enabled,
        matching: SceneMatch { apps, title_contains },
        overrides: SceneOverrides { language, prompt, ..draft.overrides.clone() },
    })
}

/// Rules of the whole list (docs/dictation.md §18.1): at most [`MAX_SCENES`] of the user's scenes,
/// their names unique ignoring ASCII case; each built-in category at most once, named by its wire
/// name. The same application may appear in several scenes (order decides).
pub fn check_scenes(scenes: &[Scene]) -> Result<(), SceneError> {
    let user: Vec<&Scene> = scenes.iter().filter(|s| s.builtin.is_none()).collect();
    if user.len() > MAX_SCENES {
        return Err(invalid(format!("场景最多 {MAX_SCENES} 个")));
    }
    for (i, a) in user.iter().enumerate() {
        if let Some(b) = user[..i].iter().find(|b| b.name.eq_ignore_ascii_case(&a.name)) {
            return Err(invalid(format!("已有名为「{}」的场景", b.name)));
        }
    }
    for (i, a) in scenes.iter().enumerate() {
        let Some(category) = a.builtin else { continue };
        if a.name != category.as_str() {
            return Err(invalid(format!("内置场景「{}」的名称必须是 {}", a.name, category.as_str())));
        }
        if scenes[..i].iter().any(|b| b.builtin == Some(category)) {
            return Err(invalid(format!("内置场景「{}」出现了两次", category.display_name())));
        }
    }
    Ok(())
}

/// The scene a take that starts in `app` runs with (docs/dictation.md §18.3): the first enabled
/// scene, in list order, whose apps contain the app's id and — when it lists title keywords — whose
/// keywords include one the window title contains (case-insensitively). `None` = the global settings.
pub fn match_scene<'a>(scenes: &'a [Scene], app: &ForegroundApp) -> Option<&'a Scene> {
    let id = normalize_app_id(&app.app_id);
    if id.is_empty() {
        return None;
    }
    let title = app.title.as_deref().map(str::to_lowercase);
    scenes.iter().filter(|s| s.enabled).find(|s| {
        s.matching.apps.iter().any(|a| normalize_app_id(a) == id)
            && (s.matching.title_contains.is_empty()
                || title.as_deref().is_some_and(|t| s.matching.title_contains.iter().any(|k| !k.is_empty() && t.contains(&k.to_lowercase()))))
    })
}

#[cfg(test)]
mod tests;
