//! The relay's policy (docs/pairing.md 「最低版本」): the lowest client version it serves in full,
//! and the release versions its statistics name.
//!
//! Two files hold it: the policy the admin console's sync writes, and a local override an operator
//! writes by hand. [`PolicyFiles::poll`] reads them by one rule, shared with the built-in service's
//! gateway: a valid override is used whole; an invalid one is ignored with a warning; a valid main
//! file is used and remembered; an invalid or missing one leaves the last valid policy in force;
//! with none ever valid the relay serves every version.

use std::cmp::Ordering;
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::SystemTime;

use serde::Deserialize;

/// The `schema` this relay reads.
pub const POLICY_SCHEMA: u32 = 1;
/// Most release versions a policy may name for the statistics.
pub const MAX_KNOWN_VERSIONS: usize = 64;

/// A client release, `x.y.z`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct ClientVersion {
    major: u32,
    minor: u32,
    patch: u32,
}

impl ClientVersion {
    /// `major.minor.patch`.
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self { major, minor, patch }
    }

    /// The version in a `hello.client_version` (`voltip/0.1.10`, or the bare `0.1.10`); `None`
    /// when it names none, which the relay serves as it is.
    pub fn of_client(client_version: &str) -> Option<Self> {
        let text = client_version.trim();
        text.strip_prefix("voltip/").unwrap_or(text).parse().ok()
    }
}

impl FromStr for ClientVersion {
    type Err = PolicyError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let invalid = || PolicyError::Version(text.to_owned());
        let mut parts = text.split('.');
        let mut next = || -> Result<u32, PolicyError> {
            let part = parts.next().ok_or_else(invalid)?;
            if part.is_empty() || part.len() > 9 || !part.bytes().all(|b| b.is_ascii_digit()) {
                return Err(invalid());
            }
            part.parse().map_err(|_| invalid())
        };
        let version = Self { major: next()?, minor: next()?, patch: next()? };
        if parts.next().is_some() {
            return Err(invalid());
        }
        Ok(version)
    }
}

impl fmt::Display for ClientVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Why a policy file was not used.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    /// It could not be read (other than missing).
    #[error("cannot read: {0}")]
    Read(String),
    /// It is not the JSON a policy is.
    #[error("not a policy: {0}")]
    Malformed(String),
    /// Another schema than [`POLICY_SCHEMA`].
    #[error("schema {0} is not {POLICY_SCHEMA}")]
    Schema(u32),
    /// A version that is not `x.y.z`.
    #[error("not a version: {0:?}")]
    Version(String),
    /// More than [`MAX_KNOWN_VERSIONS`] release versions.
    #[error("{0} known versions, more than {MAX_KNOWN_VERSIONS}")]
    TooManyVersions(usize),
}

/// What the relay enforces and how it names versions in its statistics.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RelayPolicy {
    /// Clients below it may not pair or reconnect: those that know `client_too_old` are told so
    /// and disconnected, older ones get `unsupported_version` for every pairing and `attach`.
    /// `None`: every version is served.
    pub min_version: Option<ClientVersion>,
    /// The release versions the statistics count by name; any other version counts as `other`.
    /// Empty: the first 32 versions of each hour are named.
    pub known_versions: Vec<ClientVersion>,
}

#[derive(Deserialize)]
struct PolicyFile {
    schema: u32,
    #[serde(default)]
    min_version: Option<String>,
    #[serde(default)]
    known_versions: Vec<String>,
}

impl RelayPolicy {
    /// Parse a policy file's text.
    pub fn parse(text: &str) -> Result<Self, PolicyError> {
        let file: PolicyFile = serde_json::from_str(text).map_err(|e| PolicyError::Malformed(e.to_string()))?;
        if file.schema != POLICY_SCHEMA {
            return Err(PolicyError::Schema(file.schema));
        }
        if file.known_versions.len() > MAX_KNOWN_VERSIONS {
            return Err(PolicyError::TooManyVersions(file.known_versions.len()));
        }
        let min_version = file.min_version.as_deref().map(str::parse).transpose()?;
        let mut known_versions = file.known_versions.iter().map(|v| v.parse()).collect::<Result<Vec<ClientVersion>, _>>()?;
        known_versions.sort();
        known_versions.dedup();
        Ok(Self { min_version, known_versions })
    }

    /// Whether a client of `version` is below the minimum (an unknown version never is).
    pub fn is_too_old(&self, version: Option<ClientVersion>) -> bool {
        matches!((self.min_version, version), (Some(min), Some(v)) if v.cmp(&min) == Ordering::Less)
    }
}

/// What a policy file looked like when it was last read: whether it was there and when it changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
struct Seen {
    present: bool,
    modified: Option<SystemTime>,
}

fn seen(path: &Path) -> Seen {
    match std::fs::metadata(path) {
        Ok(meta) => Seen { present: true, modified: meta.modified().ok() },
        Err(_) => Seen::default(),
    }
}

/// The two policy files and what they last held.
#[derive(Debug)]
pub struct PolicyFiles {
    main: PathBuf,
    override_path: Option<PathBuf>,
    seen: Option<(Seen, Seen)>,
    last_valid: Option<RelayPolicy>,
    current: RelayPolicy,
}

impl PolicyFiles {
    /// The policy at `main`, overridden by the one at `override_path` while it exists and is valid.
    pub fn new(main: impl Into<PathBuf>, override_path: Option<PathBuf>) -> Self {
        Self { main: main.into(), override_path, seen: None, last_valid: None, current: RelayPolicy::default() }
    }

    /// The policy in force (as of the last [`PolicyFiles::poll`]).
    pub fn current(&self) -> &RelayPolicy {
        &self.current
    }

    /// Read the files again when either changed since the last look (it appeared, went away or
    /// was modified); the first call always reads them. Returns the policy in force when it changed.
    pub fn poll(&mut self) -> Option<RelayPolicy> {
        let now = (seen(&self.main), self.override_path.as_deref().map(seen).unwrap_or_default());
        if self.seen == Some(now) {
            return None;
        }
        self.seen = Some(now);
        let next = self.read();
        if next == self.current {
            return None;
        }
        tracing::info!(min_version = ?next.min_version.map(|v| v.to_string()), known = next.known_versions.len(), "relay policy changed");
        self.current = next.clone();
        Some(next)
    }

    fn read(&mut self) -> RelayPolicy {
        if let Some(path) = &self.override_path {
            match read_file(path) {
                Ok(Some(policy)) => return policy,
                Ok(None) => {}
                Err(e) => tracing::warn!(path = %path.display(), error = %e, "the policy override is not valid; ignoring it"),
            }
        }
        match read_file(&self.main) {
            Ok(Some(policy)) => {
                self.last_valid = Some(policy.clone());
                policy
            }
            Ok(None) => self.last_valid.clone().unwrap_or_default(),
            Err(e) => {
                tracing::warn!(path = %self.main.display(), error = %e, "the relay policy is not valid; keeping the last valid one");
                self.last_valid.clone().unwrap_or_default()
            }
        }
    }
}

/// `Ok(None)` when the file does not exist.
fn read_file(path: &Path) -> Result<Option<RelayPolicy>, PolicyError> {
    match std::fs::read_to_string(path) {
        Ok(text) => RelayPolicy::parse(&text).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(PolicyError::Read(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const V0110: ClientVersion = ClientVersion::new(0, 1, 10);

    #[test]
    fn versions_parse_from_the_hello_and_compare_numerically() {
        assert_eq!(ClientVersion::of_client("voltip/0.1.10"), Some(V0110));
        assert_eq!(ClientVersion::of_client("0.1.10"), Some(V0110));
        assert!(ClientVersion::new(0, 1, 9) < V0110, "numeric, not lexical");
        assert!(ClientVersion::new(0, 2, 0) > V0110);
        for not_one in ["test", "voltip/", "voltip/0.1", "voltip/0.1.10.1", "voltip/0.1.x", "voltip/-1.0.0", "voltip/0.1.10-beta", "voltip/01234567890.0.0"] {
            assert_eq!(ClientVersion::of_client(not_one), None, "{not_one}");
        }
        assert_eq!(V0110.to_string(), "0.1.10");
    }

    #[test]
    fn a_policy_parses_and_only_a_known_older_version_is_too_old() {
        let policy = RelayPolicy::parse(r#"{"schema":1,"min_version":"0.1.10","known_versions":["0.1.9","0.1.10","0.1.9"]}"#).unwrap();
        assert_eq!(policy.min_version, Some(V0110));
        assert_eq!(policy.known_versions, [ClientVersion::new(0, 1, 9), V0110], "sorted, without duplicates");
        assert!(policy.is_too_old(Some(ClientVersion::new(0, 1, 9))));
        assert!(!policy.is_too_old(Some(V0110)));
        assert!(!policy.is_too_old(None), "a version the relay cannot read is served");
        assert!(!RelayPolicy::default().is_too_old(Some(ClientVersion::new(0, 0, 1))));
        assert_eq!(RelayPolicy::parse(r#"{"schema":1,"min_version":null}"#).unwrap(), RelayPolicy::default());
        assert_eq!(RelayPolicy::parse(r#"{"schema":2}"#), Err(PolicyError::Schema(2)));
        assert!(matches!(RelayPolicy::parse(r#"{"schema":1,"min_version":"0.2"}"#), Err(PolicyError::Version(_))));
        assert!(matches!(RelayPolicy::parse("[]"), Err(PolicyError::Malformed(_))));
        let many = (0..=MAX_KNOWN_VERSIONS).map(|i| format!("\"0.0.{i}\"")).collect::<Vec<_>>().join(",");
        assert_eq!(RelayPolicy::parse(&format!(r#"{{"schema":1,"known_versions":[{many}]}}"#)), Err(PolicyError::TooManyVersions(MAX_KNOWN_VERSIONS + 1)));
    }

    fn write(path: &Path, text: &str) {
        // A new modification time on every write, whatever the file system's resolution.
        let before = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());
        std::fs::write(path, text).unwrap();
        if let Some(before) = before {
            let file = std::fs::File::options().write(true).open(path).unwrap();
            file.set_modified(before + std::time::Duration::from_secs(1)).unwrap();
        }
    }

    /// The rule the relay and the gateway share (docs/pairing.md 「最低版本」).
    #[test]
    fn the_override_wins_while_valid_and_a_broken_main_file_keeps_the_last_valid_policy() {
        let dir = tempfile::tempdir().unwrap();
        let (main, over) = (dir.path().join("relay-policy.json"), dir.path().join("relay-policy.override.json"));
        let mut files = PolicyFiles::new(&main, Some(over.clone()));
        assert_eq!(files.poll(), None, "no file: every version is served, which is no change");
        assert_eq!(files.current(), &RelayPolicy::default());

        write(&main, r#"{"schema":1,"min_version":"0.1.10"}"#);
        assert_eq!(files.poll().and_then(|p| p.min_version), Some(V0110));
        assert_eq!(files.poll(), None, "unchanged files are not read again");

        write(&over, r#"{"schema":1,"min_version":null}"#);
        assert_eq!(files.poll(), Some(RelayPolicy::default()), "the override wins whole");
        write(&over, "{ not json");
        assert_eq!(files.poll().and_then(|p| p.min_version), Some(V0110), "an invalid override is ignored");
        std::fs::remove_file(&over).unwrap();
        assert_eq!(files.poll(), None, "back to the main file, which is what is in force");

        write(&main, r#"{"schema":9}"#);
        assert_eq!(files.poll(), None, "a broken main file keeps the last valid policy");
        assert_eq!(files.current().min_version, Some(V0110));
        std::fs::remove_file(&main).unwrap();
        assert_eq!(files.poll(), None);
        assert_eq!(files.current().min_version, Some(V0110), "so does a missing one");

        let mut fresh = PolicyFiles::new(dir.path().join("never.json"), None);
        std::fs::write(dir.path().join("never.json"), "{}").unwrap();
        assert_eq!(fresh.poll(), None);
        assert_eq!(fresh.current(), &RelayPolicy::default(), "never valid: every version is served");
    }
}
