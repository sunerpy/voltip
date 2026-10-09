//! A recording [`Injector`] for the core's and the shells' tests.

use std::sync::{Mutex, PoisonError};

use crate::{InjectError, Injection, Injector};

/// Records every `inject` call and answers with a configured outcome. The default outcome is a
/// successful paste; `chars` in a successful outcome is always recomputed from the text.
pub struct FakeInjector {
    calls: Mutex<Vec<String>>,
    outcome: Mutex<Result<Injection, InjectError>>,
}

impl Default for FakeInjector {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeInjector {
    /// Succeeds with [`Via::Paste`].
    pub fn new() -> Self {
        Self { calls: Mutex::new(Vec::new()), outcome: Mutex::new(Ok(Injection::pasted(0))) }
    }

    /// Builder form of [`FakeInjector::set_outcome`].
    pub fn with_outcome(self, outcome: Result<Injection, InjectError>) -> Self {
        self.set_outcome(outcome);
        self
    }

    /// What the next `inject` calls return. `Ok` outcomes get `chars` filled in per call.
    pub fn set_outcome(&self, outcome: Result<Injection, InjectError>) {
        *self.outcome.lock().unwrap_or_else(PoisonError::into_inner) = outcome;
    }

    /// Texts passed to `inject`, in order (including ones that were answered with an error).
    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// The most recent text passed to `inject`.
    pub fn last(&self) -> Option<String> {
        self.calls.lock().unwrap_or_else(PoisonError::into_inner).last().cloned()
    }
}

impl Injector for FakeInjector {
    fn inject(&self, text: &str) -> Result<Injection, InjectError> {
        self.calls.lock().unwrap_or_else(PoisonError::into_inner).push(text.to_string());
        let outcome = self.outcome.lock().unwrap_or_else(PoisonError::into_inner).clone();
        outcome.map(|mut injection| {
            injection.chars = text.chars().count();
            injection
        })
    }

    fn describe(&self) -> &'static str {
        "fake"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Via;

    #[test]
    fn records_calls_and_returns_configured_outcome() {
        let fake = FakeInjector::default();
        assert_eq!(fake.describe(), "fake");
        assert_eq!(fake.last(), None);
        assert_eq!(fake.inject("hello").unwrap(), Injection { via: Via::Paste, chars: 5, note: None });
        let note = crate::InjectNote::new(crate::FallbackCode::NoTool, "wayland");
        fake.set_outcome(Ok(Injection::clipboard(0, Some(note.clone()))));
        assert_eq!(fake.inject("你好").unwrap(), Injection { via: Via::Clipboard, chars: 2, note: Some(note) });
        fake.set_outcome(Err(InjectError::Clipboard("busy".into())));
        assert_eq!(fake.inject("x").unwrap_err(), InjectError::Clipboard("busy".into()));
        assert_eq!(fake.calls(), vec!["hello".to_string(), "你好".to_string(), "x".to_string()]);
        assert_eq!(fake.last().as_deref(), Some("x"));

        let failing = FakeInjector::new().with_outcome(Err(InjectError::EmptyText));
        assert_eq!(failing.inject("").unwrap_err(), InjectError::EmptyText);
        let boxed: Box<dyn Injector> = Box::new(FakeInjector::new());
        assert_eq!(boxed.inject("via trait object").unwrap().chars, 16);
    }
}
