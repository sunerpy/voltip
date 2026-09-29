//! AI presets (docs/dictation.md §21): what the clean-up does with a take. Eight built-in presets
//! ([`BuiltinPreset`]; their prompts live in `voltip_refine`) and up to [`MAX_PRESETS`] of the
//! user's own ([`CustomPreset`], `presets.json` on the shared list persistence of
//! [`crate::list_file`]). The engine settings and a scene name one by [`PresetId`]: the built-in
//! name, or a custom preset's UUID. A take whose custom preset no longer exists refines with 校对
//! ([`resolve`]), and the history says so.

use std::path::Path;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

use crate::list_file::ListFile;

/// Most custom presets kept.
pub const MAX_PRESETS: usize = 30;
/// Longest preset name, in characters (menus and the pill show it).
pub const MAX_PRESET_NAME_CHARS: usize = 24;
/// Longest custom instruction, in characters.
pub const MAX_PRESET_PROMPT_CHARS: usize = 4000;
/// Longest sample text 试一试 sends, in characters.
pub const MAX_PRESET_TRY_CHARS: usize = 2000;
/// File name of the custom presets inside the app data directory.
pub const PRESETS_FILE_NAME: &str = "presets.json";
/// On-disk schema.
pub const PRESETS_SCHEMA: u16 = 1;

/// The presets every build carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuiltinPreset {
    /// 校对 (the default).
    Proofread,
    /// 提示词优化.
    Prompt,
    /// 意图整理.
    Intent,
    /// 口语聊天.
    Chat,
    /// 中英互译.
    Translate,
    /// 要点纪要.
    Notes,
    /// 只加标点.
    Punctuation,
    /// 书面语.
    Formal,
}

impl BuiltinPreset {
    /// Every built-in preset, in the order the interface lists them.
    pub const ALL: [Self; 8] = [Self::Proofread, Self::Prompt, Self::Intent, Self::Chat, Self::Translate, Self::Notes, Self::Punctuation, Self::Formal];

    /// Wire name (`proofread` … `formal`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Proofread => "proofread",
            Self::Prompt => "prompt",
            Self::Intent => "intent",
            Self::Chat => "chat",
            Self::Translate => "translate",
            Self::Notes => "notes",
            Self::Punctuation => "punctuation",
            Self::Formal => "formal",
        }
    }

    /// The Chinese name the history keeps (the interface names a built-in preset by its id).
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Proofread => "校对",
            Self::Prompt => "提示词优化",
            Self::Intent => "意图整理",
            Self::Chat => "口语聊天",
            Self::Translate => "中英互译",
            Self::Notes => "要点纪要",
            Self::Punctuation => "只加标点",
            Self::Formal => "书面语",
        }
    }

    /// A wire name, including the refine styles scenes stored before presets existed (`default`
    /// was the proofreader, `punctuation` and `formal` keep their names).
    fn from_wire(text: &str) -> Option<Self> {
        if text == "default" {
            return Some(Self::Proofread);
        }
        Self::ALL.into_iter().find(|p| p.as_str() == text)
    }
}

/// A preset as the engine settings and a scene name it; on the wire a string: the built-in name or
/// a custom preset's UUID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PresetId {
    /// One of the built-in presets.
    Builtin(BuiltinPreset),
    /// A custom preset (it may have been deleted since).
    Custom(Uuid),
}

impl Default for PresetId {
    fn default() -> Self {
        Self::Builtin(BuiltinPreset::Proofread)
    }
}

impl PresetId {
    /// The wire string.
    pub fn to_wire(self) -> String {
        match self {
            Self::Builtin(preset) => preset.as_str().to_owned(),
            Self::Custom(id) => id.to_string(),
        }
    }

    /// Parse a wire string: a built-in name (or a refine style of old), else a UUID.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        BuiltinPreset::from_wire(text).map(Self::Builtin).or_else(|| Uuid::parse_str(text).ok().map(Self::Custom))
    }
}

impl Serialize for PresetId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_wire())
    }
}

impl<'de> Deserialize<'de> for PresetId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).ok_or_else(|| serde::de::Error::custom(format!("unknown preset {text:?}: a built-in name or a custom preset's UUID")))
    }
}

/// One of the user's presets.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomPreset {
    /// Stable id.
    pub id: Uuid,
    /// Display name, unique among the custom presets ignoring ASCII case.
    pub name: String,
    /// The instruction the clean-up follows (the output contract is added by the refiner).
    pub prompt: String,
    /// Unix milliseconds.
    pub created_at_ms: u64,
    /// Unix milliseconds of the last change.
    pub updated_at_ms: u64,
}

/// What the interface sends to create or change a custom preset.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresetDraft {
    /// Display name, 1–[`MAX_PRESET_NAME_CHARS`] characters on one line.
    pub name: String,
    /// The instruction, 1–[`MAX_PRESET_PROMPT_CHARS`] characters, newlines allowed.
    pub prompt: String,
}

impl From<&CustomPreset> for PresetDraft {
    fn from(preset: &CustomPreset) -> Self {
        Self { name: preset.name.clone(), prompt: preset.prompt.clone() }
    }
}

/// Why a preset command, the store or a draft was refused: the `presets:` prefix and a Chinese
/// detail, which the interface shows.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PresetError {
    /// A preset or the list is not acceptable.
    #[error("presets: {0}")]
    Invalid(String),
    /// `presets.json` could not be written.
    #[error("presets: {0}")]
    Store(String),
}

fn invalid(message: impl Into<String>) -> PresetError {
    PresetError::Invalid(message.into())
}

/// The instruction of a draft (or of 试一试) normalised: line endings, trimmed ends, length, no
/// control characters but the newline and the tab.
pub fn clean_preset_prompt(raw: &str) -> Result<String, PresetError> {
    let text = raw.replace("\r\n", "\n").replace('\r', "\n");
    let text = text.trim();
    if text.is_empty() {
        return Err(invalid("预设内容不能为空"));
    }
    let chars = text.chars().count();
    if chars > MAX_PRESET_PROMPT_CHARS {
        return Err(invalid(format!("预设内容最多 {MAX_PRESET_PROMPT_CHARS} 个字符（当前 {chars}）")));
    }
    if text.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
        return Err(invalid("预设内容不能包含控制字符"));
    }
    Ok(text.to_owned())
}

/// Validate and normalise a draft on its own: the name trimmed and on one line, the instruction
/// cleaned ([`clean_preset_prompt`]). Checks across the list happen in the store.
pub fn validate_preset_draft(draft: &PresetDraft) -> Result<PresetDraft, PresetError> {
    let name = draft.name.trim();
    if name.is_empty() {
        return Err(invalid("预设名称不能为空"));
    }
    let chars = name.chars().count();
    if chars > MAX_PRESET_NAME_CHARS {
        return Err(invalid(format!("预设名称最多 {MAX_PRESET_NAME_CHARS} 个字符（当前 {chars}）")));
    }
    if name.chars().any(char::is_control) {
        return Err(invalid("预设名称不能包含换行或控制字符"));
    }
    Ok(PresetDraft { name: name.to_owned(), prompt: clean_preset_prompt(&draft.prompt)? })
}

/// Rules of the whole list: at most [`MAX_PRESETS`], names unique ignoring ASCII case.
pub fn check_presets(presets: &[CustomPreset]) -> Result<(), PresetError> {
    if presets.len() > MAX_PRESETS {
        return Err(invalid(format!("自定义预设最多 {MAX_PRESETS} 个")));
    }
    for (i, a) in presets.iter().enumerate() {
        if let Some(b) = presets[..i].iter().find(|b| b.name.eq_ignore_ascii_case(&a.name)) {
            return Err(invalid(format!("已有名为「{}」的预设", b.name)));
        }
    }
    Ok(())
}

/// What a take refines with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TakePreset {
    /// A built-in preset.
    Builtin(BuiltinPreset),
    /// A custom preset as it was when the take started.
    Custom {
        /// Its id.
        id: Uuid,
        /// Its name.
        name: String,
        /// Its instruction.
        prompt: String,
    },
}

impl Default for TakePreset {
    fn default() -> Self {
        Self::Builtin(BuiltinPreset::Proofread)
    }
}

impl TakePreset {
    /// The id and name the history carries.
    pub fn to_ref(&self) -> PresetRef {
        match self {
            Self::Builtin(preset) => PresetRef { id: PresetId::Builtin(*preset), name: preset.display_name().to_owned() },
            Self::Custom { id, name, .. } => PresetRef { id: PresetId::Custom(*id), name: name.clone() },
        }
    }
}

/// A preset as the history names it (its name at the time).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresetRef {
    /// The preset (a custom one may have been deleted since).
    pub id: PresetId,
    /// Its name then.
    pub name: String,
}

/// The preset `id` names, looked up in `presets`; a custom id that is not there (deleted, or its
/// file set aside) is 校对, and the second value says it went missing.
pub fn resolve(id: PresetId, presets: &[CustomPreset]) -> (TakePreset, bool) {
    match id {
        PresetId::Builtin(preset) => (TakePreset::Builtin(preset), false),
        PresetId::Custom(id) => match presets.iter().find(|p| p.id == id) {
            Some(p) => (TakePreset::Custom { id, name: p.name.clone(), prompt: p.prompt.clone() }, false),
            None => (TakePreset::default(), true),
        },
    }
}

/// What 试一试 runs (docs/dictation.md §21): a saved preset, or the instruction being edited.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PresetTrial {
    /// A built-in or saved custom preset.
    Preset(PresetId),
    /// An unsaved instruction (already cleaned by [`clean_preset_prompt`]).
    Prompt(String),
}

impl PresetTrial {
    /// The preset the trial refines with; a missing custom preset is 校对, like a take's.
    pub fn resolve(&self, presets: &[CustomPreset]) -> TakePreset {
        match self {
            Self::Preset(id) => resolve(*id, presets).0,
            Self::Prompt(prompt) => TakePreset::Custom { id: Uuid::nil(), name: String::new(), prompt: prompt.clone() },
        }
    }
}

/// The answer to one 试一试 (not cached: the dialog that asked shows it).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PresetTryOutcome {
    /// The clean-up's text.
    Ok {
        /// The processed text.
        text: String,
        /// Round trip, milliseconds.
        latency_ms: u64,
        /// The model that answered.
        model: String,
    },
    /// Why nothing came back (no AI service configured, the service failed).
    Failed {
        /// Plain text for the dialog.
        reason: String,
    },
}

#[derive(Serialize, Deserialize)]
struct PresetsFile {
    schema: u16,
    presets: Vec<CustomPreset>,
}

fn unknown_preset(id: Uuid) -> PresetError {
    PresetError::Invalid(format!("没有 id 为 {id} 的预设"))
}

/// The custom presets (`presets.json`), in the order they were made.
#[derive(Debug)]
pub struct PresetStore {
    file: ListFile,
    presets: Vec<CustomPreset>,
}

impl PresetStore {
    /// Open `dir/presets.json`; the second value is the notice for the interface when the file
    /// could not be used (a preset that does not validate, or is not in its normalised form, counts
    /// as unusable).
    pub fn open(dir: &Path) -> (Self, Option<String>) {
        let (file, presets, notice) = ListFile::load(
            dir,
            PRESETS_FILE_NAME,
            PRESETS_SCHEMA,
            |f: PresetsFile| (f.schema == PRESETS_SCHEMA).then_some(f.presets),
            |presets: &[CustomPreset]| {
                let valid = || -> Result<(), PresetError> {
                    for p in presets {
                        let draft = PresetDraft::from(p);
                        if validate_preset_draft(&draft)? != draft {
                            return Err(invalid(format!("预设「{}」不是规范形式", p.name)));
                        }
                    }
                    check_presets(presets)
                };
                valid().map_err(|e| e.to_string())
            },
        );
        (Self { file, presets }, notice)
    }

    /// Current custom presets.
    pub fn presets(&self) -> &[CustomPreset] {
        &self.presets
    }

    fn commit(&mut self, presets: Vec<CustomPreset>) -> Result<(), PresetError> {
        check_presets(&presets)?;
        self.file.save(&PresetsFile { schema: PRESETS_SCHEMA, presets: presets.clone() }).map_err(PresetError::Store)?;
        self.presets = presets;
        Ok(())
    }

    /// Append a custom preset; returns its id.
    pub fn add(&mut self, draft: &PresetDraft, now_ms: u64) -> Result<Uuid, PresetError> {
        let draft = validate_preset_draft(draft)?;
        let id = Uuid::new_v4();
        let mut presets = self.presets.clone();
        presets.push(CustomPreset { id, name: draft.name, prompt: draft.prompt, created_at_ms: now_ms, updated_at_ms: now_ms });
        self.commit(presets)?;
        Ok(id)
    }

    /// Replace a preset's name and instruction (id, position and creation time stay).
    pub fn update(&mut self, id: Uuid, draft: &PresetDraft, now_ms: u64) -> Result<(), PresetError> {
        let draft = validate_preset_draft(draft)?;
        let mut presets = self.presets.clone();
        let Some(preset) = presets.iter_mut().find(|p| p.id == id) else { return Err(unknown_preset(id)) };
        preset.name = draft.name;
        preset.prompt = draft.prompt;
        preset.updated_at_ms = now_ms;
        self.commit(presets)
    }

    /// Delete a preset. Settings and scenes that name it keep the id; their takes refine with 校对.
    pub fn remove(&mut self, id: Uuid) -> Result<(), PresetError> {
        if !self.presets.iter().any(|p| p.id == id) {
            return Err(unknown_preset(id));
        }
        let presets = self.presets.iter().filter(|p| p.id != id).cloned().collect();
        self.commit(presets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(name: &str, prompt: &str) -> PresetDraft {
        PresetDraft { name: name.into(), prompt: prompt.into() }
    }

    #[test]
    fn preset_ids_are_strings_on_the_wire_and_old_refine_styles_still_read() {
        for preset in BuiltinPreset::ALL {
            let id = PresetId::Builtin(preset);
            assert_eq!(serde_json::to_string(&id).unwrap(), format!("\"{}\"", preset.as_str()));
            assert_eq!(serde_json::from_str::<PresetId>(&format!("\"{}\"", preset.as_str())).unwrap(), id);
        }
        let custom = Uuid::new_v4();
        assert_eq!(serde_json::from_str::<PresetId>(&format!("\"{custom}\"")).unwrap(), PresetId::Custom(custom));
        assert_eq!(serde_json::to_string(&PresetId::Custom(custom)).unwrap(), format!("\"{custom}\""));
        // The refine styles scenes stored before presets: `default` was the proofreader.
        assert_eq!(serde_json::from_str::<PresetId>("\"default\"").unwrap(), PresetId::Builtin(BuiltinPreset::Proofread));
        assert_eq!(PresetId::parse(" formal "), Some(PresetId::Builtin(BuiltinPreset::Formal)));
        assert!(serde_json::from_str::<PresetId>("\"casual\"").is_err());
        assert_eq!(PresetId::default(), PresetId::Builtin(BuiltinPreset::Proofread));
        assert_eq!(BuiltinPreset::ALL.map(BuiltinPreset::display_name)[..2], ["校对", "提示词优化"]);
    }

    #[test]
    fn drafts_are_trimmed_and_limited() {
        let ok = validate_preset_draft(&draft(" 周报 ", " 整理成周报。\r\n分三段。 ")).unwrap();
        assert_eq!(ok, draft("周报", "整理成周报。\n分三段。"));
        let too_long_name = "名".repeat(MAX_PRESET_NAME_CHARS + 1);
        let too_long_prompt = "字".repeat(MAX_PRESET_PROMPT_CHARS + 1);
        for (bad, why) in [
            (draft(" ", "x"), "预设名称不能为空"),
            (draft(&too_long_name, "x"), "预设名称最多 24 个字符（当前 25）"),
            (draft("a\nb", "x"), "预设名称不能包含换行或控制字符"),
            (draft("周报", " \n "), "预设内容不能为空"),
            (draft("周报", &too_long_prompt), "预设内容最多 4000 个字符（当前 4001）"),
            (draft("周报", "a\u{7}b"), "预设内容不能包含控制字符"),
        ] {
            assert_eq!(validate_preset_draft(&bad), Err(PresetError::Invalid(why.into())), "{bad:?}");
        }
        assert!(
            validate_preset_draft(&draft(&"名".repeat(MAX_PRESET_NAME_CHARS), &"字".repeat(MAX_PRESET_PROMPT_CHARS))).is_ok(),
            "the limits themselves pass"
        );
        assert_eq!(clean_preset_prompt("\t缩进\t保留\n").unwrap(), "缩进\t保留");
    }

    #[test]
    fn the_store_adds_updates_removes_and_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, notice) = PresetStore::open(dir.path());
        assert!(notice.is_none() && store.presets().is_empty());
        let a = store.add(&draft("周报", "整理成周报"), 1).unwrap();
        let b = store.add(&draft("邮件", "写成邮件"), 2).unwrap();
        assert_eq!(store.add(&draft("周报", "x"), 3), Err(PresetError::Invalid("已有名为「周报」的预设".into())));
        store.update(a, &draft("周报（短）", "三句话以内"), 4).unwrap();
        assert_eq!(store.update(Uuid::nil(), &draft("x", "y"), 5), Err(PresetError::Invalid(format!("没有 id 为 {} 的预设", Uuid::nil()))));
        store.remove(b).unwrap();
        assert!(store.remove(b).is_err(), "gone already");
        let (again, notice) = PresetStore::open(dir.path());
        assert!(notice.is_none());
        assert_eq!(again.presets().len(), 1);
        let kept = &again.presets()[0];
        assert_eq!((kept.id, kept.name.as_str(), kept.prompt.as_str(), kept.created_at_ms, kept.updated_at_ms), (a, "周报（短）", "三句话以内", 1, 4));
    }

    #[test]
    fn the_list_is_capped() {
        let dir = tempfile::tempdir().unwrap();
        let (mut store, _) = PresetStore::open(dir.path());
        for n in 0..MAX_PRESETS {
            store.add(&draft(&format!("预设 {n}"), "x"), 1).unwrap();
        }
        assert_eq!(store.add(&draft("再多一个", "x"), 2), Err(PresetError::Invalid("自定义预设最多 30 个".into())));
    }

    #[test]
    fn a_file_that_cannot_be_used_is_set_aside() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(PRESETS_FILE_NAME),
            r#"{"schema":1,"presets":[{"id":"00000000-0000-4000-8000-000000000001","name":" 空格 ","prompt":"x","created_at_ms":1,"updated_at_ms":1}]}"#,
        )
        .unwrap();
        let (store, notice) = PresetStore::open(dir.path());
        assert!(store.presets().is_empty());
        assert!(notice.unwrap().contains("不是规范形式"));
        assert!(!dir.path().join(PRESETS_FILE_NAME).exists(), "moved aside, never deleted");
    }

    #[test]
    fn a_missing_custom_preset_refines_with_proofread_and_says_so() {
        let kept = CustomPreset { id: Uuid::new_v4(), name: "周报".into(), prompt: "整理成周报".into(), created_at_ms: 1, updated_at_ms: 1 };
        let presets = [kept.clone()];
        assert_eq!(resolve(PresetId::Builtin(BuiltinPreset::Notes), &presets), (TakePreset::Builtin(BuiltinPreset::Notes), false));
        let (found, missing) = resolve(PresetId::Custom(kept.id), &presets);
        assert!(!missing);
        assert_eq!(found, TakePreset::Custom { id: kept.id, name: "周报".into(), prompt: "整理成周报".into() });
        assert_eq!(found.to_ref(), PresetRef { id: PresetId::Custom(kept.id), name: "周报".into() });
        let (fallback, missing) = resolve(PresetId::Custom(Uuid::new_v4()), &presets);
        assert!(missing);
        assert_eq!(fallback.to_ref(), PresetRef { id: PresetId::Builtin(BuiltinPreset::Proofread), name: "校对".into() });
    }
}
