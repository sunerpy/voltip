//! Where a grant the system remembers for Voltip is kept between pastes and restarts: the restore
//! token of GNOME's remote-desktop portal (docs/dictation.md §14.9). The desktop shell supplies a
//! store over the system keychain, the one that keeps the API keys; [`MemoryRestoreTokens`] keeps
//! the token for this process only.

use std::sync::{Mutex, PoisonError};

/// One saved token. The store logs its own failures, never the token: a token it cannot read is
/// absent (the next paste asks the user again), one it cannot save is lost the same way.
pub trait RestoreTokens: Send + Sync {
    /// The token saved last, if any.
    fn load(&self) -> Option<String>;
    /// Replace the saved token with `token`.
    fn save(&self, token: &str);
    /// Forget the saved token.
    fn clear(&self);
}

/// A token kept in memory for this process: the default where no shell provides a store.
#[derive(Default)]
pub struct MemoryRestoreTokens {
    token: Mutex<Option<String>>,
}

impl MemoryRestoreTokens {
    /// Nothing saved.
    pub fn new() -> Self {
        Self::default()
    }
}

impl std::fmt::Debug for MemoryRestoreTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The token itself never reaches a log.
        f.debug_struct("MemoryRestoreTokens").field("saved", &self.load().is_some()).finish()
    }
}

impl RestoreTokens for MemoryRestoreTokens {
    fn load(&self) -> Option<String> {
        self.token.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn save(&self, token: &str) {
        *self.token.lock().unwrap_or_else(PoisonError::into_inner) = Some(token.to_owned());
    }

    fn clear(&self) {
        *self.token.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_tokens_replace_and_clear() {
        let tokens = MemoryRestoreTokens::new();
        assert_eq!(tokens.load(), None);
        tokens.save("first");
        tokens.save("second");
        assert_eq!(tokens.load().as_deref(), Some("second"), "a new token replaces the old one");
        assert_eq!(format!("{tokens:?}"), "MemoryRestoreTokens { saved: true }", "the token is not printed");
        tokens.clear();
        assert_eq!(tokens.load(), None);
    }
}
