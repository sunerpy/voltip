//! What a speech or AI service said when it refused a request (docs/dictation.md §3.12). The
//! recognition and AI polish clients (`voltip-asr`, `voltip-refine`) put it in their errors, so the
//! pill, the history and 测试连接 show the service's own words, not just 请求失败.
//!
//! * [`error_fields`] — the error's `code` and `message`, in every shape the providers send:
//!   OpenAI's `{"error": {"code", "message"}}` (Groq, DeepSeek, Ollama's compatible API, vLLM),
//!   Model Studio's and SiliconFlow's `{"code", "message"}`, Google AI Studio's
//!   `[{"error": {"code", "message", "status"}}]`, Ollama's `{"error": "…"}` and FastAPI's
//!   `{"detail": …}`.
//! * [`summary`] — the words to show: the message with its code, an HTML page's title, or the
//!   text, through [`clean`].
//! * [`clean`] — whitespace collapsed, keys, tokens and account ids masked, cut to
//!   [`MAX_SUMMARY_CHARS`]. A refusal reaches the interface and the history, and the built-in
//!   service's account must not.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

/// Longest text [`clean`] keeps, in characters (the ellipsis included).
pub const MAX_SUMMARY_CHARS: usize = 200;

/// The `code` and `message` of an error body ([`error_fields`]); empty strings when it has none.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ErrorFields {
    /// The service's error code: a string `code`, else Google's `status` (`UNAVAILABLE`), else
    /// OpenAI's `type`, else a numeric `code` as digits.
    pub code: String,
    /// The service's error message, as it sent it.
    pub message: String,
}

/// The `code` and `message` of an error `body`, in the shapes the module documentation lists. A
/// body that is not JSON, or JSON without them, gives empty strings.
pub fn error_fields(body: &str) -> ErrorFields {
    serde_json::from_str::<Value>(body.trim()).map(|value| fields_of(&value)).unwrap_or_default()
}

fn fields_of(value: &Value) -> ErrorFields {
    // Google AI Studio wraps its error in an array (2026-10-24).
    let value = match value {
        Value::Array(items) => match items.first() {
            Some(first) => first,
            None => return ErrorFields::default(),
        },
        other => other,
    };
    match value.get("error") {
        Some(inner @ Value::Object(_)) => ErrorFields { code: code_of(inner), message: message_of(inner) },
        // Ollama's native API: `{"error": "model 'x' not found"}`.
        Some(Value::String(message)) => ErrorFields { code: code_of(value), message: message.clone() },
        _ => ErrorFields { code: code_of(value), message: message_of(value) },
    }
}

fn code_of(value: &Value) -> String {
    let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
    text("code").or_else(|| text("status")).or_else(|| text("type")).unwrap_or_else(|| match value.get("code") {
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    })
}

fn message_of(value: &Value) -> String {
    for key in ["message", "msg", "error_message", "errorMessage"] {
        if let Some(message) = value.get(key).and_then(Value::as_str) {
            return message.to_owned();
        }
    }
    match value.get("detail") {
        Some(Value::String(detail)) => detail.clone(),
        // FastAPI's validation errors: `[{"loc": […], "msg": "…", "type": "…"}]`.
        Some(Value::Array(items)) => items.iter().filter_map(|item| item.get("msg").and_then(Value::as_str)).collect::<Vec<_>>().join("; "),
        _ => String::new(),
    }
}

/// The service's own words in an error `body`, through [`clean`]: the message of a JSON error,
/// with its code when the message does not name it (an HTTP-like three-digit code says nothing
/// the status does not); the title of an HTML page, or its text; other text as it is. Empty when
/// the body says nothing.
pub fn summary(body: &str) -> String {
    let body = body.trim();
    let said = match serde_json::from_str::<Value>(body) {
        Ok(value) => {
            let ErrorFields { code, message } = fields_of(&value);
            let message = message.trim();
            let named = code.is_empty() || message.contains(code.as_str()) || is_status_like(&code);
            match (message.is_empty(), named) {
                (true, _) if !code.is_empty() => code,
                // JSON without a message: shown as it came, compact, unless it holds nothing.
                (true, _) => match &value {
                    Value::Null => String::new(),
                    Value::Object(map) if map.is_empty() => String::new(),
                    Value::Array(items) if items.is_empty() => String::new(),
                    other => other.to_string(),
                },
                (false, true) => message.to_owned(),
                (false, false) => format!("{message} ({code})"),
            }
        }
        Err(_) => page_text(body),
    };
    clean(&said)
}

/// A numeric code of at most three digits: Google's `code: 503`, vLLM's `code: 400`.
fn is_status_like(code: &str) -> bool {
    code.len() <= 3 && code.bytes().all(|b| b.is_ascii_digit())
}

/// The title of an HTML page (an nginx 502, a captive portal), else its text without tags; other
/// text as it is.
fn page_text(body: &str) -> String {
    let lower = body.to_ascii_lowercase();
    if !["<html", "<!doctype", "<title", "<body", "<head"].iter().any(|tag| lower.contains(tag)) {
        return body.to_owned();
    }
    if let (Some(open), Some(close)) = (lower.find("<title"), lower.find("</title>"))
        && let Some(start) = lower[open..].find('>').map(|i| open + i + 1)
        && start <= close
    {
        let title = body[start..close].trim();
        if !title.is_empty() {
            return title.to_owned();
        }
    }
    PATTERNS.as_ref().map_or_else(String::new, |patterns| patterns.tags.replace_all(body, " ").into_owned())
}

/// `text` for the interface: whitespace collapsed, credentials and account ids masked (`sk-***`),
/// cut to [`MAX_SUMMARY_CHARS`] characters with an ellipsis.
pub fn clean(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    mask(&collapsed).map_or_else(String::new, |masked| shorten(&masked, MAX_SUMMARY_CHARS))
}

/// The patterns [`clean`] masks with, and the tags of an HTML page; `None` only if one of these
/// literals did not compile, which a test rules out.
struct Patterns {
    tags: Regex,
    /// `Bearer <token>`.
    bearer: Regex,
    /// Provider keys by their prefixes (OpenAI and its copies `sk-`, Groq `gsk_`, xAI `xai-`,
    /// Xiaomi MiMo's plan keys `tp-` and `ttp-`) and Google's `AIza…`, masked keys included
    /// (`sk-proj-ab****yz`).
    keys: Regex,
    /// Account ids: Groq's `org_01…` names the built-in service's account in its rate-limit
    /// message; OpenAI's `org-…`.
    orgs: Regex,
    /// A credential in a URL's query: `?key=…`, `&token=…`.
    query: Regex,
    /// Any other opaque token of 32 or more letters, digits, `-` and `_`, with the dotted parts
    /// that follow it: Zhipu's keys are `<32 hex>.<16 letters and digits>`, a JWT has three parts.
    opaque: Regex,
}

impl Patterns {
    fn compile() -> Option<Self> {
        Some(Self {
            tags: Regex::new(r"<[^>]*>").ok()?,
            bearer: Regex::new(r"(?i)\b(bearer)\s+[A-Za-z0-9._~+/=-]{8,}").ok()?,
            keys: Regex::new(r"\b(sk|gsk|xai|rk|pk|tp|ttp)([-_])[A-Za-z0-9*_-]{6,}|\b(AIza)[0-9A-Za-z_*-]{10,}").ok()?,
            orgs: Regex::new(r"\b(org)([-_])[A-Za-z0-9]{6,}").ok()?,
            query: Regex::new(r"(?i)([?&](?:key|api_key|apikey|token|access_token|signature|sig)=)[^&\s]+").ok()?,
            opaque: Regex::new(r"\b[A-Za-z0-9_-]{32,}(?:\.[A-Za-z0-9_-]{8,})*\b").ok()?,
        })
    }
}

static PATTERNS: LazyLock<Option<Patterns>> = LazyLock::new(Patterns::compile);

/// `text` with credentials and account ids masked; `None` when the patterns are missing, so
/// nothing unmasked is ever shown.
fn mask(text: &str) -> Option<String> {
    let patterns = PATTERNS.as_ref()?;
    let text = patterns.bearer.replace_all(text, "$1 ***");
    let text = patterns.keys.replace_all(&text, |caps: &regex::Captures<'_>| match caps.get(3) {
        Some(google) => format!("{}***", google.as_str()),
        None => format!("{}{}***", &caps[1], &caps[2]),
    });
    let text = patterns.orgs.replace_all(&text, "$1$2***");
    let text = patterns.query.replace_all(&text, "$1***");
    // Mixed letters and digits only: a long word or a path segment of letters stays readable.
    let text = patterns.opaque.replace_all(&text, |caps: &regex::Captures<'_>| {
        let token = &caps[0];
        let mixed = token.bytes().any(|b| b.is_ascii_digit()) && token.bytes().any(|b| b.is_ascii_alphabetic());
        if mixed { "***".to_owned() } else { token.to_owned() }
    });
    Some(text.into_owned())
}

fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(max.saturating_sub(1)).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(code: &str, message: &str) -> ErrorFields {
        ErrorFields { code: code.into(), message: message.into() }
    }

    #[test]
    fn every_pattern_compiles() {
        assert!(PATTERNS.is_some());
    }

    #[test]
    fn the_providers_error_shapes_give_their_code_and_message() {
        // OpenAI and the services copying it.
        let openai = r#"{"error":{"message":"Incorrect API key provided","type":"invalid_request_error","param":null,"code":"invalid_api_key"}}"#;
        assert_eq!(error_fields(openai), fields("invalid_api_key", "Incorrect API key provided"));
        assert_eq!(error_fields(r#"{"error":{"message":"m","type":"insufficient_quota","code":null}}"#), fields("insufficient_quota", "m"));
        // Model Studio, SiliconFlow.
        assert_eq!(error_fields(r#"{"code":"AllocationQuota.FreeTierOnly","message":"m","request_id":"r"}"#), fields("AllocationQuota.FreeTierOnly", "m"));
        assert_eq!(error_fields(r#"{"code":20012,"message":"Model does not exist.","data":null}"#), fields("20012", "Model does not exist."));
        // Google AI Studio (2026-10-24).
        let google = r#"[{"error":{"code":503,"message":"This model is currently experiencing high demand.","status":"UNAVAILABLE"}}]"#;
        assert_eq!(error_fields(google), fields("UNAVAILABLE", "This model is currently experiencing high demand."));
        // Ollama's native API, FastAPI, vLLM's older form.
        assert_eq!(error_fields(r#"{"error":"model 'x' not found"}"#), fields("", "model 'x' not found"));
        assert_eq!(error_fields(r#"{"detail":"Not Found"}"#), fields("", "Not Found"));
        assert_eq!(error_fields(r#"{"detail":[{"loc":["body","file"],"msg":"field required","type":"missing"}]}"#), fields("", "field required"));
        assert_eq!(
            error_fields(r#"{"object":"error","message":"bad audio","type":"BadRequestError","param":null,"code":400}"#),
            fields("BadRequestError", "bad audio")
        );
        // Nothing to read.
        assert_eq!(error_fields(r#"{"code":7,"message":null}"#), fields("7", ""));
        assert_eq!(error_fields("bad gateway"), ErrorFields::default());
        assert_eq!(error_fields("[]"), ErrorFields::default());
    }

    #[test]
    fn a_summary_is_the_services_words_with_their_code() {
        let openai = r#"{"error":{"message":"Incorrect API key provided.","type":"invalid_request_error","code":"invalid_api_key"}}"#;
        assert_eq!(summary(openai), "Incorrect API key provided. (invalid_api_key)");
        let google = r#"[{"error":{"code":503,"message":"This model is currently experiencing high demand.","status":"UNAVAILABLE"}}]"#;
        assert_eq!(summary(google), "This model is currently experiencing high demand. (UNAVAILABLE)");
        assert_eq!(summary(r#"{"code":"InvalidParameter","message":"InvalidParameter: the audio is too long"}"#), "InvalidParameter: the audio is too long");
        assert_eq!(summary(r#"{"object":"error","message":"bad audio","code":400}"#), "bad audio", "a status-like code says nothing new");
        assert_eq!(summary(r#"{"code":"Throttling"}"#), "Throttling");
        assert_eq!(summary(r#"{"ok":false}"#), r#"{"ok":false}"#);
        assert_eq!((summary("{}"), summary("[]"), summary("null")), (String::new(), String::new(), String::new()));
        assert_eq!(summary("  upstream connect error\n\n or disconnect  "), "upstream connect error or disconnect");
        assert_eq!(summary(""), "");
    }

    #[test]
    fn an_html_page_gives_its_title_or_its_text() {
        let nginx = "<html>\r\n<head><title>502 Bad Gateway</title></head>\r\n<body>\r\n<center><h1>502 Bad Gateway</h1></center>\r\n<hr><center>nginx</center>\r\n</body>\r\n</html>";
        assert_eq!(summary(nginx), "502 Bad Gateway");
        assert_eq!(summary("<!DOCTYPE html><html><body><p>Service <b>down</b></p></body></html>"), "Service down");
    }

    /// Regression (2026-10-24): a refusal reaches the pill and the history now, and services name
    /// keys and accounts in theirs: OpenAI repeats a wrong key half-masked, Groq names the
    /// organization, which for the built-in service is the project's own account.
    #[test]
    fn regression_keys_tokens_and_accounts_are_masked() {
        assert_eq!(
            clean("Incorrect API key provided: sk-proj-abcd****************wxyz. You can find your API key at https://platform.openai.com/account/api-keys."),
            "Incorrect API key provided: sk-***. You can find your API key at https://platform.openai.com/account/api-keys."
        );
        assert_eq!(clean("key gsk_abcdefghijklmnop rejected"), "key gsk_*** rejected");
        assert_eq!(clean("API key AIzaSyA1234567890abcdefghij is invalid"), "API key AIza*** is invalid");
        assert_eq!(
            clean("Rate limit reached for model `qwen/qwen3.8-27b` in organization `org_01jabcdefgh12345` service tier `on_demand`"),
            "Rate limit reached for model `qwen/qwen3.8-27b` in organization `org_***` service tier `on_demand`"
        );
        assert_eq!(clean("Authorization: Bearer abc.def-123456789"), "Authorization: Bearer ***");
        assert_eq!(clean("see https://example.test/v1/x?key=secret&lang=zh"), "see https://example.test/v1/x?key=***&lang=zh");
        assert_eq!(clean("request 3f9c2a7b1e4d8f6a0b5c9d2e7f1a3b4c failed"), "request *** failed");
        // Zhipu's `<id>.<secret>` keys whole, and MiMo's plan keys (2026-10-25).
        assert_eq!(clean("key 0123456789abcdef0123456789abcdef.AbCdEfGh12345678 is invalid"), "key *** is invalid");
        assert_eq!(clean("token tp-abcdef123456 expired"), "token tp-*** expired");
        // Words are not tokens: a task id stays, and so do model names and long words.
        assert_eq!(clean("task-failed for qwen3-asr-flash-realtime"), "task-failed for qwen3-asr-flash-realtime");
        assert_eq!(clean("internationalizationlocalizationaccessibility"), "internationalizationlocalizationaccessibility");
    }

    #[test]
    fn a_long_text_is_cut_with_an_ellipsis() {
        let long = "字".repeat(300);
        let cut = clean(&long);
        assert_eq!(cut.chars().count(), MAX_SUMMARY_CHARS);
        assert!(cut.ends_with('…'));
        assert_eq!(clean(&"a".repeat(MAX_SUMMARY_CHARS)).chars().count(), MAX_SUMMARY_CHARS, "exactly the limit stays whole");
    }
}
