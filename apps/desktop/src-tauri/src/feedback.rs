//! In-app feedback (docs/feedback.md). The 反馈 dialog shows what a report carries, then the shell
//! posts it to the feedback endpoint (`services/feedback`, a Cloudflare Worker). The endpoint and
//! its application token are build secrets like the built-in services: `option_env!` bakes them in,
//! nothing in the tree or the UI names the host. A report carries the user's words, an optional
//! contact, and diagnostics that name no host, no key and no dictation.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use voltip_core::ui::UiState;

/// The endpoint (`https://<host>/v1/feedback`) as the build received it; see [`feedback_url`].
const FEEDBACK_URL: Option<&str> = option_env!("VOLTIP_FEEDBACK_URL");
/// The application token the endpoint checks (Bearer), as the build received it.
const FEEDBACK_TOKEN: Option<&str> = option_env!("VOLTIP_FEEDBACK_TOKEN");

/// A build value, or none when it is empty: the release workflow passes an unset secret as `""`.
fn nonempty(value: Option<&'static str>) -> Option<&'static str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

/// The feedback endpoint of this build, if it has one.
pub fn feedback_url() -> Option<&'static str> {
    nonempty(FEEDBACK_URL)
}

/// The endpoint's application token, if the build has one.
pub fn feedback_token() -> Option<&'static str> {
    nonempty(FEEDBACK_TOKEN)
}

/// The longest message, in UTF-16 code units like the dialog's `maxLength`: the endpoint's limit
/// (`services/feedback/src/validate.ts`).
pub const MAX_MESSAGE_UNITS: usize = 5000;
/// The longest contact, counted the same way.
pub const MAX_CONTACT_UNITS: usize = 200;
/// Deadline of one submission.
pub const SEND_TIMEOUT: Duration = Duration::from_secs(15);

/// What a report says it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackKind {
    /// Something does not work.
    Bug,
    /// A suggestion.
    Idea,
    /// Anything else.
    Other,
}

/// The facts a report carries besides the user's words: the version, the platform and which
/// provider kind is in use. No host, no key, no model endpoint, no dictation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostics {
    /// The app version.
    pub app_version: String,
    /// `windows`, `linux` or `macos`.
    pub os: String,
    /// `x86_64`, `aarch64`.
    pub arch: String,
    /// The Linux graphical session (`wayland`, `x11`, `xwayland`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// The UI language as the webview resolved it (`zh-CN`, `en`).
    pub locale: String,
    /// The recognition provider's id (`builtin`, `local`, `openai`, …).
    pub asr_provider: String,
    /// The local model's catalogue id, when recognition runs on the device.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_model: Option<String>,
    /// Where local models run (`auto`, `cpu`, `gpu`), when recognition runs on the device.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compute: Option<String>,
    /// The clean-up provider's id, when the clean-up is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_provider: Option<String>,
    /// The output mode the next take runs with.
    pub output_mode: String,
}

/// `feedback_diagnostics`: whether this build can send, and what a report would carry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FeedbackInfo {
    /// The build has an endpoint.
    pub configured: bool,
    /// What the report attaches.
    pub diagnostics: Diagnostics,
}

/// The endpoint's answer to a stored report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    /// The report's id.
    pub id: String,
}

/// Why a report did not go out; the wire form is the `snake_case` name, which the dialog words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendError {
    /// This build has no endpoint.
    NotConfigured,
    /// The report is empty or too long (or the endpoint refused its shape).
    Invalid,
    /// Too many reports from this address.
    RateLimited,
    /// The endpoint refused the application token.
    Unauthorized,
    /// No connection.
    Network,
    /// No answer in time.
    Timeout,
    /// The endpoint failed.
    Server,
}

impl SendError {
    /// Wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotConfigured => "not_configured",
            Self::Invalid => "invalid",
            Self::RateLimited => "rate_limited",
            Self::Unauthorized => "unauthorized",
            Self::Network => "network",
            Self::Timeout => "timeout",
            Self::Server => "server",
        }
    }
}

/// The wire form of a string enum (`auto`, `whole_take`).
fn wire<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}

/// A locale tag as the webview resolved it, or `unknown` for anything that is not one.
fn locale_tag(locale: &str) -> String {
    let tag = locale.trim();
    let ok = !tag.is_empty() && tag.len() <= 16 && tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if ok { tag.to_owned() } else { "unknown".to_owned() }
}

/// The diagnostics for the state the UI shows now.
pub fn diagnostics(state: &UiState, locale: &str, session: Option<String>) -> Diagnostics {
    let engines = &state.engines;
    let on_device = engines.asr_provider == voltip_core::ProviderId::Local;
    Diagnostics {
        app_version: crate::APP_VERSION.to_owned(),
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        session,
        locale: locale_tag(locale),
        asr_provider: engines.asr_provider.as_str().to_owned(),
        local_model: engines.local_model.clone().filter(|_| on_device),
        compute: on_device.then(|| wire(&state.settings.engines.local_device)),
        llm_provider: engines.llm_provider.filter(|_| engines.refine_enabled).map(|p| p.as_str().to_owned()),
        output_mode: wire(&engines.effective_output_mode),
    }
}

/// The user's words, trimmed and within the limits; an empty contact is none.
pub fn check(message: &str, contact: Option<&str>) -> Result<(String, Option<String>), SendError> {
    let message = message.trim();
    if message.is_empty() || message.encode_utf16().count() > MAX_MESSAGE_UNITS {
        return Err(SendError::Invalid);
    }
    let contact = contact.map(str::trim).filter(|c| !c.is_empty());
    if contact.is_some_and(|c| c.encode_utf16().count() > MAX_CONTACT_UNITS) {
        return Err(SendError::Invalid);
    }
    Ok((message.to_owned(), contact.map(str::to_owned)))
}

#[derive(Serialize)]
struct Payload<'a> {
    kind: FeedbackKind,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    contact: Option<&'a str>,
    diagnostics: &'a Diagnostics,
}

/// Post one report to `url`. Errors carry no host: the endpoint is a build secret.
pub async fn send(
    url: &str,
    token: Option<&str>,
    kind: FeedbackKind,
    message: &str,
    contact: Option<&str>,
    diagnostics: &Diagnostics,
) -> Result<Receipt, SendError> {
    let http = reqwest::Client::builder()
        .timeout(SEND_TIMEOUT)
        .user_agent(concat!("voltip-desktop/", env!("VOLTIP_APP_VERSION")))
        .build()
        .map_err(|_| SendError::Network)?;
    let mut request = http.post(url).json(&Payload { kind, message, contact, diagnostics });
    if let Some(token) = token.filter(|t| !t.is_empty()) {
        request = request.bearer_auth(token);
    }
    let response = request.send().await.map_err(|e| if e.is_timeout() { SendError::Timeout } else { SendError::Network })?;
    let status = response.status().as_u16();
    match status {
        200 | 201 => response.json::<Receipt>().await.map_err(|_| SendError::Server),
        400 | 413 | 415 => Err(SendError::Invalid),
        401 | 403 => Err(SendError::Unauthorized),
        429 => Err(SendError::RateLimited),
        _ => Err(SendError::Server),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use voltip_core::ProviderId;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn state() -> UiState {
        let mut state = UiState::default();
        state.engines.asr_provider = ProviderId::Builtin;
        state.engines.asr_host = "asr.example.test".into();
        state.engines.refine_host = "llm.example.test".into();
        state.engines.refine_enabled = true;
        state.engines.llm_provider = Some(ProviderId::Builtin);
        state.engines.local_model = Some("qwen3-asr-0.6b".into());
        state
    }

    #[test]
    fn diagnostics_name_the_platform_and_the_provider_kinds_and_nothing_else() {
        let d = diagnostics(&state(), "zh-CN", Some("wayland".into()));
        assert_eq!(d.app_version, crate::APP_VERSION);
        assert_eq!(d.os, std::env::consts::OS);
        assert_eq!(d.arch, std::env::consts::ARCH);
        assert_eq!(d.session.as_deref(), Some("wayland"));
        assert_eq!(d.locale, "zh-CN");
        assert_eq!(d.asr_provider, "builtin");
        assert_eq!(d.llm_provider.as_deref(), Some("builtin"));
        assert_eq!(d.output_mode, "whole_take");
        // Not on-device: no model, no compute device.
        assert_eq!((d.local_model.as_deref(), d.compute.as_deref()), (None, None));
        let json = serde_json::to_string(&d).unwrap();
        assert!(!json.contains("example.test"), "{json}");

        let mut local = state();
        local.engines.asr_provider = ProviderId::Local;
        local.engines.refine_enabled = false;
        let d = diagnostics(&local, "<script>", None);
        assert_eq!(d.local_model.as_deref(), Some("qwen3-asr-0.6b"));
        assert_eq!(d.compute.as_deref(), Some("auto"));
        assert_eq!(d.llm_provider, None, "the clean-up is off");
        assert_eq!(d.locale, "unknown");
        assert_eq!(d.session, None);
        assert!(!serde_json::to_string(&d).unwrap().contains("session"));
    }

    #[test]
    fn an_empty_build_value_is_no_value() {
        assert_eq!(nonempty(Some("")), None);
        assert_eq!(nonempty(Some("  ")), None);
        assert_eq!(nonempty(None), None);
        assert_eq!(nonempty(Some(" https://feedback.example.test/v1/feedback ")), Some("https://feedback.example.test/v1/feedback"));
        assert_eq!(feedback_url(), nonempty(FEEDBACK_URL));
        assert_eq!(feedback_token(), nonempty(FEEDBACK_TOKEN));
    }

    #[test]
    fn a_report_is_trimmed_and_kept_within_the_endpoints_limits() {
        assert_eq!(check("  hi  ", Some("  ")), Ok(("hi".to_owned(), None)));
        assert_eq!(check("hi", Some(" me@example.test ")), Ok(("hi".to_owned(), Some("me@example.test".to_owned()))));
        assert_eq!(check("   ", None), Err(SendError::Invalid));
        assert_eq!(check(&"长".repeat(MAX_MESSAGE_UNITS), None).map(|(m, _)| m.chars().count()), Ok(MAX_MESSAGE_UNITS));
        assert_eq!(check(&"长".repeat(MAX_MESSAGE_UNITS + 1), None), Err(SendError::Invalid));
        assert_eq!(check("hi", Some(&"a".repeat(MAX_CONTACT_UNITS + 1))), Err(SendError::Invalid));
        let names: Vec<&str> = [
            SendError::NotConfigured,
            SendError::Invalid,
            SendError::RateLimited,
            SendError::Unauthorized,
            SendError::Network,
            SendError::Timeout,
            SendError::Server,
        ]
        .map(SendError::as_str)
        .to_vec();
        assert_eq!(names, ["not_configured", "invalid", "rate_limited", "unauthorized", "network", "timeout", "server"]);
    }

    #[tokio::test]
    async fn a_report_goes_out_with_the_token_and_the_answer_is_read() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/feedback"))
            .and(header("authorization", "Bearer app-token"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({ "id": "f-1" })))
            .expect(1)
            .mount(&server)
            .await;
        let d = diagnostics(&state(), "en", None);
        let url = format!("{}/v1/feedback", server.uri());
        let receipt = send(&url, Some("app-token"), FeedbackKind::Bug, "no paste", Some("me@example.test"), &d).await.unwrap();
        assert_eq!(receipt, Receipt { id: "f-1".into() });
        let seen = server.received_requests().await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&seen[0].body).unwrap();
        assert_eq!(body["kind"], "bug");
        assert_eq!(body["message"], "no paste");
        assert_eq!(body["contact"], "me@example.test");
        assert_eq!(body["diagnostics"]["asr_provider"], "builtin");
        assert!(body["diagnostics"].get("session").is_none());
    }

    #[tokio::test]
    async fn every_refusal_has_its_reason() {
        let d = diagnostics(&state(), "en", None);
        for (status, want) in [
            (400, SendError::Invalid),
            (413, SendError::Invalid),
            (401, SendError::Unauthorized),
            (429, SendError::RateLimited),
            (500, SendError::Server),
            (302, SendError::Server),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST")).respond_with(ResponseTemplate::new(status)).mount(&server).await;
            let got = send(&server.uri(), None, FeedbackKind::Idea, "x", None, &d).await;
            assert_eq!(got, Err(want), "HTTP {status}");
        }
        // A 201 that is not a receipt is the endpoint's failure.
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(201).set_body_string("ok")).mount(&server).await;
        assert_eq!(send(&server.uri(), None, FeedbackKind::Other, "x", None, &d).await, Err(SendError::Server));
        // Nothing listening: a network failure, and no request went out without the token check.
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = closed.local_addr().unwrap().port();
        drop(closed);
        assert_eq!(send(&format!("http://127.0.0.1:{port}/v1/feedback"), Some(""), FeedbackKind::Other, "x", None, &d).await, Err(SendError::Network));
    }
}
