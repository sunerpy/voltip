//! Moving the model library to another folder (docs/dictation.md §10.9, issue #192: 「模型下载，
//! 不能设置保存路径？C 盘满了」).
//!
//! Every catalogue folder `<from>/<id>` goes to `<to>/<id>`. On one file system that is a rename.
//! Otherwise the folder is copied into `<to>/<id>.moving`, every file flushed to the disk and its
//! size checked against the original, and only then renamed to `<to>/<id>`. Nothing is deleted
//! and nothing reads from `to` until every folder is complete there: the caller's commit (the
//! location file) runs next, then the shared [`ModelsRoot`](crate::ModelsRoot) switches, then the
//! old copies are deleted. Whatever fails before the commit — a full disk, a file that cannot be
//! read, the cancel button — undoes what the move did, and the library stays where it was.
//!
//! What `to` already holds: a complete copy of a model is kept and the old one deleted; an empty
//! folder is replaced; anything else under a model's name stops the move before anything changes
//! (it may be files a person downloaded by hand). `<id>.moving` is what an interrupted copy left
//! behind, and is removed. Only catalogue folders are touched, in both places: a folder a person
//! chose may hold other things.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};

use voltip_core::CancelToken;

use crate::catalogue::ModelEntry;
use crate::store::{DISK_HEADROOM, IDLE, MOVING, ModelStore, StoreError, is_installed, sync_dir};

/// Suffix of a model folder while it is being copied into the new place.
pub const STAGING_SUFFIX: &str = ".moving";
/// The empty file that tells whether the two folders are on one file system.
const PROBE_FILE: &str = ".voltip-move-probe";
/// Bytes copied between two looks at the cancel token.
const CHUNK: usize = 1 << 20;

/// What [`ModelStore::move_to`] calls once every model is complete in the new folder, with the
/// old folders it deletes next ([`voltip_core::RelocateCommit`] without the `Arc`).
pub type Commit<'a> = &'a dyn Fn(&Path, &[PathBuf]) -> Result<(), String>;

/// One model folder to move.
struct Item {
    entry: &'static ModelEntry,
    src: PathBuf,
    dst: PathBuf,
    staging: PathBuf,
    bytes: u64,
}

/// What the move has done so far, to undo it.
enum Done {
    /// `src` is now `dst`.
    Renamed { src: PathBuf, dst: PathBuf },
    /// `dst` is a copy; its original is still there.
    Copied { dst: PathBuf },
}

/// Holds the store's background slot for a move; frees it when dropped.
struct MoveClaim<'a>(&'a AtomicU8);

impl Drop for MoveClaim<'_> {
    fn drop(&mut self) {
        self.0.store(IDLE, Ordering::SeqCst);
    }
}

impl ModelStore {
    /// Move the library into `to`; see the module documentation. Blocking (it may copy
    /// gigabytes): call it from a blocking thread. `progress` gets the bytes copied so far and in
    /// all, `commit` runs once every model is complete in `to` with the old folders the move is
    /// about to delete. Returns the old folders that could not be deleted (a file still in use).
    pub fn move_to(&self, to: &Path, progress: &dyn Fn(u64, u64), cancel: &CancelToken, commit: Commit<'_>) -> Result<Vec<PathBuf>, StoreError> {
        let _claim = self.claim_move()?;
        let from = self.root();
        if !to.is_absolute() {
            return Err(StoreError::Move(format!("{} 不是完整的路径", to.display())));
        }
        // Checked before anything is created: a refused folder is left as it was.
        let to_real = real_path(to)?;
        let from_real = match std::fs::canonicalize(&from) {
            Ok(real) => Some(real),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        if from_real.as_deref() == Some(to_real.as_path()) {
            return self.stay(to, commit);
        }
        if let Some(from_real) = &from_real {
            // `to` inside a model's folder, or the library inside the folder a model would take.
            let inside = self.catalogue.iter().any(|e| to_real.starts_with(from_real.join(e.id)));
            let around = self.catalogue.iter().any(|e| from_real.starts_with(to_real.join(e.id)));
            if inside || around {
                return Err(StoreError::Move("不能把模型移到模型自己的文件夹里，请选择别的文件夹".into()));
            }
        }
        std::fs::create_dir_all(to).map_err(|e| StoreError::Move(format!("无法创建文件夹 {}：{e}", to.display())))?;
        let probe = to.join(PROBE_FILE);
        let mark = probe_mark();
        std::fs::write(&probe, mark.as_bytes()).map_err(|e| StoreError::Move(format!("无法写入所选文件夹 {}：{e}", to.display())))?;
        // The same folder under a name the paths above did not reveal (letter case on a file system
        // that ignores it, a mount): its models must not be "moved" onto themselves and deleted.
        if std::fs::read(from.join(PROBE_FILE)).is_ok_and(|seen| seen == mark.as_bytes()) {
            let _ = std::fs::remove_file(&probe);
            return self.stay(to, commit);
        }
        let renames = renames_into(&probe, &from) && self.renames;
        let (items, mut doomed) = if from_real.is_some() { self.plan(&from, to)? } else { (Vec::new(), Vec::new()) };
        let same_fs = renames && !items.is_empty();
        let mut total: u64 = if same_fs { 0 } else { items.iter().map(|i| i.bytes).sum() };
        if total > 0 {
            self.check_move_space(to, total)?;
        }
        let mut done = Vec::new();
        let mut moved = 0;
        progress(moved, total);
        let outcome = self.carry(&items, same_fs, to, &mut done, &mut doomed, &mut moved, &mut total, progress, cancel);
        if let Err(e) = outcome {
            undo(&done);
            tracing::info!(error = %e, "moving the models failed; everything is back where it was");
            return Err(e);
        }
        if let Err(e) = commit(to, &doomed) {
            undo(&done);
            tracing::warn!(error = %e, "the models' new location could not be saved; the move was undone");
            return Err(StoreError::Move(format!("无法保存模型的新位置：{e}")));
        }
        self.root.set(to);
        tracing::info!(from = %from.display(), to = %to.display(), models = items.len(), "models moved");
        let leftovers: Vec<PathBuf> = doomed.into_iter().filter(|dir| !remove_tree(dir)).collect();
        Ok(leftovers)
    }

    /// `to` is the folder the library is in already: nothing moves; the commit records the name.
    fn stay(&self, to: &Path, commit: Commit<'_>) -> Result<Vec<PathBuf>, StoreError> {
        commit(to, &[]).map_err(|e| StoreError::Move(format!("无法保存模型的位置：{e}")))?;
        self.root.set(to);
        Ok(Vec::new())
    }

    /// Take the store's background slot, or say why it is taken.
    fn claim_move(&self) -> Result<MoveClaim<'_>, StoreError> {
        match self.busy.compare_exchange(IDLE, MOVING, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => Ok(MoveClaim(&self.busy)),
            Err(MOVING) => Err(StoreError::Busy("正在移动模型，请稍后再试")),
            Err(_) => Err(StoreError::Busy("正在下载模型，请等下载结束后再移动")),
        }
    }

    /// The folders to move, and the old folders to delete once the move is done (empty ones, and
    /// those whose model `to` already holds complete). Fails before anything changed when `to`
    /// holds something under a model's name that is not that model; clears what an earlier,
    /// interrupted move left there.
    fn plan(&self, from: &Path, to: &Path) -> Result<(Vec<Item>, Vec<PathBuf>), StoreError> {
        let (mut items, mut doomed, mut clear) = (Vec::new(), Vec::new(), Vec::new());
        for entry in self.catalogue {
            let src = from.join(entry.id);
            if !std::fs::metadata(&src).is_ok_and(|m| m.is_dir()) {
                continue;
            }
            if is_empty(&src)? {
                doomed.push(src);
                continue;
            }
            let dst = to.join(entry.id);
            let staging = to.join(format!("{}{STAGING_SUFFIX}", entry.id));
            if std::fs::symlink_metadata(&staging).is_ok() {
                clear.push(staging.clone());
            }
            match std::fs::metadata(&dst) {
                Ok(meta) if meta.is_dir() && is_installed(&dst, entry) => {
                    // The new place has this model already: keep that copy.
                    doomed.push(src);
                    continue;
                }
                Ok(meta) if meta.is_dir() && is_empty(&dst)? => clear.push(dst.clone()),
                Ok(_) => {
                    return Err(StoreError::Move(format!("所选文件夹里已经有 {}，而且不是完整的模型。请先把它移走或删除，再移动模型", dst.display())));
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            let bytes = tree_size(&src)?;
            items.push(Item { entry, src, dst, staging, bytes });
        }
        for dir in clear {
            std::fs::remove_dir_all(&dir).map_err(|e| StoreError::Move(format!("无法删除 {}：{e}", dir.display())))?;
        }
        Ok((items, doomed))
    }

    /// Refuse a copy that would not fit where it goes, with [`DISK_HEADROOM`] to spare. A disk
    /// that cannot say how much is free does not block it: the copy fails on its own when full.
    fn check_move_space(&self, to: &Path, bytes: u64) -> Result<(), StoreError> {
        let needed = bytes.saturating_add(DISK_HEADROOM);
        match self.free_space.at(to) {
            Ok(available) if available < needed => {
                tracing::warn!(needed, available, "not enough free space to move the models");
                Err(StoreError::MoveNoSpace { needed, available })
            }
            Ok(_) => Ok(()),
            Err(e) => {
                tracing::warn!(error = %e, "free space unknown; moving the models anyway");
                Ok(())
            }
        }
    }

    /// Put every item in its new place; `done` and `doomed` record what happened.
    #[allow(clippy::too_many_arguments)]
    fn carry(
        &self,
        items: &[Item],
        same_fs: bool,
        to: &Path,
        done: &mut Vec<Done>,
        doomed: &mut Vec<PathBuf>,
        moved: &mut u64,
        total: &mut u64,
        progress: &dyn Fn(u64, u64),
        cancel: &CancelToken,
    ) -> Result<(), StoreError> {
        for item in items {
            if cancel.is_cancelled() {
                return Err(StoreError::Cancelled);
            }
            if same_fs {
                match std::fs::rename(&item.src, &item.dst) {
                    Ok(()) => {
                        sync_dir(Some(to));
                        done.push(Done::Renamed { src: item.src.clone(), dst: item.dst.clone() });
                        continue;
                    }
                    Err(e) => {
                        // A file in it may be in use (Windows); a copy reads it all the same.
                        tracing::info!(model = item.entry.id, error = %e, "a model folder did not rename; copying it instead");
                        self.check_move_space(to, item.bytes)?;
                        *total += item.bytes;
                        progress(*moved, *total);
                    }
                }
            }
            if let Err(e) = copy_tree(&item.src, &item.staging, moved, *total, progress, cancel).and_then(|()| check_copy(item)) {
                remove_tree(&item.staging);
                return Err(e);
            }
            std::fs::rename(&item.staging, &item.dst).map_err(|e| {
                remove_tree(&item.staging);
                StoreError::Move(format!("无法完成 {} 的移动：{e}", item.dst.display()))
            })?;
            sync_dir(Some(to));
            done.push(Done::Copied { dst: item.dst.clone() });
            doomed.push(item.src.clone());
        }
        Ok(())
    }
}

/// What the probe file holds: this process at this instant, so a probe an earlier move left behind
/// is never taken for this one.
fn probe_mark() -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    format!("{}-{nanos}", std::process::id())
}

/// `path` with its symbolic links and `..` resolved, as far as it exists; the rest is appended
/// as written (a folder about to be created).
fn real_path(path: &Path) -> Result<PathBuf, StoreError> {
    let mut existing = path;
    let mut rest = Vec::new();
    loop {
        match std::fs::canonicalize(existing) {
            Ok(real) => return Ok(rest.iter().rev().fold(real, |path, name| path.join(name))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let (Some(parent), Some(name)) = (existing.parent(), existing.file_name()) else { return Err(e.into()) };
                rest.push(name.to_owned());
                existing = parent;
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// Whether a rename from the folder `probe` is in to `dir` works (one file system): `probe` is
/// moved there and deleted, whatever the answer.
fn renames_into(probe: &Path, dir: &Path) -> bool {
    let back = dir.join(PROBE_FILE);
    let renamed = std::fs::rename(probe, &back).is_ok();
    let _ = std::fs::remove_file(if renamed { &back } else { probe });
    renamed
}

/// Undo `done`, newest first: renamed folders go back, copies are deleted. What cannot be undone
/// is logged; the library stays where it was either way.
fn undo(done: &[Done]) {
    for step in done.iter().rev() {
        match step {
            Done::Renamed { src, dst } => {
                if let Err(e) = std::fs::rename(dst, src) {
                    tracing::error!(from = %dst.display(), to = %src.display(), error = %e, "a moved model folder could not be put back");
                }
            }
            Done::Copied { dst } => {
                remove_tree(dst);
            }
        }
    }
}

/// Copy the folder `src` to `dst` (which must not exist), file by file, every file flushed to
/// the disk; `moved` counts the bytes copied.
fn copy_tree(src: &Path, dst: &Path, moved: &mut u64, total: u64, progress: &dyn Fn(u64, u64), cancel: &CancelToken) -> Result<(), StoreError> {
    std::fs::create_dir(dst).map_err(|e| StoreError::Move(format!("无法创建文件夹 {}：{e}", dst.display())))?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let (from, to) = (entry.path(), dst.join(entry.file_name()));
        if std::fs::metadata(&from)?.is_dir() {
            copy_tree(&from, &to, moved, total, progress, cancel)?;
        } else {
            copy_file(&from, &to, moved, total, progress, cancel)?;
        }
    }
    sync_dir(Some(dst));
    Ok(())
}

fn copy_file(from: &Path, to: &Path, moved: &mut u64, total: u64, progress: &dyn Fn(u64, u64), cancel: &CancelToken) -> Result<(), StoreError> {
    let failed = |e: std::io::Error| StoreError::Move(format!("复制 {} 失败：{e}", from.display()));
    let mut input = std::fs::File::open(from).map_err(failed)?;
    let expected = input.metadata().map_err(failed)?.len();
    let mut output = std::fs::OpenOptions::new().write(true).create_new(true).open(to).map_err(failed)?;
    let mut buf = vec![0u8; CHUNK];
    let mut copied = 0u64;
    loop {
        if cancel.is_cancelled() {
            return Err(StoreError::Cancelled);
        }
        let n = input.read(&mut buf).map_err(failed)?;
        if n == 0 {
            break;
        }
        output.write_all(&buf[..n]).map_err(failed)?;
        copied += n as u64;
        *moved += n as u64;
        progress(*moved, total.max(*moved));
    }
    output.sync_all().map_err(failed)?;
    if copied != expected || output.metadata().map_err(failed)?.len() != expected {
        return Err(StoreError::Move(format!("复制 {} 后大小不一致", from.display())));
    }
    Ok(())
}

/// The copy holds as many bytes as the original, and a complete model is still complete.
fn check_copy(item: &Item) -> Result<(), StoreError> {
    let copied = tree_size(&item.staging)?;
    if copied != item.bytes || (is_installed(&item.src, item.entry) && !is_installed(&item.staging, item.entry)) {
        return Err(StoreError::Move(format!("复制 {} 后检查未通过", item.src.display())));
    }
    Ok(())
}

/// Bytes of every file under `dir`.
fn tree_size(dir: &Path) -> Result<u64, StoreError> {
    let mut bytes = 0;
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let meta = std::fs::metadata(&path)?;
        bytes += if meta.is_dir() { tree_size(&path)? } else { meta.len() };
    }
    Ok(bytes)
}

fn is_empty(dir: &Path) -> Result<bool, StoreError> {
    Ok(std::fs::read_dir(dir)?.next().is_none())
}

/// Delete `dir` and what it holds; `false` when some of it is still there.
fn remove_tree(dir: &Path) -> bool {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
        Err(e) => {
            tracing::warn!(dir = %dir.display(), error = %e, "a model folder could not be deleted yet");
            false
        }
    }
}
