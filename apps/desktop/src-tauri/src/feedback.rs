//! In-app feedback (docs/feedback.md). The 反馈 page shows what a report carries, then the shell
//! posts it to the feedback endpoint (`services/feedback`, a Cloudflare Worker). The endpoint and
//! its application token are build secrets like the built-in services: `option_env!` bakes them in,
//! nothing in the tree or the UI names the host. A report carries the user's words, an optional
//! contact, diagnostics that name no host, no key and no dictation, and the screenshots or screen
//! recordings the user attached: the page stages them here (`feedback_attachment_add`), the report
//! declares them, and their bytes follow in chunks with the upload token the endpoint answered.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
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

/// What an attachment may be (the endpoint's list, `services/feedback/src/validate.ts`).
pub const ATTACHMENT_TYPES: [&str; 7] = ["image/png", "image/jpeg", "image/gif", "image/webp", "video/mp4", "video/webm", "video/quicktime"];
/// Attachments one report may carry.
pub const MAX_ATTACHMENTS: usize = 3;
/// The largest screenshot.
pub const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
/// The largest screen recording.
pub const MAX_VIDEO_BYTES: usize = 20 * 1024 * 1024;
/// All attachments of one report together.
pub const MAX_ATTACHMENT_TOTAL_BYTES: usize = 25 * 1024 * 1024;
/// The longest file name, in characters; a longer one is shortened, keeping its extension.
pub const MAX_ATTACHMENT_NAME_CHARS: usize = 120;
/// Deadline of one chunk upload.
pub const CHUNK_TIMEOUT: Duration = Duration::from_secs(60);
/// Tries per chunk (a network failure or a 5xx is tried again).
pub const CHUNK_ATTEMPTS: usize = 3;
/// The largest chunk the endpoint may ask for (it asks for 1 MiB).
const MAX_CHUNK_BYTES: usize = 8 * 1024 * 1024;

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

/// The endpoint's answer, with the upload ticket of a report that declared attachments.
#[derive(Deserialize)]
struct Answer {
    id: String,
    #[serde(default)]
    upload: Option<Ticket>,
}

#[derive(Deserialize)]
struct Ticket {
    token: String,
    chunk_bytes: usize,
}

/// Why an attachment was not staged; the wire form is the name the page words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachError {
    /// Not a screenshot or recording format the endpoint takes.
    Type,
    /// Empty, or over the per-type limit.
    TooLarge,
    /// Already [`MAX_ATTACHMENTS`].
    TooMany,
    /// Over [`MAX_ATTACHMENT_TOTAL_BYTES`] together.
    Total,
    /// No usable file name.
    Name,
}

impl AttachError {
    /// Wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Type => "attachment_type",
            Self::TooLarge => "attachment_too_large",
            Self::TooMany => "attachment_too_many",
            Self::Total => "attachment_total",
            Self::Name => "attachment_name",
        }
    }
}

/// A file the page staged for the next report.
#[derive(Clone, Debug)]
pub struct Staged {
    /// The handle the page refers to it by.
    pub id: String,
    /// The file name, cleaned ([`clean_name`]).
    pub name: String,
    /// One of [`ATTACHMENT_TYPES`].
    pub mime: String,
    /// The bytes.
    pub bytes: Arc<Vec<u8>>,
    /// Hex SHA-256 of the bytes, declared with the report.
    pub sha256: String,
}

/// What the page shows of a staged file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StagedAttachment {
    /// Handle for `feedback_attachment_remove` and `feedback_submit`.
    pub id: String,
    /// Cleaned file name.
    pub name: String,
    /// MIME type.
    #[serde(rename = "type")]
    pub mime: String,
    /// Bytes.
    pub size: usize,
}

impl Staged {
    fn view(&self) -> StagedAttachment {
        StagedAttachment { id: self.id.clone(), name: self.name.clone(), mime: self.mime.clone(), size: self.bytes.len() }
    }
}

/// The files staged for the next report (managed state). Only what the user picked on the 反馈
/// page, in memory, until the report went out or the page removed them.
#[derive(Default)]
pub struct Attachments {
    staged: Mutex<Vec<Staged>>,
}

impl Attachments {
    /// Stage `bytes` as `name` (`mime`) within the limits.
    pub fn add(&self, name: &str, mime: &str, bytes: Vec<u8>) -> Result<StagedAttachment, AttachError> {
        let name = clean_name(name)?;
        if !ATTACHMENT_TYPES.contains(&mime) {
            return Err(AttachError::Type);
        }
        let limit = if mime.starts_with("video/") { MAX_VIDEO_BYTES } else { MAX_IMAGE_BYTES };
        if bytes.is_empty() || bytes.len() > limit {
            return Err(AttachError::TooLarge);
        }
        let mut staged = self.staged.lock();
        if staged.len() >= MAX_ATTACHMENTS {
            return Err(AttachError::TooMany);
        }
        if staged.iter().map(|s| s.bytes.len()).sum::<usize>() + bytes.len() > MAX_ATTACHMENT_TOTAL_BYTES {
            return Err(AttachError::Total);
        }
        let sha256 = hex::encode(Sha256::digest(&bytes));
        let entry = Staged { id: uuid::Uuid::new_v4().to_string(), name, mime: mime.to_owned(), bytes: Arc::new(bytes), sha256 };
        let view = entry.view();
        staged.push(entry);
        Ok(view)
    }

    /// Drop a staged file; whether it was there.
    pub fn remove(&self, id: &str) -> bool {
        let mut staged = self.staged.lock();
        let before = staged.len();
        staged.retain(|s| s.id != id);
        staged.len() != before
    }

    /// The staged files `ids` name, in that order; an id that is not staged makes the report
    /// invalid (the page and the shell disagree about what goes along).
    pub fn pick(&self, ids: &[String]) -> Result<Vec<Staged>, SendError> {
        let staged = self.staged.lock();
        ids.iter().map(|id| staged.iter().find(|s| &s.id == id).cloned().ok_or(SendError::Invalid)).collect()
    }

    /// Forget the files of a report that went out.
    pub fn forget(&self, ids: &[String]) {
        self.staged.lock().retain(|s| !ids.contains(&s.id));
    }

    /// Forget every staged file (the page opened afresh, or was left).
    pub fn clear(&self) {
        self.staged.lock().clear();
    }
}

/// A file name the endpoint takes: the last path component, trimmed, without control characters,
/// quotes or separators, and at most [`MAX_ATTACHMENT_NAME_CHARS`] characters (a longer one keeps
/// its extension).
pub fn clean_name(name: &str) -> Result<String, AttachError> {
    let base = name.rsplit(['/', '\\']).next().unwrap_or_default().trim();
    let cleaned: String = base.chars().map(|c| if c.is_control() || c == '"' { '_' } else { c }).collect();
    if cleaned.is_empty() {
        return Err(AttachError::Name);
    }
    if cleaned.chars().count() <= MAX_ATTACHMENT_NAME_CHARS {
        return Ok(cleaned);
    }
    let ext = cleaned.rfind('.').map(|i| &cleaned[i..]).filter(|e| e.chars().count() <= 10).unwrap_or("");
    let stem: String = cleaned.chars().take(MAX_ATTACHMENT_NAME_CHARS - ext.chars().count()).collect();
    Ok(format!("{}{ext}", stem.trim_end_matches(ext)))
}

/// `%XX` escapes decoded (the page sends the file name percent-encoded in a header); anything
/// that is not valid UTF-8 afterwards is replaced.
pub fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| char::from(b).to_digit(16).and_then(|d| u8::try_from(d).ok());
        if bytes[i] == b'%'
            && let (Some(hi), Some(lo)) = (bytes.get(i + 1).copied().and_then(hex), bytes.get(i + 2).copied().and_then(hex))
        {
            out.push(hi * 16 + lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
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
    /// The endpoint's attachment storage is full.
    StorageFull,
    /// The report went out, but an attachment did not finish uploading.
    Attachments,
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
            Self::StorageFull => "storage_full",
            Self::Attachments => "attachments",
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<Declared<'a>>,
}

/// An attachment as the report declares it; the bytes follow.
#[derive(Serialize)]
struct Declared<'a> {
    name: &'a str,
    #[serde(rename = "type")]
    mime: &'a str,
    size: usize,
    sha256: &'a str,
}

/// Post one report to `url`, then the `attachments`' bytes in the chunks the endpoint asks for.
/// Errors carry no host: the endpoint is a build secret.
pub async fn send(
    url: &str,
    token: Option<&str>,
    kind: FeedbackKind,
    message: &str,
    contact: Option<&str>,
    diagnostics: &Diagnostics,
    attachments: &[Staged],
) -> Result<Receipt, SendError> {
    let http = reqwest::Client::builder().user_agent(concat!("voltip-desktop/", env!("VOLTIP_APP_VERSION"))).build().map_err(|_| SendError::Network)?;
    let token = token.filter(|t| !t.is_empty());
    let declared = attachments.iter().map(|a| Declared { name: &a.name, mime: &a.mime, size: a.bytes.len(), sha256: &a.sha256 }).collect();
    let mut request = http.post(url).timeout(SEND_TIMEOUT).json(&Payload { kind, message, contact, diagnostics, attachments: declared });
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let response = request.send().await.map_err(|e| if e.is_timeout() { SendError::Timeout } else { SendError::Network })?;
    let answer = match response.status().as_u16() {
        200 | 201 => response.json::<Answer>().await.map_err(|_| SendError::Server)?,
        400 | 413 | 415 => return Err(SendError::Invalid),
        401 | 403 => return Err(SendError::Unauthorized),
        429 => return Err(SendError::RateLimited),
        507 => return Err(SendError::StorageFull),
        _ => return Err(SendError::Server),
    };
    if attachments.is_empty() {
        return Ok(Receipt { id: answer.id });
    }
    // The id goes into the upload paths: only what the endpoint hands out (a UUID) is taken.
    let safe_id = !answer.id.is_empty() && answer.id.len() <= 64 && answer.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    let ticket = answer.upload.filter(|t| safe_id && (1..=MAX_CHUNK_BYTES).contains(&t.chunk_bytes)).ok_or(SendError::Attachments)?;
    let base = url.trim_end_matches('/');
    for (idx, attachment) in attachments.iter().enumerate() {
        for (seq, chunk) in attachment.bytes.chunks(ticket.chunk_bytes).enumerate() {
            let target = format!("{base}/{}/attachments/{idx}/{seq}", answer.id);
            put_chunk(&http, &target, token, &ticket.token, chunk).await?;
        }
    }
    Ok(Receipt { id: answer.id })
}

/// One chunk, tried [`CHUNK_ATTEMPTS`] times on a network failure or a 5xx; a chunk the endpoint
/// already has in full (409) counts as sent.
async fn put_chunk(http: &reqwest::Client, url: &str, token: Option<&str>, upload_token: &str, chunk: &[u8]) -> Result<(), SendError> {
    for attempt in 1..=CHUNK_ATTEMPTS {
        let mut request =
            http.put(url).timeout(CHUNK_TIMEOUT).header("content-type", "application/octet-stream").header("x-upload-token", upload_token).body(chunk.to_vec());
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        match request.send().await {
            Ok(response) if response.status().is_success() || response.status().as_u16() == 409 => return Ok(()),
            Ok(response) if response.status().is_server_error() && attempt < CHUNK_ATTEMPTS => {
                tracing::debug!(status = response.status().as_u16(), attempt, "feedback chunk refused; trying again");
            }
            Ok(response) => {
                tracing::warn!(status = response.status().as_u16(), "feedback chunk refused");
                return Err(SendError::Attachments);
            }
            Err(e) if attempt < CHUNK_ATTEMPTS => tracing::debug!(timeout = e.is_timeout(), attempt, "feedback chunk failed; trying again"),
            Err(_) => return Err(SendError::Attachments),
        }
    }
    Err(SendError::Attachments)
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
            SendError::StorageFull,
            SendError::Attachments,
        ]
        .map(SendError::as_str)
        .to_vec();
        assert_eq!(names, ["not_configured", "invalid", "rate_limited", "unauthorized", "network", "timeout", "server", "storage_full", "attachments"]);
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
        let receipt = send(&url, Some("app-token"), FeedbackKind::Bug, "no paste", Some("me@example.test"), &d, &[]).await.unwrap();
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
            (507, SendError::StorageFull),
            (500, SendError::Server),
            (302, SendError::Server),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST")).respond_with(ResponseTemplate::new(status)).mount(&server).await;
            let got = send(&server.uri(), None, FeedbackKind::Idea, "x", None, &d, &[]).await;
            assert_eq!(got, Err(want), "HTTP {status}");
        }
        // A 201 that is not a receipt is the endpoint's failure.
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(201).set_body_string("ok")).mount(&server).await;
        assert_eq!(send(&server.uri(), None, FeedbackKind::Other, "x", None, &d, &[]).await, Err(SendError::Server));
        // Nothing listening: a network failure, and no request went out without the token check.
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = closed.local_addr().unwrap().port();
        drop(closed);
        assert_eq!(send(&format!("http://127.0.0.1:{port}/v1/feedback"), Some(""), FeedbackKind::Other, "x", None, &d, &[]).await, Err(SendError::Network));
    }

    fn png(n: usize) -> Vec<u8> {
        vec![0x89; n]
    }

    /// Regression (user feedback 2026-09-28): a report may carry screenshots and screen
    /// recordings, within the endpoint's limits.
    #[test]
    fn attachments_are_staged_within_the_limits() {
        let staged = Attachments::default();
        let shot = staged.add("C:\\Users\\me\\屏幕截图.png", "image/png", png(10)).unwrap();
        assert_eq!((shot.name.as_str(), shot.mime.as_str(), shot.size), ("屏幕截图.png", "image/png", 10));
        assert_eq!(staged.add("a.pdf", "application/pdf", png(1)), Err(AttachError::Type));
        assert_eq!(staged.add("big.png", "image/png", png(MAX_IMAGE_BYTES + 1)), Err(AttachError::TooLarge));
        assert_eq!(staged.add("empty.png", "image/png", Vec::new()), Err(AttachError::TooLarge));
        assert_eq!(staged.add("  ", "image/png", png(1)), Err(AttachError::Name));
        let clip = staged.add("clip.mp4", "video/mp4", png(MAX_VIDEO_BYTES)).unwrap();
        // 10 B + 20 MiB staged: another 5 MiB would pass the 25 MiB total.
        assert_eq!(staged.add("two.png", "image/png", png(MAX_IMAGE_BYTES)), Err(AttachError::Total));
        staged.add("three.png", "image/png", png(1)).unwrap();
        assert_eq!(staged.add("four.png", "image/png", png(1)), Err(AttachError::TooMany));
        assert!(staged.remove(&clip.id));
        assert!(!staged.remove(&clip.id));
        let picked = staged.pick(std::slice::from_ref(&shot.id)).unwrap();
        assert_eq!(picked[0].sha256, hex::encode(Sha256::digest(png(10))));
        assert_eq!(staged.pick(&["nope".to_owned()]).unwrap_err(), SendError::Invalid);
        staged.forget(std::slice::from_ref(&shot.id));
        assert!(staged.pick(std::slice::from_ref(&shot.id)).is_err());
        let names: Vec<&str> =
            [AttachError::Type, AttachError::TooLarge, AttachError::TooMany, AttachError::Total, AttachError::Name].map(AttachError::as_str).to_vec();
        assert_eq!(names, ["attachment_type", "attachment_too_large", "attachment_too_many", "attachment_total", "attachment_name"]);
    }

    #[test]
    fn file_names_are_cleaned_and_shortened_keeping_the_extension() {
        assert_eq!(clean_name("/tmp/a/b.png").unwrap(), "b.png");
        assert_eq!(clean_name(" say \"hi\"\u{7}.webm ").unwrap(), "say _hi__.webm");
        let long = format!("{}.mov", "长".repeat(200));
        let short = clean_name(&long).unwrap();
        assert_eq!(short.chars().count(), MAX_ATTACHMENT_NAME_CHARS);
        assert!(short.ends_with(".mov"), "{short}");
        assert_eq!(clean_name("dir/").unwrap_err(), AttachError::Name);
        assert_eq!(percent_decode("%E5%B1%8F%E5%B9%95.png"), "屏幕.png");
        assert_eq!(percent_decode("100%25 %zz%4"), "100% %zz%4");
    }

    /// The report declares its attachments, the endpoint answers with an upload ticket, and every
    /// chunk goes out with the upload token; a 5xx chunk is tried again.
    #[tokio::test]
    async fn attachments_follow_the_report_in_chunks() {
        use wiremock::matchers::path_regex;
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/feedback"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({ "id": "f-2", "upload": { "token": "up", "chunk_bytes": 4 } })))
            .mount(&server)
            .await;
        // The very first chunk fails once with a 503, then everything succeeds.
        Mock::given(method("PUT")).and(path("/v1/feedback/f-2/attachments/0/0")).respond_with(ResponseTemplate::new(503)).up_to_n_times(1).mount(&server).await;
        Mock::given(method("PUT"))
            .and(path_regex(r"^/v1/feedback/f-2/attachments/\d/\d$"))
            .and(header("x-upload-token", "up"))
            .and(header("authorization", "Bearer app-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "received": 1, "complete": false })))
            .mount(&server)
            .await;
        let staged = Attachments::default();
        let shot = staged.add("shot.png", "image/png", b"0123456789".to_vec()).unwrap();
        let clip = staged.add("clip.webm", "video/webm", b"abc".to_vec()).unwrap();
        let files = staged.pick(&[shot.id, clip.id]).unwrap();
        let d = diagnostics(&state(), "en", None);
        let url = format!("{}/v1/feedback", server.uri());
        let receipt = send(&url, Some("app-token"), FeedbackKind::Bug, "look", None, &d, &files).await.unwrap();
        assert_eq!(receipt.id, "f-2");
        let seen = server.received_requests().await.unwrap();
        let report: serde_json::Value = serde_json::from_slice(&seen[0].body).unwrap();
        assert_eq!(report["attachments"][0]["name"], "shot.png");
        assert_eq!(report["attachments"][0]["type"], "image/png");
        assert_eq!(report["attachments"][0]["size"], 10);
        assert_eq!(report["attachments"][1]["sha256"], hex::encode(Sha256::digest(b"abc")));
        let puts: Vec<(String, Vec<u8>)> = seen[1..].iter().map(|r| (r.url.path().to_owned(), r.body.clone())).collect();
        assert_eq!(
            puts,
            [
                ("/v1/feedback/f-2/attachments/0/0".to_owned(), b"0123".to_vec()),
                ("/v1/feedback/f-2/attachments/0/0".to_owned(), b"0123".to_vec()),
                ("/v1/feedback/f-2/attachments/0/1".to_owned(), b"4567".to_vec()),
                ("/v1/feedback/f-2/attachments/0/2".to_owned(), b"89".to_vec()),
                ("/v1/feedback/f-2/attachments/1/0".to_owned(), b"abc".to_vec()),
            ]
        );
    }

    #[tokio::test]
    async fn an_upload_that_cannot_finish_says_the_report_went_out_without_it() {
        let d = diagnostics(&state(), "en", None);
        let staged = Attachments::default();
        let shot = staged.add("shot.png", "image/png", b"0123".to_vec()).unwrap();
        let files = staged.pick(&[shot.id]).unwrap();
        for (answer, chunk_status) in [
            (serde_json::json!({ "id": "f-3", "upload": { "token": "up", "chunk_bytes": 4 } }), 410),
            (serde_json::json!({ "id": "f-3" }), 200),
            (serde_json::json!({ "id": "../x", "upload": { "token": "up", "chunk_bytes": 4 } }), 200),
            (serde_json::json!({ "id": "f-3", "upload": { "token": "up", "chunk_bytes": 0 } }), 200),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST")).respond_with(ResponseTemplate::new(201).set_body_json(answer.clone())).mount(&server).await;
            Mock::given(method("PUT")).respond_with(ResponseTemplate::new(chunk_status)).mount(&server).await;
            let got = send(&format!("{}/v1/feedback", server.uri()), None, FeedbackKind::Bug, "x", None, &d, &files).await;
            assert_eq!(got, Err(SendError::Attachments), "{answer}");
        }
        // A chunk the endpoint already has counts as sent.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({ "id": "f-4", "upload": { "token": "up", "chunk_bytes": 4 } })))
            .mount(&server)
            .await;
        Mock::given(method("PUT")).respond_with(ResponseTemplate::new(409)).mount(&server).await;
        assert!(send(&format!("{}/v1/feedback", server.uri()), None, FeedbackKind::Bug, "x", None, &d, &files).await.is_ok());
    }
}
