//! The bearer token every request but `/healthz` carries (docs/dictation.md §23.7), compared in
//! constant time. The token can be replaced while the service runs: the next request needs the
//! new one.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::header;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use parking_lot::RwLock;
use subtle::ConstantTimeEq as _;

use crate::openai::ApiError;

/// The token the service accepts now.
#[derive(Clone)]
pub struct Token(Arc<RwLock<String>>);

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(..)")
    }
}

impl Token {
    /// Accept `token`.
    pub fn new(token: String) -> Self {
        Self(Arc::new(RwLock::new(token)))
    }

    /// Accept `token` from now on, and only it.
    pub fn set(&self, token: String) {
        *self.0.write() = token;
    }

    /// Whether an `Authorization` header value carries the token.
    pub fn accepts(&self, authorization: &str) -> bool {
        let Some(given) = authorization.strip_prefix("Bearer ").map(str::trim) else { return false };
        let expected = self.0.read();
        !expected.is_empty() && bool::from(given.as_bytes().ct_eq(expected.as_bytes()))
    }
}

/// Middleware: a request without the token is answered 401 before anything else happens.
pub async fn require(State(token): State<Token>, request: Request, next: Next) -> Response {
    let authorized = request.headers().get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).is_some_and(|v| token.accepts(v));
    if !authorized {
        return ApiError::unauthorized().into_response();
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_current_token_is_accepted() {
        let token = Token::new("abc".into());
        assert!(token.accepts("Bearer abc"));
        assert!(token.accepts("Bearer  abc "));
        for bad in ["", "abc", "Bearer ", "Bearer abd", "Bearer abcd", "Basic abc", "bearer abc"] {
            assert!(!token.accepts(bad), "{bad:?}");
        }
        token.set("new".into());
        assert!(!token.accepts("Bearer abc") && token.accepts("Bearer new"));
        token.set(String::new());
        assert!(!token.accepts("Bearer "), "an empty token accepts nothing");
        assert_eq!(format!("{token:?}"), "Token(..)");
    }
}
