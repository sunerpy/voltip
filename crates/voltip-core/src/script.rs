//! Chinese script normalisation of the recogniser's text (docs/dictation.md §17).
//!
//! Recognisers disagree on the script: the default local model (Qwen3-ASR 0.6B through
//! transcribe.cpp, which takes no language hint) answers some Mandarin in Traditional characters
//! where SenseVoice answers the same audio in Simplified. [`normalize`] brings every recogniser's
//! text — whole takes, streaming partials, committed sentences, flushes, `live_inject` remainders —
//! to [`crate::ChineseScript`] right after recognition and before the dictionary corrections, so the
//! dictionary always sees the chosen script.
//!
//! The converter is `ferrous-opencc` (pure-Rust OpenCC, Apache-2.0): `t2s` for Simplified, `s2t` for
//! Traditional, each built once per process on first use (≈ 0.1 ms / 1.6 ms) from the dictionaries
//! embedded at build time. Text without a Han character is returned untouched without looking at a
//! dictionary; a converter that fails to build or panics returns the text unchanged (fail-open).

use std::borrow::Cow;
use std::panic::AssertUnwindSafe;
use std::sync::OnceLock;

use ferrous_opencc::OpenCC;
use ferrous_opencc::config::BuiltinConfig;

use crate::engines::ChineseScript;

static TO_SIMPLIFIED: OnceLock<Option<OpenCC>> = OnceLock::new();
static TO_TRADITIONAL: OnceLock<Option<OpenCC>> = OnceLock::new();

/// A Han ideograph (the characters OpenCC maps between the scripts).
fn is_han(c: char) -> bool {
    matches!(c as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x3134F)
}

fn converter(script: ChineseScript) -> Option<&'static OpenCC> {
    let (cell, config) = match script {
        ChineseScript::AsIs => return None,
        ChineseScript::Simplified => (&TO_SIMPLIFIED, BuiltinConfig::T2s),
        ChineseScript::Traditional => (&TO_TRADITIONAL, BuiltinConfig::S2t),
    };
    cell.get_or_init(|| match OpenCC::from_config(config) {
        Ok(converter) => Some(converter),
        Err(e) => {
            tracing::warn!(script = script.as_str(), error = %e, "Chinese script converter unavailable; text stays as recognised");
            None
        }
    })
    .as_ref()
}

/// `text` in `script`: borrowed when nothing changes (as-is, no Han character, already in the
/// script, or the converter unavailable), owned otherwise.
pub fn normalize(script: ChineseScript, text: &str) -> Cow<'_, str> {
    if script == ChineseScript::AsIs || !text.chars().any(is_han) {
        return Cow::Borrowed(text);
    }
    let Some(converter) = converter(script) else { return Cow::Borrowed(text) };
    match std::panic::catch_unwind(AssertUnwindSafe(|| converter.convert(text))) {
        Ok(converted) if converted != text => Cow::Owned(converted),
        Ok(_) => Cow::Borrowed(text),
        Err(_) => {
            tracing::warn!(script = script.as_str(), "Chinese script conversion panicked; text stays as recognised");
            Cow::Borrowed(text)
        }
    }
}

/// [`normalize`] into an owned `String`.
pub fn normalized(script: ChineseScript, text: &str) -> String {
    normalize(script, text).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Qwen3-ASR answer to the public `zh.wav` sample (docs/dictation.md §17).
    const QWEN3_TRADITIONAL: &str = "開放時間：早上九點至下午五點。";
    const SIMPLIFIED: &str = "开放时间：早上九点至下午五点。";

    #[test]
    fn traditional_becomes_simplified_by_default() {
        assert_eq!(ChineseScript::default(), ChineseScript::Simplified);
        assert_eq!(normalize(ChineseScript::Simplified, QWEN3_TRADITIONAL), SIMPLIFIED);
        assert!(matches!(normalize(ChineseScript::Simplified, SIMPLIFIED), Cow::Borrowed(_)), "already simplified: untouched");
        assert_eq!(normalize(ChineseScript::Simplified, "集成在Teams裡面，讓大家更方便"), "集成在Teams里面，让大家更方便");
    }

    #[test]
    fn simplified_becomes_traditional_on_request() {
        assert_eq!(normalize(ChineseScript::Traditional, SIMPLIFIED), QWEN3_TRADITIONAL);
        assert_eq!(normalized(ChineseScript::Traditional, "我想创建一个 good idea"), "我想創建一個 good idea");
    }

    #[test]
    fn as_is_and_non_chinese_text_are_left_alone() {
        assert!(matches!(normalize(ChineseScript::AsIs, QWEN3_TRADITIONAL), Cow::Borrowed(t) if t == QWEN3_TRADITIONAL));
        for script in [ChineseScript::Simplified, ChineseScript::Traditional, ChineseScript::AsIs] {
            for text in ["", "Ship the fix on Friday.", "こんにちは", "안녕하세요", "123 + 456"] {
                assert!(matches!(normalize(script, text), Cow::Borrowed(t) if t == text), "{script:?}: {text}");
            }
        }
        assert!(is_han('開') && is_han('开') && is_han('𠀀') && !is_han('あ') && !is_han('a') && !is_han('，'));
    }
}
