//! What every request goes through before the protocol sees it (docs/dictation.md §25):
//!
//! 1. `Host` must be `127.0.0.1:<port>` or `localhost:<port>`: a web page that rebinds a domain
//!    of its own to 127.0.0.1 sends its own name and is refused (403).
//! 2. A request with an `Origin` header is refused (403): browsers send one with every request a
//!    page makes, the AI tools never do. No CORS header is ever sent either.
//! 3. Everything but `/healthz` carries `Authorization: Bearer <token>`, compared in constant
//!    time (401 with `WWW-Authenticate: Bearer`). The token can be replaced while the service
//!    runs; the next request needs the new one.
//! 4. A request that would open a session (a POST without `Mcp-Session-Id`) waits for the others
//!    that do, and is refused (503) while `max_sessions` are open.
//! 5. Each session makes at most `calls_per_second` calls in any second (429).
//!
//! A refusal is a JSON-RPC error whose message says what happened, in Chinese and English.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use parking_lot::{Mutex, RwLock};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use subtle::ConstantTimeEq as _;

/// The header a session's requests carry.
pub const SESSION_HEADER: &str = "mcp-session-id";
/// A session not heard from this long is forgotten by the call counter.
const FORGET_AFTER: Duration = Duration::from_secs(60);

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

/// The `Host` values a listener on `port` accepts.
pub fn allowed_hosts(port: u16) -> Vec<String> {
    vec![format!("127.0.0.1:{port}"), format!("localhost:{port}")]
}

struct Inner {
    token: Token,
    allowed: Vec<String>,
    sessions: Arc<LocalSessionManager>,
    max_sessions: usize,
    calls_per_second: u32,
    /// Held while a request that may open a session goes through, so the count stays honest.
    opening: tokio::sync::Mutex<()>,
    /// Each session's calls of the last second.
    calls: Mutex<HashMap<String, VecDeque<Instant>>>,
}

/// The checks of one listener.
#[derive(Clone)]
pub struct Guard(Arc<Inner>);

impl std::fmt::Debug for Guard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Guard").field("allowed", &self.0.allowed).field("max_sessions", &self.0.max_sessions).finish_non_exhaustive()
    }
}

impl Guard {
    /// The checks with `token`, the `allowed` hosts, the session store whose sessions count, and the
    /// limits.
    pub fn new(token: Token, allowed: Vec<String>, sessions: Arc<LocalSessionManager>, max_sessions: usize, calls_per_second: u32) -> Self {
        Self(Arc::new(Inner {
            token,
            allowed,
            sessions,
            max_sessions: max_sessions.max(1),
            calls_per_second: calls_per_second.max(1),
            opening: tokio::sync::Mutex::new(()),
            calls: Mutex::new(HashMap::new()),
        }))
    }

    /// The token (replace it with [`Token::set`]).
    pub fn token(&self) -> Token {
        self.0.token.clone()
    }

    /// Sessions open now.
    pub async fn sessions(&self) -> usize {
        self.0.sessions.sessions.read().await.len()
    }

    fn host_allowed(&self, request: &Request) -> bool {
        let host =
            request.headers().get(header::HOST).and_then(|v| v.to_str().ok()).map(str::to_owned).or_else(|| request.uri().authority().map(|a| a.to_string()));
        host.is_some_and(|host| self.0.allowed.iter().any(|allowed| allowed.eq_ignore_ascii_case(host.trim())))
    }

    /// Note a call of session `id` now; `false` when it already made its calls of the last second.
    fn allow_call(&self, id: &str) -> bool {
        let now = Instant::now();
        let mut calls = self.0.calls.lock();
        if calls.len() > 64 {
            calls.retain(|_, times| times.back().is_some_and(|last| now.duration_since(*last) < FORGET_AFTER));
        }
        let times = calls.entry(id.to_owned()).or_default();
        while times.front().is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(1)) {
            times.pop_front();
        }
        if times.len() >= self.0.calls_per_second as usize {
            return false;
        }
        times.push_back(now);
        true
    }
}

/// A refusal before the protocol: a JSON-RPC error with no id.
fn refuse(status: StatusCode, zh: &str, en: &str) -> Response {
    let body = serde_json::json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32000, "message": format!("{zh}\n{en}") } });
    let mut response = (status, axum::Json(body)).into_response();
    if matches!(status, StatusCode::TOO_MANY_REQUESTS | StatusCode::SERVICE_UNAVAILABLE) {
        response.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
    }
    response
}

/// The middleware.
pub async fn check(State(guard): State<Guard>, request: Request, next: Next) -> Response {
    if !guard.host_allowed(&request) {
        tracing::info!(status = 403, "MCP request refused: Host is not this computer");
        return refuse(StatusCode::FORBIDDEN, "请求的 Host 不是这台电脑。", "The request's Host does not name this computer.");
    }
    if request.headers().contains_key(header::ORIGIN) {
        tracing::info!(status = 403, "MCP request refused: it came from a web page");
        return refuse(StatusCode::FORBIDDEN, "网页不能连接 Voltip 的 AI 工具接入。", "Web pages cannot connect to Voltip's AI tool access.");
    }
    if request.uri().path() == "/healthz" {
        return next.run(request).await;
    }
    let authorized = request.headers().get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).is_some_and(|v| guard.0.token.accepts(v));
    if !authorized {
        tracing::info!(status = 401, "MCP request refused: no or wrong token");
        let mut response = refuse(
            StatusCode::UNAUTHORIZED,
            "缺少令牌或令牌不正确：请在 Voltip 的「设置 › AI 工具接入」中重新复制接入命令。",
            "The token is missing or wrong: copy the setup command again in Voltip's Settings › AI tools.",
        );
        response.headers_mut().insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        return response;
    }
    let session = request.headers().get(SESSION_HEADER).and_then(|v| v.to_str().ok()).map(str::to_owned);
    match session {
        Some(id) => {
            if request.method() == Method::POST && !guard.allow_call(&id) {
                tracing::info!(status = 429, "MCP call refused: too many calls a second");
                return refuse(
                    StatusCode::TOO_MANY_REQUESTS,
                    &format!("调用过于频繁：每个会话每秒最多 {} 次。", guard.0.calls_per_second),
                    &format!("Too many calls: at most {} a second per session.", guard.0.calls_per_second),
                );
            }
            next.run(request).await
        }
        None if request.method() == Method::POST => {
            let _opening = guard.0.opening.lock().await;
            if guard.sessions().await >= guard.0.max_sessions {
                tracing::info!(status = 503, "MCP session refused: the sessions are full");
                return refuse(
                    StatusCode::SERVICE_UNAVAILABLE,
                    &format!("同时最多 {} 个会话：请关闭不用的 AI 工具会话后重试。", guard.0.max_sessions),
                    &format!("At most {} sessions at a time: close an AI tool session you do not use and try again.", guard.0.max_sessions),
                );
            }
            next.run(request).await
        }
        None => next.run(request).await,
    }
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

    #[test]
    fn a_session_makes_at_most_its_calls_a_second() {
        let guard = Guard::new(Token::new("t".into()), allowed_hosts(1), Arc::new(LocalSessionManager::default()), 4, 3);
        assert!((0..3).all(|_| guard.allow_call("a")));
        assert!(!guard.allow_call("a"), "the fourth in the same second");
        assert!(guard.allow_call("b"), "another session counts on its own");
        assert_eq!(allowed_hosts(47841), ["127.0.0.1:47841", "localhost:47841"]);
        assert!(format!("{guard:?}").contains("127.0.0.1:1"));
    }
}
