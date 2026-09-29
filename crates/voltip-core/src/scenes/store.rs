//! `scenes.json` in the app data directory (docs/dictation.md §18.1), on the shared list
//! persistence of [`crate::list_file`]: every mutation validates the whole new list, writes it
//! atomically and only then replaces the list in memory; a file that cannot be used is moved aside
//! to `scenes.json.corrupt-<unix seconds>` (never deleted) and the store starts empty.
//!
//! On a desktop the list always holds the built-in scenes (§18.10): opening appends a category the
//! file lacks, switched off and after everything else, and writes the list back; nothing already
//! stored changes. They cannot be deleted or renamed, and [`SceneStore::restore`] puts their
//! defaults back.

use std::path::Path;

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use voltip_protocol::Platform;

use super::{BuiltinScene, Scene, SceneDraft, SceneError, check_scenes, has_builtin_scenes, validate_scene_draft_with};
use crate::list_file::{ListFile, permute};

/// File name of the scene list inside the app data directory.
pub const SCENES_FILE_NAME: &str = "scenes.json";
/// On-disk schema.
pub const SCENES_SCHEMA: u16 = 1;

#[derive(Serialize, Deserialize)]
struct ScenesFile {
    schema: u16,
    scenes: Vec<Scene>,
}

fn unknown_scene(id: Uuid) -> SceneError {
    SceneError::Invalid(format!("没有 id 为 {id} 的场景"))
}

fn scene_from(draft: SceneDraft, id: Uuid, created_at_ms: u64, now_ms: u64, builtin: Option<BuiltinScene>) -> Scene {
    Scene { id, name: draft.name, enabled: draft.enabled, matching: draft.matching, overrides: draft.overrides, created_at_ms, updated_at_ms: now_ms, builtin }
}

/// The scenes (`scenes.json`), in matching order.
#[derive(Debug)]
pub struct SceneStore {
    file: ListFile,
    scenes: Vec<Scene>,
    /// Whose default applications the built-in scenes get (the host's).
    platform: Platform,
}

impl SceneStore {
    /// Open `dir/scenes.json` on this host; the second value is the notice for the UI when the file
    /// could not be used (a scene that does not validate, or is not in its normalised form, counts
    /// as unusable).
    pub fn open(dir: &Path) -> (Self, Option<String>) {
        Self::open_on(dir, Platform::current(), crate::now_ms())
    }

    /// [`Self::open`] for `platform`, filling in the built-in scenes a desktop lacks at `now_ms`.
    pub fn open_on(dir: &Path, platform: Platform, now_ms: u64) -> (Self, Option<String>) {
        let (file, scenes, notice) = ListFile::load(
            dir,
            SCENES_FILE_NAME,
            SCENES_SCHEMA,
            |f: ScenesFile| (f.schema == SCENES_SCHEMA).then_some(f.scenes),
            |scenes: &[Scene]| {
                let valid = || -> Result<(), SceneError> {
                    for s in scenes {
                        let draft = SceneDraft::from(s);
                        if validate_scene_draft_with(&draft, s.builtin.is_none())? != draft {
                            return Err(SceneError::Invalid(format!("场景「{}」不是规范形式", s.name)));
                        }
                    }
                    check_scenes(scenes)
                };
                valid().map_err(|e| e.to_string())
            },
        );
        let mut store = Self { file, scenes, platform };
        store.fill_builtin(now_ms);
        (store, notice)
    }

    /// Append the built-in categories the list lacks (a desktop only) and write the list back, so
    /// their ids stay the same from one launch to the next. A list that cannot be written keeps them
    /// in memory for this run.
    fn fill_builtin(&mut self, now_ms: u64) {
        if !has_builtin_scenes(self.platform) {
            return;
        }
        let missing: Vec<BuiltinScene> = BuiltinScene::ALL.into_iter().filter(|c| !self.scenes.iter().any(|s| s.builtin == Some(*c))).collect();
        if missing.is_empty() {
            return;
        }
        let mut scenes = self.scenes.clone();
        for category in &missing {
            scenes.push(scene_from(category.template(self.platform), Uuid::new_v4(), now_ms, now_ms, Some(*category)));
        }
        let names: Vec<&str> = missing.iter().map(|c| c.as_str()).collect();
        match self.commit(scenes.clone()) {
            Ok(()) => tracing::info!(?names, "built-in scenes added"),
            Err(e) => {
                tracing::warn!(error = %e, ?names, "built-in scenes could not be saved; they are kept for this run");
                self.scenes = scenes;
            }
        }
    }

    /// Current scenes, in matching order.
    pub fn scenes(&self) -> &[Scene] {
        &self.scenes
    }

    fn commit(&mut self, scenes: Vec<Scene>) -> Result<(), SceneError> {
        check_scenes(&scenes)?;
        self.file.save(&ScenesFile { schema: SCENES_SCHEMA, scenes: scenes.clone() }).map_err(SceneError::Store)?;
        self.scenes = scenes;
        Ok(())
    }

    /// Add a new scene of the user's before the first built-in one (the user's scenes match first
    /// unless the user moves them), at the end when there is none; returns its id.
    pub fn add(&mut self, draft: &SceneDraft, now_ms: u64) -> Result<Uuid, SceneError> {
        let draft = validate_scene_draft_with(draft, true)?;
        let id = Uuid::new_v4();
        let mut scenes = self.scenes.clone();
        let at = scenes.iter().position(|s| s.builtin.is_some()).unwrap_or(scenes.len());
        scenes.insert(at, scene_from(draft, id, now_ms, now_ms, None));
        self.commit(scenes)?;
        Ok(id)
    }

    /// Replace a scene's name, flag, match and overrides (id, position, creation time and a
    /// built-in scene's category stay). A built-in scene may list no application but keeps its name.
    pub fn update(&mut self, id: Uuid, draft: &SceneDraft, now_ms: u64) -> Result<(), SceneError> {
        let mut scenes = self.scenes.clone();
        let Some(scene) = scenes.iter_mut().find(|s| s.id == id) else { return Err(unknown_scene(id)) };
        let draft = validate_scene_draft_with(draft, scene.builtin.is_none())?;
        if scene.builtin.is_some() && draft.name != scene.name {
            return Err(SceneError::Invalid("内置场景不能改名".into()));
        }
        *scene = scene_from(draft, id, scene.created_at_ms, now_ms, scene.builtin);
        self.commit(scenes)
    }

    /// Delete a scene of the user's; a built-in one can only be switched off.
    pub fn remove(&mut self, id: Uuid) -> Result<(), SceneError> {
        let Some(scene) = self.scenes.iter().find(|s| s.id == id) else { return Err(unknown_scene(id)) };
        if scene.builtin.is_some() {
            return Err(SceneError::Invalid("内置场景不能删除，可以关闭".into()));
        }
        let scenes = self.scenes.iter().filter(|s| s.id != id).cloned().collect();
        self.commit(scenes)
    }

    /// Put a built-in scene's applications and overrides back to its defaults (its switch, place
    /// and id stay).
    pub fn restore(&mut self, id: Uuid, now_ms: u64) -> Result<(), SceneError> {
        let mut scenes = self.scenes.clone();
        let Some(scene) = scenes.iter_mut().find(|s| s.id == id) else { return Err(unknown_scene(id)) };
        let Some(category) = scene.builtin else { return Err(SceneError::Invalid("只有内置场景可以恢复默认".into())) };
        let template = category.template(self.platform);
        scene.matching = template.matching;
        scene.overrides = template.overrides;
        scene.updated_at_ms = now_ms;
        self.commit(scenes)
    }

    /// Put the scenes in the order of `ids`, which must be exactly the current ids.
    pub fn reorder(&mut self, ids: &[Uuid]) -> Result<(), SceneError> {
        let scenes = permute(&self.scenes, ids, |s| s.id).ok_or_else(|| SceneError::Invalid("新的顺序必须恰好包含现有的全部场景".into()))?;
        self.commit(scenes)
    }
}
