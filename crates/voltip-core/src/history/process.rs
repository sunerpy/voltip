//! 用 AI 预设处理 (docs/dictation.md §22): a long entry's text through a preset, in parts. The text
//! is cut at sentence ends into parts of at most [`PART_MAX_CHARS`], each part is cleaned up in
//! order, and the results are joined; 要点纪要 then summarises its joined points once more when they
//! fit one part. The entry keeps its own text; the result is stored beside it.

use serde::{Deserialize, Serialize};

use super::ProcessedText;
use crate::dictation::{DictationError, RefineHints, Refiner, inject_separator};
use crate::presets::{BuiltinPreset, TakePreset};

/// Where one 用 AI 预设处理 request is (`CoreEvent::HistoryProcess`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ProcessState {
    /// `done` of `total` requests are back.
    Running {
        /// Requests answered.
        done: u32,
        /// Requests in all (the parts, and 要点纪要's summary).
        total: u32,
    },
    /// Stored with the entry.
    Done {
        /// What the entry now carries.
        processed: ProcessedText,
    },
    /// Nothing was stored.
    Failed {
        /// Plain text for the page.
        reason: String,
    },
    /// Stopped on request; nothing was stored.
    Cancelled,
}

/// Why nothing can be processed without an AI service.
pub const PROCESS_UNCONFIGURED: &str = "尚未配置 AI 润色服务，无法用预设处理";
/// Why an entry that was deleted meanwhile is not processed.
pub const PROCESS_ENTRY_GONE: &str = "这条记录已删除";

/// The longest part one clean-up request gets.
pub const PART_MAX_CHARS: usize = 1_500;

/// A character after which a sentence ends: CJK and Latin full stops, question and exclamation
/// marks, semicolons, ellipses and line breaks (a Latin full stop only before whitespace, so
/// `3.14` stays whole).
fn ends_sentence(chars: &[char], i: usize) -> bool {
    match chars[i] {
        '。' | '！' | '？' | '；' | '…' | '!' | '?' | ';' | '\n' => true,
        '.' => chars.get(i + 1).is_none_or(|c| c.is_whitespace()),
        _ => false,
    }
}

/// A character after which a long sentence may be cut.
fn pauses(c: char) -> bool {
    matches!(c, '，' | '、' | '：' | ',' | ':' | ' ')
}

/// `text` in parts of at most `max` characters: each cut after the last sentence end that fits
/// (and the spaces after it), else after the last pause, else at `max`. The parts put back
/// together are `text`.
pub fn parts(text: &str, max: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let max = max.max(1);
    let mut parts = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        if chars.len() - start <= max {
            parts.push(chars[start..].iter().collect());
            break;
        }
        let window = start..start + max;
        let mut cut = window
            .clone()
            .rev()
            .find(|&i| ends_sentence(&chars, i))
            .or_else(|| window.clone().rev().find(|&i| pauses(chars[i])))
            .map_or(start + max, |i| i + 1);
        while cut < window.end && chars[cut].is_whitespace() {
            cut += 1;
        }
        parts.push(chars[start..cut].iter().collect());
        start = cut;
    }
    parts
}

/// Whether the preset's points are summarised once more (要点纪要).
fn summarises(preset: &TakePreset) -> bool {
    matches!(preset, TakePreset::Builtin(BuiltinPreset::Notes))
}

/// The parts' results as one text: points one per line (要点纪要), prose as sentences join.
fn join(outputs: &[String], points: bool) -> String {
    if points {
        return outputs.iter().map(|o| o.trim()).filter(|o| !o.is_empty()).collect::<Vec<_>>().join("\n");
    }
    let mut text = String::new();
    for output in outputs.iter().map(|o| o.trim()).filter(|o| !o.is_empty()) {
        if !text.is_empty() {
            let separator = inject_separator(&text);
            text.push_str(separator);
        }
        text.push_str(output);
    }
    text
}

/// Run `text` through `refiner` with `hints`, a part at a time; `progress(done, total)` before the
/// first part and after each (要点纪要's summary counts as one more). A part that comes back empty
/// keeps its own text (a summary of points may be empty). The first failed request fails the whole.
pub async fn run(refiner: &dyn Refiner, hints: &RefineHints, text: &str, mut progress: impl FnMut(u32, u32) + Send) -> Result<String, DictationError> {
    let parts = parts(text.trim(), PART_MAX_CHARS);
    let points = summarises(&hints.preset);
    let summary = points && parts.len() > 1;
    let total = u32::try_from(parts.len() + usize::from(summary)).unwrap_or(u32::MAX);
    progress(0, total);
    let mut outputs = Vec::with_capacity(parts.len());
    for (i, part) in parts.iter().enumerate() {
        let out = refiner.refine(part, hints).await?;
        outputs.push(if out.text.trim().is_empty() && !points { part.clone() } else { out.text });
        progress(u32::try_from(i + 1).unwrap_or(u32::MAX), total);
    }
    let joined = join(&outputs, points);
    if !summary {
        return Ok(joined);
    }
    // The points of every part together, summarised once more when that fits one request.
    let summarised = if joined.chars().count() <= PART_MAX_CHARS {
        let out = refiner.refine(&joined, hints).await?;
        if out.text.trim().is_empty() { joined } else { out.text.trim().to_owned() }
    } else {
        joined
    };
    progress(total, total);
    Ok(summarised)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dictation::fakes::FakeRefiner;

    #[test]
    fn parts_are_cut_after_sentence_ends_and_lose_nothing() {
        let sentence = "今天的会议讨论了三件事。";
        let text = sentence.repeat(300);
        let cut = parts(&text, PART_MAX_CHARS);
        assert!(cut.iter().all(|p| p.chars().count() <= PART_MAX_CHARS));
        assert!(cut.iter().all(|p| p.ends_with('。')), "every part ends a sentence");
        assert_eq!(cut.concat(), text);
        assert_eq!(cut.len(), 3);
        // No sentence end in reach: after the last pause, else at the limit.
        let long = format!("{}，{}", "字".repeat(10), "字".repeat(20));
        assert_eq!(parts(&long, 15), ["字".repeat(10) + "，", "字".repeat(15), "字".repeat(5)]);
        assert_eq!(parts("Pi is 3.14 and e is 2.72. Done", 20), ["Pi is 3.14 and e is ", "2.72. Done"]);
        assert_eq!(parts("Hello. World.", 7), ["Hello. ", "World."]);
        assert_eq!(parts("", 10), Vec::<String>::new());
        assert_eq!(parts("短句。", 10), ["短句。"]);
    }

    #[test]
    fn results_join_as_sentences_or_as_points() {
        let outputs = ["第一段。".to_owned(), " Second part. ".to_owned(), String::new(), "第三段".to_owned()];
        assert_eq!(join(&outputs, false), "第一段。Second part. 第三段");
        assert_eq!(join(&["- 甲\n- 乙".to_owned(), "- 丙".to_owned()], true), "- 甲\n- 乙\n- 丙");
    }

    fn hints(preset: TakePreset) -> RefineHints {
        RefineHints { preset, ..RefineHints::default() }
    }

    /// Three parts through 校对: three requests in order, progress 0/3 … 3/3, the answers joined.
    #[tokio::test]
    async fn each_part_is_cleaned_up_in_order() {
        let refiner = FakeRefiner::ok("整理后的段落。");
        let text = "今天的会议讨论了三件事。".repeat(300);
        let mut seen = Vec::new();
        let out = run(&refiner, &hints(TakePreset::default()), &text, |done, total| seen.push((done, total))).await.unwrap();
        assert_eq!(out, "整理后的段落。".repeat(3));
        assert_eq!(seen, [(0, 3), (1, 3), (2, 3), (3, 3)]);
        let inputs = refiner.inputs();
        assert_eq!(inputs.len(), 3);
        assert_eq!(inputs.iter().map(|(t, _)| t.as_str()).collect::<String>(), text, "the parts are the whole text, in order");
        // An empty answer keeps the part.
        let empty = FakeRefiner::ok("  ");
        assert_eq!(run(&empty, &hints(TakePreset::default()), "短文。", |_, _| {}).await.unwrap(), "短文。");
    }

    /// 要点纪要: the points of every part, then one more request over them; one part needs none.
    #[tokio::test]
    async fn points_are_summarised_once_more_when_they_fit() {
        let notes = hints(TakePreset::Builtin(BuiltinPreset::Notes));
        let refiner = FakeRefiner::ok("- 要点");
        let text = "今天的会议讨论了三件事。".repeat(300);
        let mut seen = Vec::new();
        let out = run(&refiner, &notes, &text, |done, total| seen.push((done, total))).await.unwrap();
        assert_eq!(out, "- 要点");
        assert_eq!(seen.last(), Some(&(4, 4)));
        let inputs = refiner.inputs();
        assert_eq!(inputs.len(), 4);
        assert_eq!(inputs[3].0, "- 要点\n- 要点\n- 要点", "the summary gets the joined points");
        let one = FakeRefiner::ok("- 要点");
        run(&one, &notes, "一句话。", |_, _| {}).await.unwrap();
        assert_eq!(one.calls(), 1, "a single part is summarised already");
        // Points too long for one request stay as they are.
        let long_points = "- ".to_owned() + &"要".repeat(600);
        let wordy = FakeRefiner::ok(&long_points);
        let out = run(&wordy, &notes, &text, |_, _| {}).await.unwrap();
        assert_eq!(wordy.calls(), 3);
        assert_eq!(out, [long_points.as_str(); 3].join("\n"));
    }

    #[tokio::test]
    async fn a_failed_request_fails_the_whole() {
        let refiner = FakeRefiner::err("429 rate limited");
        let err = run(&refiner, &hints(TakePreset::default()), "一句话。", |_, _| {}).await.unwrap_err();
        assert!(err.to_string().contains("429"), "{err}");
    }
}
