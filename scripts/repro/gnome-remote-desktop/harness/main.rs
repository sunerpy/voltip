//! The paste path of the desktop shell, for scripts/repro-gnome-remote-desktop.sh
//! (docs/dictation.md §14.9, issue #235): `voltip_inject::system_paste_injector` with the Linux
//! paste options, as `NativeInjector::system` builds it, run COUNT times in one process — COUNT
//! dictations of one Voltip run. Each paste prints one line: `<label> <n>: <via> <code>`. After each
//! paste the process stays a second (`VOLTIP_REPRO_PAUSE_MS`), as Voltip runs on between dictations:
//! the clipboard is this process's, and the field reads the text from it after the keys arrived.
//!
//! The script builds it twice: against the base revision, and against the fix with the `fixed`
//! feature, which keeps the portal's restore token in the file `VOLTIP_REPRO_TOKEN_FILE` between
//! runs, as the shell keeps it in the system keychain. The token is never printed.
//!
//!   voltip-paste-repro COUNT LABEL

#[cfg(feature = "fixed")]
use std::sync::Arc;

use voltip_inject::{Injector as _, PasteOptions};

/// The restore token in a file, written whole each time (the keychain item of the shell).
#[cfg(feature = "fixed")]
struct FileTokens(std::path::PathBuf);

#[cfg(feature = "fixed")]
impl voltip_inject::RestoreTokens for FileTokens {
    fn load(&self) -> Option<String> {
        std::fs::read_to_string(&self.0).ok().filter(|token| !token.is_empty())
    }

    fn save(&self, token: &str) {
        if let Err(e) = std::fs::write(&self.0, token) {
            eprintln!("voltip-paste-repro: could not save the token: {e}");
        }
    }

    fn clear(&self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let count: u32 = args.next().and_then(|n| n.parse().ok()).unwrap_or(1);
    let label = args.next().unwrap_or_else(|| "paste".to_owned());
    let pause = std::time::Duration::from_millis(std::env::var("VOLTIP_REPRO_PAUSE_MS").ok().and_then(|ms| ms.parse().ok()).unwrap_or(1000));
    let options = PasteOptions::for_os("linux");
    #[cfg(feature = "fixed")]
    let injector = {
        let file = std::env::var_os("VOLTIP_REPRO_TOKEN_FILE").expect("VOLTIP_REPRO_TOKEN_FILE names the token file");
        voltip_inject::system_paste_injector_with(options, Arc::new(FileTokens(file.into())))
    };
    #[cfg(not(feature = "fixed"))]
    let injector = voltip_inject::system_paste_injector(options);
    for n in 1..=count {
        let text = format!("{label} {n}");
        match injector.inject(&format!("{text} ")) {
            Ok(injection) => {
                let code = injection.note.as_ref().map_or_else(|| "-".to_owned(), |note| format!("{:?}", note.code));
                println!("{text}: {} {code}", injection.via);
            }
            Err(e) => println!("{text}: failed {e}"),
        }
        std::thread::sleep(pause);
    }
}
