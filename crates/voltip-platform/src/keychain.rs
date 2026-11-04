//! The notice the desktop shell shows when the system keychain is unavailable as it starts
//! (docs/dictation.md §34). Voltip keeps the keys and the device identity there and never falls
//! back to anything weaker, so without it the app says what it needs and quits. The notice
//! appears before the core has read the settings: it follows the system language, which on Linux
//! is the environment's message locale ([`posix_message_language`]).
//!
//! The English texts avoid the words the AppImage catalog's screenshot check takes for an error
//! message (`code/check-screenshot.sh` of AppImage/appimage.github.io, which runs applications in
//! the C locale, where the notice is English).

use crate::HostOs;
use crate::tray::{TrayAction, TrayLocale};

/// What the notice says, in one of the UI's two locales.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeychainNotice {
    /// The dialog's title.
    pub title: &'static str,
    /// What Voltip keeps in the keychain and what to do.
    pub message: &'static str,
    /// The one button; closing the notice in any way quits Voltip.
    pub button: &'static str,
}

impl KeychainNotice {
    /// The notice for `host` in `locale`: Windows Credential Manager, the macOS keychain, and the
    /// Secret Service (GNOME Keyring or KWallet) on Linux and everywhere else. The button is the
    /// tray's 退出 Voltip / Quit Voltip.
    pub const fn new(host: HostOs, locale: TrayLocale) -> Self {
        let (title, message) = match (host, locale) {
            (HostOs::Macos, TrayLocale::ZhCn) => {
                ("Voltip 需要钥匙串", "Voltip 把密钥和设备身份保存在钥匙串中。请解锁钥匙串，并在系统询问时允许 Voltip 访问，然后重新打开 Voltip。")
            }
            (HostOs::Macos, TrayLocale::En) => (
                "Voltip needs the keychain",
                "Voltip keeps your keys and this device's identity in the keychain. Unlock the keychain and allow Voltip to use it when macOS asks, then open Voltip again.",
            ),
            (HostOs::Windows, TrayLocale::ZhCn) => {
                ("Voltip 需要凭据管理器", "Voltip 把密钥和设备身份保存在 Windows 凭据管理器中。请确认凭据管理器服务正在运行，然后重新打开 Voltip。")
            }
            (HostOs::Windows, TrayLocale::En) => (
                "Voltip needs Credential Manager",
                "Voltip keeps your keys and this device's identity in Windows Credential Manager. Make sure the Credential Manager service is running, then open Voltip again.",
            ),
            (HostOs::Linux | HostOs::Other, TrayLocale::ZhCn) => (
                "Voltip 需要系统钥匙串",
                "Voltip 把密钥和设备身份保存在系统钥匙串中。请启动并解锁 GNOME Keyring 或 KWallet（Secret Service），然后重新打开 Voltip。",
            ),
            (HostOs::Linux | HostOs::Other, TrayLocale::En) => (
                "Voltip needs the system keychain",
                "Voltip keeps your keys and this device's identity in the system keychain. Start and unlock GNOME Keyring or KWallet (Secret Service), then open Voltip again.",
            ),
        };
        Self { title, message, button: TrayAction::Quit.label(locale) }
    }
}

/// The language a POSIX environment asks messages in, the way GNU gettext reads it: the locale is
/// `LC_ALL`, else `LC_MESSAGES`, else `LANG` (the first one set to something); in the `C` /
/// `POSIX` locale (`C.UTF-8` included), or with no locale at all, messages stay untranslated and
/// this is `None`, whatever `LANGUAGE` says; otherwise `LANGUAGE`'s first entry, when it has one,
/// comes before the locale's own language. `var` reads one environment variable.
pub fn posix_message_language(var: impl Fn(&str) -> Option<String>) -> Option<String> {
    let set = |name: &str| var(name).map(|value| value.trim().to_owned()).filter(|value| !value.is_empty());
    let locale = set("LC_ALL").or_else(|| set("LC_MESSAGES")).or_else(|| set("LANG"))?;
    let name = locale.split(['.', '@']).next().unwrap_or_default();
    if name == "C" || name == "POSIX" {
        return None;
    }
    let preferred = set("LANGUAGE").and_then(|list| list.split(':').map(str::trim).find(|entry| !entry.is_empty()).map(str::to_owned));
    Some(preferred.unwrap_or(locale))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOSTS: [HostOs; 4] = [HostOs::Linux, HostOs::Macos, HostOs::Windows, HostOs::Other];
    const LOCALES: [TrayLocale; 2] = [TrayLocale::ZhCn, TrayLocale::En];

    /// What a screenshot check reads off the screen: title, text and button on one line.
    fn screen_text(notice: KeychainNotice) -> String {
        format!("{} {} {}", notice.title, notice.message, notice.button)
    }

    /// Each host names its own keychain, in both languages: the Secret Service with GNOME Keyring
    /// and KWallet on Linux (and on the BSDs, `Other`), the keychain on macOS, Credential Manager
    /// on Windows. Its one button reads like the tray's 退出 Voltip / Quit Voltip.
    #[test]
    fn each_host_names_its_keychain_in_both_languages() {
        for locale in LOCALES {
            let linux = KeychainNotice::new(HostOs::Linux, locale);
            for name in ["Secret Service", "GNOME Keyring", "KWallet"] {
                assert!(linux.message.contains(name), "{locale:?}: {name} in {:?}", linux.message);
            }
            assert_eq!(KeychainNotice::new(HostOs::Other, locale), linux);
            for host in HOSTS {
                let notice = KeychainNotice::new(host, locale);
                assert!(!notice.title.is_empty() && notice.message.contains("Voltip"), "{host:?} {locale:?}: {notice:?}");
                assert_eq!(notice.button, TrayAction::Quit.label(locale), "{host:?} {locale:?}");
            }
        }
        let zh = |host| screen_text(KeychainNotice::new(host, TrayLocale::ZhCn));
        let en = |host| screen_text(KeychainNotice::new(host, TrayLocale::En));
        assert!(zh(HostOs::Linux).contains("系统钥匙串") && en(HostOs::Linux).contains("system keychain"));
        assert!(zh(HostOs::Macos).contains("钥匙串") && en(HostOs::Macos).contains("keychain"));
        assert!(zh(HostOs::Windows).contains("凭据管理器") && en(HostOs::Windows).contains("Credential Manager"));
        for other in ["Secret Service", "凭据管理器", "Credential Manager"] {
            assert!(!zh(HostOs::Macos).contains(other) && !en(HostOs::Macos).contains(other), "macOS names {other}");
        }
        for other in ["Secret Service", "钥匙串", "keychain"] {
            assert!(!zh(HostOs::Windows).contains(other) && !en(HostOs::Windows).contains(other), "Windows names {other}");
        }
    }

    /// The words AppImage/appimage.github.io's `code/check-screenshot.sh` takes for an error
    /// message in a screenshot (its HARD list fails the test, its SOFT list warns; the alternations
    /// widened to their stems). The catalog runs applications in the C locale, where the notice is
    /// English, and it fails a screenshot whose text is mostly not English.
    #[test]
    fn regression_the_english_notice_has_no_word_the_appimage_catalog_reads_as_an_error() {
        const HARD: [&str; 16] = [
            "traceback",
            "exception",
            "segmentation fault",
            "fatal",
            "error while loading",
            "glibc",
            "not installed",
            "cannot open display",
            "permission denied",
            "no such file",
            "could not",
            "failed to",
            "cannot",
            "unable to",
            "command not found",
            "core dumped",
        ];
        const SOFT: [&str; 7] = ["error", "failed", "failure", "could not", "cannot", "unable to", "not found"];
        for host in HOSTS {
            let text = screen_text(KeychainNotice::new(host, TrayLocale::En));
            let lower = text.to_lowercase();
            for word in HARD.iter().chain(SOFT.iter()) {
                assert!(!lower.contains(word), "{host:?}: {word:?} in {text:?}");
            }
            assert!(text.is_ascii(), "{host:?}: the English notice is English: {text:?}");
            assert!(!KeychainNotice::new(host, TrayLocale::ZhCn).message.is_ascii(), "{host:?}: the Chinese notice is Chinese");
        }
    }

    /// The register of the interface's dictionaries (`packages/shared/src/i18n/copy.test.ts`):
    /// no internal words, nothing colloquial.
    #[test]
    fn the_notice_reads_like_the_interface() {
        for host in HOSTS {
            let zh = screen_text(KeychainNotice::new(host, TrayLocale::ZhCn));
            for word in ["核心", "后端", "还没", "没能", "免得", "搭的", "咋", "啥", "说说", "试试", "看看", "读不到"] {
                assert!(!zh.contains(word), "{host:?}: {word} in {zh}");
            }
            let en = screen_text(KeychainNotice::new(host, TrayLocale::En)).to_lowercase();
            for word in [" core", "backend", " just ", "gonna", "stuff", "something"] {
                assert!(!en.contains(word), "{host:?}: {word:?} in {en}");
            }
        }
    }

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| vars.iter().find(|(key, _)| *key == name).map(|(_, value)| (*value).to_owned())
    }

    /// GNU gettext's rule: `LC_ALL`, else `LC_MESSAGES`, else `LANG` names the locale; the `C` /
    /// `POSIX` locale (the AppImage catalog's test) and no locale at all leave messages
    /// untranslated, whatever `LANGUAGE` says; otherwise `LANGUAGE`'s first entry comes first.
    #[test]
    fn the_message_language_is_read_the_way_gettext_reads_it() {
        let untranslated: [&[(&str, &str)]; 9] = [
            &[],
            &[("LANG", "C")],
            &[("LANG", "C.UTF-8")],
            &[("LANG", "C.utf8")],
            &[("LC_ALL", "POSIX")],
            &[("LANG", " ")],
            &[("LANG", "zh_CN.UTF-8"), ("LC_ALL", "C.UTF-8")],
            &[("LANG", "C.UTF-8"), ("LANGUAGE", "zh_CN:zh")],
            &[("LANGUAGE", "zh_CN")],
        ];
        for vars in untranslated {
            assert_eq!(posix_message_language(env(vars)), None, "{vars:?}");
        }
        let cases: [(&[(&str, &str)], &str); 6] = [
            (&[("LANG", "zh_CN.UTF-8")], "zh_CN.UTF-8"),
            (&[("LANG", "en_US.UTF-8")], "en_US.UTF-8"),
            (&[("LANG", "en_US.UTF-8"), ("LC_MESSAGES", "zh_CN.UTF-8")], "zh_CN.UTF-8"),
            (&[("LANG", "zh_CN.UTF-8"), ("LC_MESSAGES", "zh_CN.UTF-8"), ("LC_ALL", "de_DE.UTF-8")], "de_DE.UTF-8"),
            (&[("LANG", "en_US.UTF-8"), ("LANGUAGE", ":zh_CN:en")], "zh_CN"),
            (&[("LANG", "zh_CN.UTF-8"), ("LANGUAGE", "")], "zh_CN.UTF-8"),
        ];
        for (vars, language) in cases {
            assert_eq!(posix_message_language(env(vars)).as_deref(), Some(language), "{vars:?}");
        }
        assert_eq!(TrayLocale::for_language("zh_CN.UTF-8"), TrayLocale::ZhCn);
        assert_eq!(TrayLocale::for_language("de_DE.UTF-8"), TrayLocale::En);
    }
}
