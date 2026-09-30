//! The keychain hand-over of an in-app update (macOS; docs/runbook.md 发布 · macOS 签名与钥匙串).
//!
//! macOS asks before one build reads a keychain item another build created: a build signed with
//! a self-signed certificate has a partition of its own, its cdhash (user report 2026-09-30).
//! So the releases keep their secrets in items they created ([`voltip_identity::PerBuildStore`]),
//! and after installing an update the old build starts the new one itself and hands over what its
//! store holds ([`voltip_identity::handoff`]). Each side first checks that the other process
//! satisfies this build's designated requirement, so the secrets only go to a release signed with
//! the same certificate.
//!
//! Security.framework's `SecCodeCopyDesignatedRequirement` and `SecCodeCopySigningInformation` are
//! not in security-framework, hence the few `extern` calls here.
#![allow(unsafe_code)]

use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use core_foundation::base::{CFType, TCFType};
use core_foundation::data::CFData;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::CFString;
use core_foundation_sys::dictionary::CFDictionaryRef;
use core_foundation_sys::string::CFStringRef;
use security_framework::os::macos::code_signing::{Flags, GuestAttributes, SecCode, SecRequirement};
use security_framework_sys::code_signing::{SecRequirementRef, SecStaticCodeRef};
use voltip_identity::Entries;
use voltip_identity::handoff::{self, HANDOFF_ENV, PeerCheck, StartError};

/// How long the new build waits for the hand-over, and the old one for its acknowledgement.
const TIMEOUT: Duration = Duration::from_secs(10);

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecCodeCopyDesignatedRequirement(code: SecStaticCodeRef, flags: u32, requirement: *mut SecRequirementRef) -> i32;
    fn SecCodeCopySigningInformation(code: SecStaticCodeRef, flags: u32, information: *mut CFDictionaryRef) -> i32;
    static kSecCodeInfoUnique: CFStringRef;
}

/// This process's code, as the static code the two calls above take (a `SecCodeRef` is accepted
/// wherever a `SecStaticCodeRef` is).
fn own_code() -> Option<SecCode> {
    SecCode::for_self(Flags::NONE).ok()
}

/// This build's designated requirement (`identifier "dev.voltip.desktop" and certificate leaf =
/// H"…"` for a release).
fn designated_requirement() -> Option<SecRequirement> {
    let code = own_code()?;
    let mut requirement: SecRequirementRef = std::ptr::null_mut();
    // SAFETY: `code` is a live code object for the whole call; on success `requirement` holds a
    // +1 reference, which `wrap_under_create_rule` takes over.
    let status = unsafe { SecCodeCopyDesignatedRequirement(code.as_concrete_TypeRef() as SecStaticCodeRef, 0, &mut requirement) };
    // SAFETY: a non-null +1 reference from the call above.
    (status == 0 && !requirement.is_null()).then(|| unsafe { SecRequirement::wrap_under_create_rule(requirement) })
}

/// This build's cdhash in hex: the partition macOS files the keychain items it creates under.
pub fn cdhash() -> Option<String> {
    let code = own_code()?;
    let mut information: CFDictionaryRef = std::ptr::null();
    // SAFETY: as in `designated_requirement`; `information` receives a +1 dictionary.
    let status = unsafe { SecCodeCopySigningInformation(code.as_concrete_TypeRef() as SecStaticCodeRef, 0, &mut information) };
    if status != 0 || information.is_null() {
        return None;
    }
    // SAFETY: a non-null +1 dictionary from the call above.
    let information: CFDictionary<CFString, CFType> = unsafe { CFDictionary::wrap_under_create_rule(information) };
    // SAFETY: an immutable constant exported by Security.framework.
    let key = unsafe { CFString::wrap_under_get_rule(kSecCodeInfoUnique) };
    let unique = information.find(&key)?.downcast::<CFData>()?;
    Some(unique.bytes().iter().map(|b| format!("{b:02x}")).collect())
}

/// Trusts a process that satisfies this build's designated requirement.
struct SignedLikeThisBuild(SecRequirement);

impl SignedLikeThisBuild {
    fn new() -> Option<Self> {
        designated_requirement().map(Self)
    }
}

impl PeerCheck for SignedLikeThisBuild {
    fn trusted(&self, pid: u32) -> bool {
        let Ok(pid) = i32::try_from(pid) else { return false };
        let mut attributes = GuestAttributes::new();
        attributes.set_pid(pid);
        SecCode::copy_guest_with_attribues(None, &attributes, Flags::NONE).and_then(|code| code.check_validity(Flags::NONE, &self.0)).is_ok()
    }
}

/// What the previous build handed over, until the secret store takes it.
static RECEIVED: Mutex<Option<Entries>> = Mutex::new(None);

/// The new build's side, called first thing in `run`, before any other thread exists and before
/// the single-instance check: take the hand-over the old build started this process with.
pub fn receive_at_startup() {
    if std::env::var_os(HANDOFF_ENV).is_none_or(|v| v != "1") {
        return;
    }
    // SAFETY: `run` calls this before it starts any thread, so nothing reads the environment
    // concurrently; later children must not inherit the request.
    unsafe { std::env::remove_var(HANDOFF_ENV) };
    let received = SignedLikeThisBuild::new().ok_or(handoff::HandoffError::NotTrusted).and_then(|check| handoff::take_from_stdin(&check, TIMEOUT));
    match received {
        Ok(entries) => {
            tracing::info!(entries = entries.len(), "keychain hand-over received from the previous build");
            *RECEIVED.lock().unwrap_or_else(PoisonError::into_inner) = Some(entries);
        }
        Err(e) => tracing::warn!(error = %e, "no keychain hand-over taken; an earlier build's items are read once"),
    }
}

/// The hand-over [`receive_at_startup`] took (empty when there was none); taken once.
pub fn take_received() -> Entries {
    RECEIVED.lock().unwrap_or_else(PoisonError::into_inner).take().unwrap_or_default()
}

/// The old build's side once an update is installed: start the new build (the bundle at this
/// path now holds it) with this process's arguments and the hand-over.
pub fn start_new_build(entries: &Entries) -> Result<(), StartError> {
    let exe = std::env::current_exe().map_err(StartError::NotStarted)?;
    let check = SignedLikeThisBuild::new().ok_or_else(|| StartError::NotStarted(std::io::Error::other("this build has no designated requirement")))?;
    handoff::start(&exe, std::env::args_os().skip(1), entries, &check, TIMEOUT).map(|_| ())
}
