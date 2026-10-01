//! macOS harness for `.github/scripts/check-keychain-preinstall.sh`: act as the running build,
//! create an identity under the caller's isolated `USER`, and drive the production staged-update
//! hand-over against an already verified `.app.tar.gz`. It never prints secret values.

#[cfg(target_os = "macos")]
fn main() -> std::process::ExitCode {
    match macos::run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("keychain_preinstall: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("keychain_preinstall: macOS only");
}

#[cfg(target_os = "macos")]
mod macos {
    use std::sync::Arc;

    use voltip_identity::{Entries, IdentityManager, PerBuildStore, SecretStore, SecurityKeychain};

    pub fn run() -> Result<(), String> {
        let package = std::env::args_os().nth(1).ok_or("usage: keychain_preinstall <Voltip.app.tar.gz>")?;
        let user = std::env::var("USER").map_err(|e| format!("USER: {e}"))?;
        let build = voltip_desktop_lib::keychain_handoff::cdhash().ok_or("the harness has no cdhash")?;
        let keychain = SecurityKeychain::login().map_err(|e| e.to_string())?;
        let store = Arc::new(PerBuildStore::new(keychain, voltip_desktop_lib::KEYCHAIN_SERVICE, &user, build, Entries::new()));
        let identity = IdentityManager::new(store.clone()).load_or_create("Preinstall Check").map_err(|e| e.to_string())?;
        println!("device {}", identity.device_id);
        let entries = store.handoff_state().ok_or("the store has no hand-over")?;
        let bytes = std::fs::read(package).map_err(|e| e.to_string())?;
        voltip_desktop_lib::keychain_handoff::prepare_update(&bytes, &entries)?;
        println!("prepared {} entries", entries.len());
        Ok(())
    }
}
