//! How an import combines a backup's lists with the ones here (docs/dictation.md §4.6), as pure
//! functions: each returns the new list and what became of every item of the backup. The store
//! then checks the list with its own rules and writes it in one go.
//!
//! - `merge`: the list here stays as it is. An item of the backup is added unless it is here
//!   already (the same id, or the same term, name or built-in scene), does not pass the store's
//!   checks, contradicts an entry here, or the list is full.
//! - `replace`: the backup's items alone, under the same checks among themselves.
//!
//! Items keep their ids, their times and every field they carry, so a field a later version adds
//! goes along without a field list here.

use std::collections::HashMap;

use uuid::Uuid;
use voltip_protocol::Platform;

use super::{BackupPart, PartReport, Skip};
use crate::presets::{CustomPreset, MAX_PRESETS, PresetDraft, PresetId, validate_preset_draft};
use crate::scenes::{MAX_SCENES, Scene, SceneDraft, has_builtin_scenes, scenes_need_apps, validate_scene_draft_with};
use crate::vocabulary::{
    DictionaryDraft, DictionaryEntry, ImportMode, MAX_DICTIONARY_ENTRIES, MAX_RULES, ReplacementRule, RuleDraft, validate_dictionary_draft, validate_rule_draft,
};

/// A new list and what became of every item of the backup.
#[derive(Debug)]
pub(crate) struct Merged<T> {
    /// The list to store.
    pub(crate) list: Vec<T>,
    /// Restored and skipped, by reason.
    pub(crate) report: PartReport,
}

impl<T: Clone> Merged<T> {
    /// What merging starts from: the list here, or nothing.
    fn start(part: BackupPart, current: &[T], incoming: usize, mode: ImportMode) -> Self {
        let list = match mode {
            ImportMode::Merge => current.to_vec(),
            ImportMode::Replace => Vec::new(),
        };
        Self { list, report: PartReport::new(part, incoming) }
    }

    /// `item` goes in, or is skipped for `why`.
    fn take(&mut self, item: &T, why: Option<Skip>) {
        match why {
            Some(why) => self.report.skip(why),
            None => {
                self.list.push(item.clone());
                self.report.restored += 1;
            }
        }
    }
}

/// The personal dictionary (`DictionaryStore`'s rules: at most [`MAX_DICTIONARY_ENTRIES`], terms
/// unique ignoring ASCII case, a mis-hearing belongs to one entry and is nobody's term).
pub(crate) fn dictionary(current: &[DictionaryEntry], incoming: &[DictionaryEntry], mode: ImportMode) -> Merged<DictionaryEntry> {
    let mut merged = Merged::start(BackupPart::Dictionary, current, incoming.len(), mode);
    for entry in incoming {
        let list = &merged.list;
        let why = if list.iter().any(|e| e.id == entry.id) {
            Some(Skip::Duplicate)
        } else if !normalised_entry(entry) {
            Some(Skip::Invalid)
        } else if list.iter().any(|e| e.term.eq_ignore_ascii_case(&entry.term)) {
            Some(Skip::Duplicate)
        } else if list.len() >= MAX_DICTIONARY_ENTRIES {
            Some(Skip::Limit)
        } else if list.iter().any(|e| spellings_clash(e, entry)) {
            Some(Skip::Conflict)
        } else {
            None
        };
        merged.take(entry, why);
    }
    merged
}

/// The form `dictionary.json` keeps an entry in: its draft validates to itself.
fn normalised_entry(entry: &DictionaryEntry) -> bool {
    let draft = DictionaryDraft { term: entry.term.clone(), heard_as: entry.heard_as.clone(), enabled: entry.enabled };
    validate_dictionary_draft(&draft).is_ok_and(|d| d == draft)
}

/// `check_dictionary`'s rules between an entry of the list (`here`) and one after it (`new`), but
/// equal terms (a duplicate): a mis-hearing belongs to one entry, and is nobody's term.
fn spellings_clash(here: &DictionaryEntry, new: &DictionaryEntry) -> bool {
    here.heard_as.iter().any(|h| h.eq_ignore_ascii_case(&new.term))
        || new.heard_as.iter().any(|v| here.term.eq_ignore_ascii_case(v) || here.heard_as.iter().any(|h| h.eq_ignore_ascii_case(v)))
}

/// The replacement rules (`RuleStore`'s rules: at most [`MAX_RULES`], names unique, every regex
/// compiles); the backup's rules go after the ones here, in their order.
pub(crate) fn rules(current: &[ReplacementRule], incoming: &[ReplacementRule], mode: ImportMode) -> Merged<ReplacementRule> {
    let mut merged = Merged::start(BackupPart::Rules, current, incoming.len(), mode);
    for rule in incoming {
        let list = &merged.list;
        let why = if list.iter().any(|r| r.id == rule.id) {
            Some(Skip::Duplicate)
        } else if !normalised_rule(rule) {
            Some(Skip::Invalid)
        } else if list.iter().any(|r| r.name == rule.name) {
            Some(Skip::Duplicate)
        } else if list.len() >= MAX_RULES {
            Some(Skip::Limit)
        } else {
            None
        };
        merged.take(rule, why);
    }
    merged
}

/// The form `rules.json` keeps a rule in: its draft validates (its regex compiles) to itself.
fn normalised_rule(rule: &ReplacementRule) -> bool {
    let draft = RuleDraft::from(rule);
    validate_rule_draft(&draft).is_ok_and(|d| d == draft)
}

/// The custom presets (`PresetStore`'s rules: at most [`MAX_PRESETS`], names unique ignoring ASCII
/// case), and for each preset of the backup skipped because one here has its name, the id of the
/// one here: a scene of the backup that names it then names the one here ([`scenes`]).
pub(crate) fn presets(current: &[CustomPreset], incoming: &[CustomPreset], mode: ImportMode) -> (Merged<CustomPreset>, HashMap<Uuid, Uuid>) {
    let mut merged = Merged::start(BackupPart::Presets, current, incoming.len(), mode);
    let mut same = HashMap::new();
    for preset in incoming {
        let list = &merged.list;
        let why = if list.iter().any(|p| p.id == preset.id) {
            Some(Skip::Duplicate)
        } else if !normalised_preset(preset) {
            Some(Skip::Invalid)
        } else if let Some(here) = list.iter().find(|p| p.name.eq_ignore_ascii_case(&preset.name)) {
            same.insert(preset.id, here.id);
            Some(Skip::Duplicate)
        } else if list.len() >= MAX_PRESETS {
            Some(Skip::Limit)
        } else {
            None
        };
        merged.take(preset, why);
    }
    (merged, same)
}

/// The form `presets.json` keeps a preset in: its draft validates to itself.
fn normalised_preset(preset: &CustomPreset) -> bool {
    let draft = PresetDraft::from(preset);
    validate_preset_draft(&draft).is_ok_and(|d| d == draft)
}

/// The scenes (`SceneStore`'s rules on `here`: at most [`MAX_SCENES`] of the user's, their names
/// unique ignoring ASCII case, each built-in category once). A built-in scene stands for its
/// category: merging keeps the one here; replacing gives the one here the backup's switch,
/// applications and instruction, under its own id (the settings here may name it). Default
/// applications of the backup's system (`from`) become this system's. When merging, the backup's
/// own scenes go before the first built-in scene, as a new scene does. A scene that names a preset
/// of the backup skipped for one of the same name here ([`presets`]) names the one here.
pub(crate) fn scenes(
    current: &[Scene],
    incoming: &[Scene],
    mode: ImportMode,
    here: Platform,
    from: Platform,
    same_preset: &HashMap<Uuid, Uuid>,
) -> Merged<Scene> {
    let mut merged = Merged::start(BackupPart::Scenes, current, incoming.len(), mode);
    // When merging, the user's scenes of the backup, inserted before the first built-in one.
    let mut added: Vec<Scene> = Vec::new();
    let need_apps = scenes_need_apps(here);
    for scene in incoming {
        let all = || merged.list.iter().chain(added.iter());
        let why = if all().any(|s| s.id == scene.id) {
            Some(Skip::Duplicate)
        } else if !normalised_scene(scene, need_apps) {
            Some(Skip::Invalid)
        } else if let Some(category) = scene.builtin {
            if !has_builtin_scenes(here) {
                Some(Skip::Invalid)
            } else if all().any(|s| s.builtin == Some(category)) {
                // Merging keeps the one here; the same category twice in one backup contradicts itself.
                Some(if mode == ImportMode::Merge { Skip::Duplicate } else { Skip::Conflict })
            } else {
                None
            }
        } else if all().any(|s| s.builtin.is_none() && s.name.eq_ignore_ascii_case(&scene.name)) {
            Some(Skip::Duplicate)
        } else if all().filter(|s| s.builtin.is_none()).count() >= MAX_SCENES {
            Some(Skip::Limit)
        } else {
            None
        };
        if let Some(why) = why {
            merged.report.skip(why);
            continue;
        }
        let mut scene = scene.clone();
        if let Some(PresetId::Custom(id)) = scene.overrides.refine_preset
            && let Some(here_id) = same_preset.get(&id)
        {
            scene.overrides.refine_preset = Some(PresetId::Custom(*here_id));
        }
        merged.report.restored += 1;
        match scene.builtin {
            Some(category) => {
                if from != here && has_builtin_scenes(from) && scene.matching == category.template(from).matching {
                    scene.matching = category.template(here).matching;
                }
                if let Some(mine) = current.iter().find(|s| s.builtin == Some(category)) {
                    scene.id = mine.id;
                    scene.created_at_ms = mine.created_at_ms;
                }
                merged.list.push(scene);
            }
            None if mode == ImportMode::Merge => added.push(scene),
            None => merged.list.push(scene),
        }
    }
    if !added.is_empty() {
        let at = merged.list.iter().position(|s| s.builtin.is_some()).unwrap_or(merged.list.len());
        merged.list.splice(at..at, added);
    }
    merged
}

/// The form `scenes.json` keeps a scene in on a system whose scenes `need_apps` (or not): its draft
/// validates to itself, and a built-in scene is named by its category.
fn normalised_scene(scene: &Scene, need_apps: bool) -> bool {
    let draft = SceneDraft::from(scene);
    let named = scene.builtin.is_none_or(|category| scene.name == category.as_str());
    named && validate_scene_draft_with(&draft, scene.builtin.is_none() && need_apps).is_ok_and(|d| d == draft)
}
