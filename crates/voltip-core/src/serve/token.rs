//! The local speech service's bearer token (docs/dictation.md §23.7): 32 random bytes as 64
//! hexadecimal digits in `<data dir>/serve/token`, created with mode 0600 (on Unix) the first time a host
//! starts, kept across restarts, replaced only on request. The headless server and the app share
//! the file, so a client keeps working with whichever of the two runs.

use std::io::Write as _;
use std::path::{Path, PathBuf};

/// Directory of the service's files inside the data directory.
pub const SERVE_DIR: &str = "serve";
/// The token's file name.
pub const TOKEN_FILE: &str = "token";
/// Where uploads are decoded to.
pub const UPLOADS_DIR: &str = "uploads";

/// `<data dir>/serve`.
pub fn serve_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(SERVE_DIR)
}

/// `<data dir>/serve/token`.
pub fn default_token_path(data_dir: &Path) -> PathBuf {
    serve_dir(data_dir).join(TOKEN_FILE)
}

/// `<data dir>/serve/uploads`.
pub fn uploads_dir(data_dir: &Path) -> PathBuf {
    serve_dir(data_dir).join(UPLOADS_DIR)
}

/// A fresh token.
pub fn new_token() -> String {
    hex::encode(voltip_crypto::random_nonce::<32>())
}

/// The token in `path`, created when the file does not exist. The second value is a warning when
/// other users may read the file (Unix).
pub fn load_or_create_token(path: &Path) -> Result<(String, Option<String>), String> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let token = text.trim().to_owned();
            if token.is_empty() {
                return Err(format!("令牌文件为空：{}", path.display()));
            }
            Ok((token, too_open(path)))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let token = new_token();
            write_new(path, &token)?;
            Ok((token, None))
        }
        Err(e) => Err(format!("令牌文件无法读取：{}（{e}）", path.display())),
    }
}

/// Replace the token in `path` with a fresh one and return it (written to a temporary file, then
/// renamed over the old one).
pub fn rotate_token(path: &Path) -> Result<String, String> {
    let token = new_token();
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    write_new(&tmp, &token)?;
    std::fs::rename(&tmp, path).map_err(|e| format!("令牌文件无法写入：{}（{e}）", path.display()))?;
    Ok(token)
}

/// Create `path` (and its directory) holding `token`, readable and writable by this user only.
fn write_new(path: &Path, token: &str) -> Result<(), String> {
    let fail = |e: std::io::Error| format!("令牌文件无法写入：{}（{e}）", path.display());
    if let Some(dir) = path.parent() {
        create_private_dir(dir).map_err(fail)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(fail)?;
    file.write_all(format!("{token}\n").as_bytes()).map_err(fail)?;
    file.sync_all().map_err(fail)
}

/// Create `dir` (and its parents) with mode 0700 on Unix for the directories it creates.
pub fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(dir)
}

#[cfg(unix)]
fn too_open(path: &Path) -> Option<String> {
    use std::os::unix::fs::PermissionsExt as _;
    let mode = std::fs::metadata(path).ok()?.permissions().mode();
    (mode & 0o077 != 0).then(|| format!("令牌文件可被其他用户读取：{}（建议 chmod 600）", path.display()))
}

#[cfg(not(unix))]
fn too_open(_path: &Path) -> Option<String> {
    None
}
