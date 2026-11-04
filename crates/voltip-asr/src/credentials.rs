//! A credential of several fields, as the core stores it (docs/dictation.md §3.15): one JSON
//! object in the provider's secret-store entry, naming its set, for example
//! `{"set":"app_token","app_id":"…","access_token":"…"}`. The core checks the fields when it saves
//! them (`voltip_core::providers::merge_credentials`); a client reads its own struct out of it and
//! fails on what is missing, so a hand-edited entry cannot send a half-made request.

use serde::de::DeserializeOwned;

use crate::error::AsrError;

/// The fields in `token`, read into `T` (whose fields are the set's, `#[serde(default)]` for the
/// ones that may be missing). The `set` key is ignored here: [`set_of`] reads it.
pub fn parse<T: DeserializeOwned>(token: Option<&str>) -> Result<T, AsrError> {
    let raw = token.map(str::trim).filter(|t| !t.is_empty()).ok_or_else(|| AsrError::InvalidConfig("the credential is missing".into()))?;
    serde_json::from_str(raw).map_err(|e| AsrError::InvalidConfig(format!("the credential is incomplete: {}", field_error(&e))))
}

/// The set the credential in `token` names, if it names one.
pub fn set_of(token: Option<&str>) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(token?.trim()).ok()?;
    value.get("set")?.as_str().map(str::to_owned)
}

/// A field that is missing or blank, as an error.
pub fn require(value: &str, field: &str) -> Result<String, AsrError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(AsrError::InvalidConfig(format!("the credential is incomplete: {field} is empty")));
    }
    Ok(value.to_owned())
}

/// What serde said, without the values: a missing field by its name ("missing field `app_id`"),
/// anything else (a value of the wrong type, not JSON at all) without quoting what it read.
fn field_error(error: &serde_json::Error) -> String {
    let text = error.to_string();
    let head = text.split(" at line").next().unwrap_or(&text);
    if head.starts_with("missing field") { head.to_owned() } else { "it is not an object of text fields".to_owned() }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Pair {
        app_id: String,
        api_key: String,
    }

    #[test]
    fn a_stored_credential_reads_into_its_fields() {
        let stored = r#"{"set":"default","app_id":"1234","api_key":"k"}"#;
        assert_eq!(parse::<Pair>(Some(stored)), Ok(Pair { app_id: "1234".into(), api_key: "k".into() }));
        assert_eq!(set_of(Some(stored)).as_deref(), Some("default"));
        assert_eq!(set_of(Some("plain-api-key")), None);
    }

    #[test]
    fn a_missing_field_or_credential_is_named_without_values() {
        let err = parse::<Pair>(Some(r#"{"app_id":"secret-looking-1234"}"#)).unwrap_err();
        assert_eq!(err, AsrError::InvalidConfig("the credential is incomplete: missing field `api_key`".into()));
        assert!(!err.to_string().contains("secret-looking"));
        assert_eq!(parse::<Pair>(None), Err(AsrError::InvalidConfig("the credential is missing".into())));
        assert_eq!(parse::<Pair>(Some("  ")), Err(AsrError::InvalidConfig("the credential is missing".into())));
        assert!(matches!(parse::<Pair>(Some("plain-api-key")), Err(AsrError::InvalidConfig(_))));
        assert_eq!(require(" v ", "app_id"), Ok("v".into()));
        assert_eq!(require(" ", "app_id"), Err(AsrError::InvalidConfig("the credential is incomplete: app_id is empty".into())));
    }
}
