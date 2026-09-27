//! `scenes.json` in the app data directory (docs/dictation.md §18.1), on the shared list
//! persistence of [`crate::list_file`]: every mutation validates the whole new list, writes it
//! atomically and only then replaces the list in memory; a file that cannot be used is moved aside
//! to `scenes.json.corrupt-<unix seconds>` (never deleted) and the store starts empty.

use std::path::Path;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{Scene, SceneDraft, SceneError, check_scenes, validate_scene_draft};
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

fn scene_from(draft: SceneDraft, id: Uuid, created_at_ms: u64, now_ms: u64) -> Scene {
    Scene { id, name: draft.name, enabled: draft.enabled, matching: draft.matching, overrides: draft.overrides, created_at_ms, updated_at_ms: now_ms }
}

/// The scenes (`scenes.json`), in matching order.
#[derive(Debug)]
pub struct SceneStore {
    file: ListFile,
    scenes: Vec<Scene>,
}

impl SceneStore {
    /// Open `dir/scenes.json`; the second value is the notice for the UI when the file could not
    /// be used (a scene that does not validate, or is not in its normalised form, counts as unusable).
    pub fn open(dir: &Path) -> (Self, Option<String>) {
        let (file, scenes, notice) = ListFile::load(
            dir,
            SCENES_FILE_NAME,
            SCENES_SCHEMA,
            |f: ScenesFile| (f.schema == SCENES_SCHEMA).then_some(f.scenes),
            |scenes: &[Scene]| {
                let valid = || -> Result<(), SceneError> {
                    for s in scenes {
                        let draft = SceneDraft::from(s);
                        if validate_scene_draft(&draft)? != draft {
                            return Err(SceneError::Invalid(format!("场景「{}」不是规范形式", s.name)));
                        }
                    }
                    check_scenes(scenes)
                };
                valid().map_err(|e| e.to_string())
            },
        );
        (Self { file, scenes }, notice)
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

    /// Append a new scene; returns its id.
    pub fn add(&mut self, draft: &SceneDraft, now_ms: u64) -> Result<Uuid, SceneError> {
        let draft = validate_scene_draft(draft)?;
        let id = Uuid::new_v4();
        let mut scenes = self.scenes.clone();
        scenes.push(scene_from(draft, id, now_ms, now_ms));
        self.commit(scenes)?;
        Ok(id)
    }

    /// Replace a scene's name, flag, match and overrides (id, position and creation time stay).
    pub fn update(&mut self, id: Uuid, draft: &SceneDraft, now_ms: u64) -> Result<(), SceneError> {
        let draft = validate_scene_draft(draft)?;
        let mut scenes = self.scenes.clone();
        let Some(scene) = scenes.iter_mut().find(|s| s.id == id) else { return Err(unknown_scene(id)) };
        *scene = scene_from(draft, id, scene.created_at_ms, now_ms);
        self.commit(scenes)
    }

    /// Delete a scene.
    pub fn remove(&mut self, id: Uuid) -> Result<(), SceneError> {
        if !self.scenes.iter().any(|s| s.id == id) {
            return Err(unknown_scene(id));
        }
        let scenes = self.scenes.iter().filter(|s| s.id != id).cloned().collect();
        self.commit(scenes)
    }

    /// Put the scenes in the order of `ids`, which must be exactly the current ids.
    pub fn reorder(&mut self, ids: &[Uuid]) -> Result<(), SceneError> {
        let scenes = permute(&self.scenes, ids, |s| s.id).ok_or_else(|| SceneError::Invalid("新的顺序必须恰好包含现有的全部场景".into()))?;
        self.commit(scenes)
    }
}
