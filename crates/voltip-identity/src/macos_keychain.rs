//! The login keychain as [`Keychain`] (macOS), through security-framework's safe calls.
//!
//! Every call names the default keychain explicitly: lookups through the implicit search list
//! found nothing on GitHub's Macs, even for the item's own creator (runs 36722230660 and
//! 36725475355). Listing (attributes only) and removing (by reference, never reading the value) do
//! not need the item's partition, so neither ever asks (same runs, both kinds of Mac).
//!
//! Excluded from the coverage gate with the rest of the platform store: it only runs where a real
//! keychain is unlocked. `.github/scripts/check-keychain-handoff.sh` runs it on both Macs.

use parking_lot::Mutex;
use security_framework::base::Error as SecError;
use security_framework::item::{ItemClass, ItemSearchOptions, Limit, Reference, SearchResult};
use security_framework::os::macos::keychain::SecKeychain;
use zeroize::Zeroizing;

use crate::IdentityError;
use crate::per_build::{Ask, Keychain, Read, Stored};

/// `errSecItemNotFound`.
const NOT_FOUND: i32 = -25300;
/// `errSecAuthFailed`: another build's item with the dialog turned off (the partition check).
const AUTH_FAILED: i32 = -25293;
/// `errSecInteractionNotAllowed`.
const INTERACTION_NOT_ALLOWED: i32 = -25308;

/// The user's default keychain (the login keychain).
pub struct SecurityKeychain {
    keychain: SecKeychain,
    /// One call at a time: turning the dialog off is process-wide.
    lock: Mutex<()>,
}

impl std::fmt::Debug for SecurityKeychain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecurityKeychain").finish_non_exhaustive()
    }
}

impl SecurityKeychain {
    /// The user's default keychain.
    pub fn login() -> Result<Self, IdentityError> {
        Ok(Self { keychain: SecKeychain::default().map_err(unavailable)?, lock: Mutex::new(()) })
    }

    fn search(&self, service: &str) -> ItemSearchOptions {
        let mut options = ItemSearchOptions::new();
        options.class(ItemClass::generic_password()).keychains(std::slice::from_ref(&self.keychain)).service(service).limit(Limit::All);
        options
    }
}

fn unavailable(e: SecError) -> IdentityError {
    IdentityError::StoreUnavailable(e.to_string())
}

impl Keychain for SecurityKeychain {
    fn items(&self, service: &str) -> Result<Vec<Stored>, IdentityError> {
        let _one = self.lock.lock();
        let found = match self.search(service).load_attributes(true).search() {
            Ok(found) => found,
            Err(e) if e.code() == NOT_FOUND => return Ok(Vec::new()),
            Err(e) => return Err(unavailable(e)),
        };
        Ok(found
            .iter()
            .filter_map(SearchResult::simplify_dict)
            .filter_map(|attributes| {
                let account = attributes.get("acct")?.clone();
                Some(Stored { account, created: attributes.get("cdat").cloned().unwrap_or_default() })
            })
            .collect())
    }

    fn read(&self, service: &str, account: &str, ask: Ask) -> Result<Read, IdentityError> {
        let _one = self.lock.lock();
        // Quietly first: an item this build may read, or no item at all, never needs the dialog.
        let quiet = {
            let _quiet = SecKeychain::disable_user_interaction().map_err(unavailable)?;
            self.keychain.find_generic_password(service, account)
        };
        match quiet {
            Ok((password, _item)) => return Ok(Read::Found(Zeroizing::new(password.to_vec()))),
            Err(e) if e.code() == NOT_FOUND => return Ok(Read::Missing),
            Err(e) if matches!(e.code(), AUTH_FAILED | INTERACTION_NOT_ALLOWED) => {
                if ask == Ask::Never {
                    return Ok(Read::WouldAsk);
                }
            }
            Err(e) => return Err(unavailable(e)),
        }
        tracing::info!(service, account, "reading the keychain item an earlier build stored; macOS asks once");
        match self.keychain.find_generic_password(service, account) {
            Ok((password, _item)) => Ok(Read::Found(Zeroizing::new(password.to_vec()))),
            Err(e) if e.code() == NOT_FOUND => Ok(Read::Missing),
            Err(e) => Err(unavailable(e)),
        }
    }

    fn write(&self, service: &str, account: &str, value: &[u8]) -> Result<(), IdentityError> {
        let _one = self.lock.lock();
        // An item of this build's never needs the dialog; one that would is not overwritten.
        let _quiet = SecKeychain::disable_user_interaction().map_err(unavailable)?;
        self.keychain.set_generic_password(service, account, value).map_err(unavailable)
    }

    fn remove(&self, service: &str, account: &str) -> Result<(), IdentityError> {
        let _one = self.lock.lock();
        let _quiet = SecKeychain::disable_user_interaction().map_err(unavailable)?;
        let found = match self.search(service).account(account).load_refs(true).search() {
            Ok(found) => found,
            Err(e) if e.code() == NOT_FOUND => return Ok(()),
            Err(e) => return Err(unavailable(e)),
        };
        for result in found {
            if let SearchResult::Ref(Reference::KeychainItem(item)) = result {
                item.delete();
            }
        }
        // `delete` reports nothing: look again.
        match self.search(service).account(account).load_refs(true).search() {
            Err(e) if e.code() == NOT_FOUND => Ok(()),
            Ok(left) if left.is_empty() => Ok(()),
            Ok(_) => Err(IdentityError::StoreUnavailable(format!("the keychain item {service} ({account}) could not be removed"))),
            Err(e) => Err(unavailable(e)),
        }
    }
}
