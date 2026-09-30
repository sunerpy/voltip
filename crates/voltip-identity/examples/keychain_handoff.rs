//! Harness of `.github/scripts/check-keychain-handoff.sh` (macOS, GitHub's runners): it stands in
//! for the build an in-app update replaces. It keeps a device identity in items of its own in the
//! login keychain, then starts the app the way an old build does once an update is installed,
//! handing the identity over ([`voltip_identity::handoff::start`]).
//!
//! ```text
//! keychain_handoff create <cdhash>                load or create the identity; prints its device id
//! keychain_handoff handoff <cdhash> <app binary>  start the app with the identity handed over
//! ```
//!
//! `<cdhash>` is this binary's own (`codesign -dvvv`). The script signs it with the app's identifier
//! and certificate, so the app's check of its parent passes. The harness does not check the app it
//! starts: the check under test is the app's.

#[cfg(target_os = "macos")]
fn main() -> std::process::ExitCode {
    match macos::run(&std::env::args().skip(1).collect::<Vec<_>>()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("keychain_handoff: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("keychain_handoff: macOS only");
}

#[cfg(target_os = "macos")]
mod macos {
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    use voltip_identity::handoff::{self, PeerCheck};
    use voltip_identity::{Entries, IdentityManager, PerBuildStore, SecretStore, SecurityKeychain};

    /// The app's keychain service (`KEYCHAIN_SERVICE` of the desktop shell).
    const SERVICE: &str = "dev.voltip.desktop";

    struct AnyChild;

    impl PeerCheck for AnyChild {
        fn trusted(&self, _pid: u32) -> bool {
            true
        }
    }

    fn store(build: &str) -> Result<Arc<PerBuildStore<SecurityKeychain>>, String> {
        let user = std::env::var("USER").map_err(|e| format!("USER: {e}"))?;
        let keychain = SecurityKeychain::login().map_err(|e| e.to_string())?;
        Ok(Arc::new(PerBuildStore::new(keychain, SERVICE, user, build, Entries::new())))
    }

    pub fn run(args: &[String]) -> Result<(), String> {
        match args {
            [cmd, build] if cmd == "create" => {
                let store = store(build)?;
                let identity = IdentityManager::new(store).load_or_create("Handoff Check").map_err(|e| e.to_string())?;
                println!("device {}", identity.device_id);
                Ok(())
            }
            [cmd, build, app] if cmd == "handoff" => {
                let store = store(build)?;
                let identity = IdentityManager::new(store.clone()).load().map_err(|e| e.to_string())?.ok_or("no identity to hand over")?;
                println!("device {}", identity.device_id);
                let entries = store.handoff_state().ok_or("the store has no hand-over")?;
                handoff::start(Path::new(app), Vec::new(), &entries, &AnyChild, Duration::from_secs(10)).map_err(|e| e.to_string())?;
                println!("handed over {} entries", entries.len());
                Ok(())
            }
            _ => Err("usage: keychain_handoff create <cdhash> | handoff <cdhash> <app binary>".into()),
        }
    }
}
