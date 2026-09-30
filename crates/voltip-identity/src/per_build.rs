//! A signed macOS build keeps its secrets in keychain items it created itself, one account per
//! build (docs/runbook.md 发布 · macOS 签名与钥匙串).
//!
//! Why (user report 2026-09-30, 0.0.12 → 0.0.14 still asked): the login keychain gives every item
//! a partition list, and code signed with a self-signed certificate (no Apple Team ID) gets the
//! partition `cdhash:<that build>`. Reading an item another build created shows the system's
//! dialog, and 「始终允许」 lets in only the build it was pressed for, so every update asked again.
//! Here each build stores its items under `<user>.signed.<its cdhash>`, and an in-app update hands
//! the values over ([`crate::handoff`]): the new build creates its own items from them without
//! reading an older one. Without a hand-over (a hand install, or the first update from a build
//! without this) the newest older item is read once, which asks once, and moved over.

use std::collections::HashMap;

use parking_lot::Mutex;
use zeroize::Zeroizing;

use crate::IdentityError;
use crate::secret_store::{SIGNED_ACCOUNT_SUFFIX, SecretStore, Slot, read_moving};

/// What one keychain store knew in this process: each entry's value, or `None` for an entry known
/// to be absent (a hand-over carries that too, so an older item is not brought back).
pub type Entries = Vec<(String, Option<Zeroizing<Vec<u8>>>)>;

/// Whether a read may show the system's keychain dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    /// The user may be asked (an older build's item, read once).
    Allowed,
    /// Never ask: an item that would need the dialog reads as [`Read::WouldAsk`].
    Never,
}

/// What reading one item found.
#[derive(Debug)]
pub enum Read {
    /// The value.
    Found(Zeroizing<Vec<u8>>),
    /// No such item.
    Missing,
    /// Another build created the item: reading it needs the dialog.
    WouldAsk,
}

/// One item of an entry, as a listing shows it (never with its value).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stored {
    /// The item's account.
    pub account: String,
    /// Creation time as text that sorts in time order (the keychain's
    /// `2026-09-30 13:57:47 +0000`): larger is newer.
    pub created: String,
}

/// The keychain operations [`PerBuildStore`] needs. `service` is the item's service attribute
/// (`dev.voltip.desktop/voltip.identity.x25519`).
pub trait Keychain: Send + Sync {
    /// The items of `service`, without their values (listing never asks).
    fn items(&self, service: &str) -> Result<Vec<Stored>, IdentityError>;
    /// Read one item.
    fn read(&self, service: &str, account: &str, ask: Ask) -> Result<Read, IdentityError>;
    /// Create or overwrite an item of this build's.
    fn write(&self, service: &str, account: &str, value: &[u8]) -> Result<(), IdentityError>;
    /// Remove an item without reading it (never asks); a missing one is not an error.
    fn remove(&self, service: &str, account: &str) -> Result<(), IdentityError>;
}

/// Secrets in items this build created, under `<user>.signed.<build>`; see the module docs.
pub struct PerBuildStore<K> {
    keychain: K,
    service: String,
    user: String,
    build: String,
    /// What the build before this one handed over, until each entry is first read.
    handed: Mutex<HashMap<String, Option<Zeroizing<Vec<u8>>>>>,
    /// Each entry's state as this process last read, wrote or removed it.
    known: Mutex<HashMap<String, Option<Zeroizing<Vec<u8>>>>>,
}

impl<K> std::fmt::Debug for PerBuildStore<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PerBuildStore").field("service", &self.service).field("build", &self.build).finish_non_exhaustive()
    }
}

impl<K: Keychain> PerBuildStore<K> {
    /// A store on `keychain` for `service` (`dev.voltip.desktop`) and `user`, as build `build` (its
    /// cdhash), starting from what the previous build handed over (empty when it handed nothing).
    pub fn new(keychain: K, service: impl Into<String>, user: impl Into<String>, build: impl Into<String>, handed: Entries) -> Self {
        Self {
            keychain,
            service: service.into(),
            user: user.into(),
            build: build.into(),
            handed: Mutex::new(handed.into_iter().collect()),
            known: Mutex::new(HashMap::new()),
        }
    }

    /// What this process knows, for the next build: every entry it read, wrote or removed, and
    /// whatever it was handed and never read.
    pub fn known(&self) -> Entries {
        let mut all = self.handed.lock().clone();
        all.extend(self.known.lock().iter().map(|(k, v)| (k.clone(), v.clone())));
        let mut entries: Entries = all.into_iter().collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries
    }

    fn item_service(&self, entry: &str) -> String {
        format!("{}/{entry}", self.service)
    }

    fn own_account(&self) -> String {
        format!("{}{SIGNED_ACCOUNT_SUFFIX}.{}", self.user, self.build)
    }

    fn slot(&self, entry: &str, account: String, ask: Ask) -> KeychainSlot<'_, K> {
        KeychainSlot { keychain: &self.keychain, service: self.item_service(entry), account, ask }
    }

    /// Every other copy of `entry`: other builds' items, newest first, then the item 0.0.12–0.0.14
    /// used (`<user>.signed`) and the one before (`<user>`).
    fn others(&self, entry: &str) -> Vec<String> {
        let prefix = format!("{}{SIGNED_ACCOUNT_SUFFIX}.", self.user);
        let own = self.own_account();
        let mut builds: Vec<Stored> = match self.keychain.items(&self.item_service(entry)) {
            Ok(items) => items.into_iter().filter(|s| s.account.starts_with(&prefix) && s.account != own).collect(),
            Err(e) => {
                tracing::warn!(error = %e, entry, "keychain items could not be listed; looking at the older accounts only");
                Vec::new()
            }
        };
        builds.sort_by(|a, b| b.created.cmp(&a.created));
        let mut accounts: Vec<String> = builds.into_iter().map(|s| s.account).collect();
        accounts.push(format!("{}{SIGNED_ACCOUNT_SUFFIX}", self.user));
        accounts.push(self.user.clone());
        accounts
    }

    /// Remove every other copy of `entry`, never asking; one that cannot go stays, unused.
    fn remove_others(&self, entry: &str) {
        for account in self.others(entry) {
            if let Err(e) = self.keychain.remove(&self.item_service(entry), &account) {
                tracing::warn!(error = %e, entry, "an older keychain item could not be removed; it stays, unused");
            }
        }
    }

    fn load(&self, entry: &str) -> Result<Option<Zeroizing<Vec<u8>>>, IdentityError> {
        let own = self.slot(entry, self.own_account(), Ask::Never);
        if let Some(value) = own.read()? {
            return Ok(Some(value));
        }
        if let Some(handed) = self.handed.lock().remove(entry) {
            match &handed {
                Some(value) => match own.write(value).and_then(|()| own.read()) {
                    Ok(Some(back)) if back.as_slice() == value.as_slice() => {
                        tracing::info!(entry, "keychain item stored from the update's hand-over");
                        self.remove_others(entry);
                    }
                    outcome => {
                        tracing::warn!(stored = outcome.is_ok(), entry, "the hand-over could not be stored; the older items stay");
                        let _ = own.remove();
                    }
                },
                None => self.remove_others(entry),
            }
            return Ok(handed);
        }
        for account in self.others(entry) {
            let older = self.slot(entry, account, Ask::Allowed);
            if let Some(value) = read_moving(&own, &older)? {
                self.remove_others(entry);
                return Ok(Some(value));
            }
        }
        Ok(None)
    }
}

impl<K: Keychain> SecretStore for PerBuildStore<K> {
    fn get(&self, entry: &str) -> Result<Option<Zeroizing<Vec<u8>>>, IdentityError> {
        if let Some(known) = self.known.lock().get(entry) {
            return Ok(known.clone());
        }
        let value = self.load(entry)?;
        self.known.lock().insert(entry.to_owned(), value.clone());
        Ok(value)
    }

    fn set(&self, entry: &str, value: &[u8]) -> Result<(), IdentityError> {
        self.slot(entry, self.own_account(), Ask::Never).write(value)?;
        self.handed.lock().remove(entry);
        self.known.lock().insert(entry.to_owned(), Some(Zeroizing::new(value.to_vec())));
        self.remove_others(entry);
        Ok(())
    }

    /// Every copy goes, older builds' too, or the entry would come back from one of them.
    fn delete(&self, entry: &str) -> Result<(), IdentityError> {
        self.keychain.remove(&self.item_service(entry), &self.own_account())?;
        for account in self.others(entry) {
            self.keychain.remove(&self.item_service(entry), &account)?;
        }
        self.handed.lock().remove(entry);
        self.known.lock().insert(entry.to_owned(), None);
        Ok(())
    }

    fn backend_name(&self) -> &'static str {
        "keychain"
    }

    fn handoff_state(&self) -> Option<Entries> {
        Some(self.known())
    }
}

/// One item of a [`Keychain`] as a [`Slot`] for [`read_moving`].
struct KeychainSlot<'a, K> {
    keychain: &'a K,
    service: String,
    account: String,
    ask: Ask,
}

impl<K: Keychain> Slot for KeychainSlot<'_, K> {
    fn read(&self) -> Result<Option<Zeroizing<Vec<u8>>>, IdentityError> {
        match self.keychain.read(&self.service, &self.account, self.ask)? {
            Read::Found(value) => Ok(Some(value)),
            Read::Missing => Ok(None),
            Read::WouldAsk => {
                Err(IdentityError::StoreUnavailable(format!("the keychain item {} ({}) needs the user's permission", self.service, self.account)))
            }
        }
    }

    fn write(&self, value: &[u8]) -> Result<(), IdentityError> {
        self.keychain.write(&self.service, &self.account, value)
    }

    fn remove(&self) -> Result<(), IdentityError> {
        self.keychain.remove(&self.service, &self.account)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::Arc;

    use super::*;
    use crate::SECRET_KEY_ENTRY;

    const META: &str = "voltip.identity.meta";
    const SVC: &str = "dev.voltip.desktop";

    struct FakeItem {
        value: Vec<u8>,
        /// Builds in the item's partition list: its creator, and any the user always allowed.
        partition: BTreeSet<String>,
        created: String,
    }

    /// A login keychain as macOS keeps it: an item another build created is read only by asking
    /// (counted in `asked`; the user answers 「允许」, which adds nothing), listing and removing
    /// never ask.
    #[derive(Clone, Default)]
    struct Login {
        items: Arc<Mutex<BTreeMap<(String, String), FakeItem>>>,
        clock: Arc<Mutex<u64>>,
        asked: Arc<Mutex<Vec<String>>>,
    }

    impl Login {
        fn as_build(&self, build: &str) -> Fake {
            Fake { login: self.clone(), build: build.to_owned(), fail_write: false }
        }
        fn accounts(&self, entry: &str) -> Vec<String> {
            let service = format!("{SVC}/{entry}");
            self.items.lock().keys().filter(|(s, _)| *s == service).map(|(_, a)| a.clone()).collect()
        }
        fn asked(&self) -> Vec<String> {
            self.asked.lock().clone()
        }
        /// An item build `build` stored before (as an older release did).
        fn put(&self, build: &str, entry: &str, account: &str, value: &[u8]) {
            self.as_build(build).write(&format!("{SVC}/{entry}"), account, value).unwrap();
        }
    }

    struct Fake {
        login: Login,
        build: String,
        fail_write: bool,
    }

    impl Keychain for Fake {
        fn items(&self, service: &str) -> Result<Vec<Stored>, IdentityError> {
            let items = self.login.items.lock();
            Ok(items.iter().filter(|((s, _), _)| s == service).map(|((_, a), item)| Stored { account: a.clone(), created: item.created.clone() }).collect())
        }
        fn read(&self, service: &str, account: &str, ask: Ask) -> Result<Read, IdentityError> {
            let items = self.login.items.lock();
            let Some(item) = items.get(&(service.to_owned(), account.to_owned())) else { return Ok(Read::Missing) };
            if item.partition.contains(&self.build) {
                return Ok(Read::Found(Zeroizing::new(item.value.clone())));
            }
            match ask {
                Ask::Never => Ok(Read::WouldAsk),
                Ask::Allowed => {
                    self.login.asked.lock().push(format!("{service} {account}"));
                    Ok(Read::Found(Zeroizing::new(item.value.clone())))
                }
            }
        }
        fn write(&self, service: &str, account: &str, value: &[u8]) -> Result<(), IdentityError> {
            if self.fail_write {
                return Err(IdentityError::StoreUnavailable("keychain locked".into()));
            }
            let mut items = self.login.items.lock();
            let key = (service.to_owned(), account.to_owned());
            if let Some(item) = items.get_mut(&key) {
                if !item.partition.contains(&self.build) {
                    return Err(IdentityError::StoreUnavailable("another build's item".into()));
                }
                item.value = value.to_vec();
                return Ok(());
            }
            let mut clock = self.login.clock.lock();
            *clock += 1;
            items.insert(key, FakeItem { value: value.to_vec(), partition: BTreeSet::from([self.build.clone()]), created: format!("{:020}", *clock) });
            Ok(())
        }
        fn remove(&self, service: &str, account: &str) -> Result<(), IdentityError> {
            self.login.items.lock().remove(&(service.to_owned(), account.to_owned()));
            Ok(())
        }
    }

    fn store(login: &Login, build: &str, handed: Entries) -> PerBuildStore<Fake> {
        PerBuildStore::new(login.as_build(build), SVC, "mac", build, handed)
    }

    fn read(store: &PerBuildStore<Fake>, entry: &str) -> Option<Vec<u8>> {
        store.get(entry).unwrap().map(|v| v.to_vec())
    }

    /// Regression (user report 2026-09-30: 0.0.12 → 0.0.14 asked again for
    /// voltip.identity.x25519 and .meta): the build an in-app update starts reads nothing another
    /// build created. It stores what the old build handed over in items of its own, removes the old
    /// build's, and later starts read its own items without asking either.
    #[test]
    fn regression_an_in_app_update_reads_the_keychain_without_asking() {
        let login = Login::default();
        let old = store(&login, "aaaa", Entries::new());
        old.set(SECRET_KEY_ENTRY, b"key").unwrap();
        old.set(META, b"meta").unwrap();
        let handed = old.handoff_state().unwrap();

        let new = store(&login, "bbbb", handed);
        assert_eq!(read(&new, SECRET_KEY_ENTRY).as_deref(), Some(&b"key"[..]));
        assert_eq!(read(&new, META).as_deref(), Some(&b"meta"[..]));
        assert_eq!(login.asked(), Vec::<String>::new(), "nothing asked");
        assert_eq!(login.accounts(SECRET_KEY_ENTRY), ["mac.signed.bbbb"], "the old build's item is gone");

        let again = store(&login, "bbbb", Entries::new());
        assert_eq!(read(&again, SECRET_KEY_ENTRY).as_deref(), Some(&b"key"[..]));
        assert_eq!(login.asked(), Vec::<String>::new());
    }

    /// Without a hand-over (a hand install) the newest older item is read once, which asks once per
    /// entry, and moved to an item of this build's: the next start asks nothing.
    #[test]
    fn a_hand_install_asks_once_per_entry_and_then_never() {
        let login = Login::default();
        let old = store(&login, "aaaa", Entries::new());
        old.set(SECRET_KEY_ENTRY, b"key").unwrap();
        old.set(META, b"meta").unwrap();

        let new = store(&login, "cccc", Entries::new());
        assert_eq!(read(&new, SECRET_KEY_ENTRY).as_deref(), Some(&b"key"[..]));
        assert_eq!(read(&new, META).as_deref(), Some(&b"meta"[..]));
        assert_eq!(login.asked().len(), 2, "{:?}", login.asked());
        assert_eq!(login.accounts(META), ["mac.signed.cccc"]);
        let again = store(&login, "cccc", Entries::new());
        assert_eq!(read(&again, META).as_deref(), Some(&b"meta"[..]));
        assert_eq!(login.asked().len(), 2, "no more asking");
    }

    /// The first update into this scheme: 0.0.12–0.0.14 kept the items under `<user>.signed` (and
    /// earlier builds under `<user>`). They are read once and moved over.
    #[test]
    fn the_items_older_releases_kept_are_moved_once() {
        let login = Login::default();
        login.put("0.0.12", SECRET_KEY_ENTRY, "mac.signed", b"key");
        login.put("0.0.6", META, "mac", b"meta");
        let new = store(&login, "bbbb", Entries::new());
        assert_eq!(read(&new, SECRET_KEY_ENTRY).as_deref(), Some(&b"key"[..]));
        assert_eq!(read(&new, META).as_deref(), Some(&b"meta"[..]));
        assert_eq!(login.asked(), [format!("{SVC}/{SECRET_KEY_ENTRY} mac.signed"), format!("{SVC}/{META} mac")]);
        assert_eq!(login.accounts(SECRET_KEY_ENTRY), ["mac.signed.bbbb"]);
        assert_eq!(login.accounts(META), ["mac.signed.bbbb"]);
    }

    /// Of several older builds' items, the newest is the one that counts (it holds the latest
    /// rename); the rest go.
    #[test]
    fn the_newest_older_item_wins() {
        let login = Login::default();
        login.put("aaaa", META, "mac.signed.aaaa", b"first");
        login.put("cccc", META, "mac.signed.cccc", b"latest");
        let new = store(&login, "bbbb", Entries::new());
        assert_eq!(read(&new, META).as_deref(), Some(&b"latest"[..]));
        assert_eq!(login.asked().len(), 1);
        assert_eq!(login.accounts(META), ["mac.signed.bbbb"]);
    }

    /// An entry the old build knew to be absent (a provider key the user removed) stays absent:
    /// an older copy of it is removed, not read.
    #[test]
    fn an_entry_handed_over_as_absent_is_not_brought_back() {
        let login = Login::default();
        login.put("0.0.12", "voltip.provider.openai.llm", "mac.signed", b"old key");
        let new = store(&login, "bbbb", vec![("voltip.provider.openai.llm".to_owned(), None)]);
        assert_eq!(read(&new, "voltip.provider.openai.llm"), None);
        assert!(login.asked().is_empty());
        assert!(login.accounts("voltip.provider.openai.llm").is_empty());
    }

    /// A hand-over that cannot be stored is still used for this run, and the older items stay
    /// for the next one (which asks once): nothing is lost.
    #[test]
    fn a_hand_over_that_cannot_be_stored_keeps_the_older_items() {
        let login = Login::default();
        login.put("aaaa", META, "mac.signed.aaaa", b"meta");
        let mut locked = store(&login, "bbbb", vec![(META.to_owned(), Some(Zeroizing::new(b"meta".to_vec())))]);
        locked.keychain.fail_write = true;
        assert_eq!(read(&locked, META).as_deref(), Some(&b"meta"[..]));
        assert_eq!(login.accounts(META), ["mac.signed.aaaa"]);
        assert!(login.asked().is_empty());
    }

    /// What the next build is handed is what this one ended with: values read, written and
    /// handed over, and entries removed as absent.
    #[test]
    fn the_hand_over_is_what_this_build_ended_with() {
        let login = Login::default();
        login.put("aaaa", META, "mac.signed.aaaa", b"meta");
        let s = store(&login, "bbbb", vec![("voltip.unread".to_owned(), Some(Zeroizing::new(b"kept".to_vec())))]);
        s.set(SECRET_KEY_ENTRY, b"key").unwrap();
        assert_eq!(read(&s, META).as_deref(), Some(&b"meta"[..]));
        s.set("voltip.provider.x.llm", b"k").unwrap();
        s.delete("voltip.provider.x.llm").unwrap();
        assert_eq!(read(&s, "voltip.never.stored"), None);
        let state: Vec<(String, Option<Vec<u8>>)> = s.handoff_state().unwrap().into_iter().map(|(k, v)| (k, v.map(|v| v.to_vec()))).collect();
        assert_eq!(
            state,
            [
                ("voltip.identity.meta".to_owned(), Some(b"meta".to_vec())),
                ("voltip.identity.x25519".to_owned(), Some(b"key".to_vec())),
                ("voltip.never.stored".to_owned(), None),
                ("voltip.provider.x.llm".to_owned(), None),
                ("voltip.unread".to_owned(), Some(b"kept".to_vec())),
            ]
        );
    }

    /// Resetting the device removes every copy, older builds' too, so none can come back.
    #[test]
    fn delete_removes_every_copy() {
        let login = Login::default();
        login.put("0.0.12", SECRET_KEY_ENTRY, "mac.signed", b"key");
        login.put("aaaa", SECRET_KEY_ENTRY, "mac.signed.aaaa", b"key");
        let s = store(&login, "bbbb", Entries::new());
        s.set(SECRET_KEY_ENTRY, b"key").unwrap();
        s.delete(SECRET_KEY_ENTRY).unwrap();
        assert!(login.accounts(SECRET_KEY_ENTRY).is_empty());
        assert_eq!(read(&s, SECRET_KEY_ENTRY), None);
        assert_eq!(read(&store(&login, "cccc", Entries::new()), SECRET_KEY_ENTRY), None);
    }

    /// Nothing anywhere: nothing asked, nothing created, and the answer is remembered.
    #[test]
    fn nothing_stored_is_none_without_asking() {
        let login = Login::default();
        let s = store(&login, "bbbb", Entries::new());
        assert_eq!(read(&s, SECRET_KEY_ENTRY), None);
        assert_eq!(read(&s, SECRET_KEY_ENTRY), None);
        assert!(login.asked().is_empty());
        assert!(login.accounts(SECRET_KEY_ENTRY).is_empty());
        assert_eq!(s.backend_name(), "keychain");
        assert!(format!("{s:?}").contains("bbbb"));
    }

    /// An item under this build's own account that this build cannot read (it never is, but a
    /// keychain can be restored from elsewhere) is an error, not a silent new identity.
    #[test]
    fn an_unreadable_item_of_this_build_is_an_error() {
        let login = Login::default();
        login.put("elsewhere", SECRET_KEY_ENTRY, "mac.signed.bbbb", b"key");
        let s = store(&login, "bbbb", Entries::new());
        assert!(matches!(s.get(SECRET_KEY_ENTRY), Err(IdentityError::StoreUnavailable(_))));
    }
}
