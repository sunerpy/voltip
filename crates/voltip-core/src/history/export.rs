//! Exporting a long entry (docs/dictation.md §22): subtitles (SRT) from its segments, and its text
//! (TXT). A subtitle line holds at most 20 CJK characters or 42 Latin ones (mixed lines by width);
//! a segment longer than a line becomes several cues, its time shared out in proportion to their
//! width.

use serde::{Deserialize, Serialize};

use super::HistoryEntry;
use crate::dictation::Segment;

/// What an export writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    /// Subtitles from the segments.
    Srt,
    /// The text (the processed one when there is one).
    Txt,
}

impl ExportFormat {
    /// The file extension.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Srt => "srt",
            Self::Txt => "txt",
        }
    }
}

/// Which text subtitles carry (docs/dictation.md §31).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SegmentText {
    /// What was recognised.
    Raw,
    /// Each segment's polished text where it has one (an imported file polished segment by
    /// segment), else what was recognised.
    #[default]
    Refined,
}

/// The file `entry` exports to in `format`, its subtitles carrying `text`; `None` for subtitles of
/// an entry without segments.
pub fn render(entry: &HistoryEntry, format: ExportFormat, text: SegmentText) -> Option<String> {
    match format {
        ExportFormat::Srt => entry.segments.as_deref().filter(|s| s.iter().any(|s| !s.text.trim().is_empty())).map(|s| srt(s, text)),
        ExportFormat::Txt => Some(txt(entry)),
    }
}

/// The name the save dialog offers: `name` without what a file system refuses (path separators,
/// `<>:"|?*`, control characters, leading dots and trailing dots or spaces), at most 120
/// characters, with the format's extension; `voltip` when nothing is left.
pub fn file_name(name: &str, format: ExportFormat) -> String {
    let cleaned: String = name.chars().map(|c| if c.is_control() || "/\\<>:\"|?*".contains(c) { ' ' } else { c }).collect();
    let stem: String = cleaned.trim().trim_start_matches('.').trim_start().chars().take(120).collect();
    let stem = stem.trim_end_matches(['.', ' ']);
    let stem = if stem.is_empty() { "voltip" } else { stem };
    format!("{stem}.{}", format.extension())
}

/// The width of a line: 20 CJK characters (21 units each) or 42 others (10 units each).
const LINE_WIDTH: u32 = 420;
/// A cut after one of these is a good line end.
const LINE_BREAKS: &[char] = &['，', '。', '！', '？', '、', '；', '：', ',', '.', '!', '?', ';', ':'];

/// A character drawn twice as wide as a Latin one: CJK ideographs and symbols, kana, Hangul, the
/// full-width forms.
fn wide(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F | 0x2E80..=0x303F | 0x3040..=0x33FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF
        | 0xA960..=0xA97F | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6)
}

fn units(c: char) -> u32 {
    if wide(c) { 21 } else { 10 }
}

/// `text` as subtitle lines no wider than [`LINE_WIDTH`]: each breaks after the last punctuation
/// that fits in its second half, else at the last space, else where it is full.
pub fn lines(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.trim().chars().collect();
    let mut lines = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let mut width = 0;
        let mut end = start;
        while end < chars.len() && width + units(chars[end]) <= LINE_WIDTH {
            width += units(chars[end]);
            end += 1;
        }
        if end < chars.len() {
            let half = start + (end - start) / 2;
            let at_break = (half..end).rev().find(|&i| LINE_BREAKS.contains(&chars[i])).map(|i| i + 1);
            let at_space = (start + 1..end).rev().find(|&i| chars[i] == ' ');
            end = at_break.or(at_space).unwrap_or(end).max(start + 1);
        }
        let line: String = chars[start..end].iter().collect::<String>().trim().to_owned();
        if !line.is_empty() {
            lines.push(line);
        }
        start = end;
    }
    lines
}

/// `hh:mm:ss,mmm`.
fn timestamp(ms: u64) -> String {
    format!("{:02}:{:02}:{:02},{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

/// Subtitles from `segments` carrying `text`: a cue per line, numbered from 1, CRLF line ends
/// (SubRip's own). A segment without text has no cue. The polished text of a segment fills that
/// segment's time as its recognised text would (docs/dictation.md §31): the polish rewrites within
/// a segment, so the times stay the segment's.
pub fn srt(segments: &[Segment], text: SegmentText) -> String {
    let mut out = String::new();
    let mut n = 0;
    for segment in segments {
        let shown = match text {
            SegmentText::Refined => segment.refined.as_deref().filter(|t| !t.trim().is_empty()).unwrap_or(&segment.text),
            SegmentText::Raw => &segment.text,
        };
        let lines = lines(shown);
        let total: u64 = lines.iter().map(|l| l.chars().map(|c| u64::from(units(c))).sum::<u64>()).sum();
        let span = segment.end_ms.saturating_sub(segment.start_ms);
        let mut before = 0;
        for line in lines {
            let width: u64 = line.chars().map(|c| u64::from(units(c))).sum();
            let start = segment.start_ms + span * before / total.max(1);
            before += width;
            let end = segment.start_ms + span * before / total.max(1);
            n += 1;
            out.push_str(&format!("{n}\r\n{} --> {}\r\n{line}\r\n\r\n", timestamp(start), timestamp(end.max(start))));
        }
    }
    out
}

/// The text a TXT export holds: the processed text when there is one, else the entry's text.
pub fn txt(entry: &HistoryEntry) -> String {
    let text = entry.processed.as_ref().map_or(entry.text.as_str(), |p| p.text.as_str());
    format!("{}\n", text.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(text: &str, start_ms: u64, end_ms: u64) -> Segment {
        Segment { text: text.into(), refined: None, start_ms, end_ms }
    }

    #[test]
    fn lines_hold_20_cjk_or_42_latin_characters_and_break_at_punctuation() {
        let cjk = "字".repeat(45);
        assert_eq!(lines(&cjk), ["字".repeat(20), "字".repeat(20), "字".repeat(5)]);
        let sentence = format!("{}，{}。", "甲".repeat(14), "乙".repeat(10));
        assert_eq!(lines(&sentence), ["甲".repeat(14) + "，", "乙".repeat(10) + "。"], "after the comma, not at 20");
        let latin = "The quick brown fox jumps over the lazy dog and keeps running far away";
        let got = lines(latin);
        assert!(got.iter().all(|l| l.chars().count() <= 42), "{got:?}");
        assert_eq!(got.join(" "), latin, "broken at spaces");
        assert_eq!(got[0], "The quick brown fox jumps over the lazy");
        // Mixed: 10 CJK + 21 Latin fill one line exactly.
        let mixed = format!("{}{}", "中".repeat(10), "a".repeat(21));
        assert_eq!(lines(&mixed), [mixed.as_str()]);
        assert!(lines("  ").is_empty());
        assert_eq!(lines("[未识别 00:00:29–00:00:59]"), ["[未识别 00:00:29–00:00:59]"]);
    }

    #[test]
    fn a_segment_becomes_cues_whose_time_follows_their_width() {
        let text = format!("{}，{}。", "甲".repeat(14), "乙".repeat(10));
        let out = srt(&[seg(&text, 1_000, 26_000), seg("", 26_000, 30_000), seg("Hello.", 3_600_000, 3_602_500)], SegmentText::Raw);
        // 15 and 11 characters: 25 s shared 15:11.
        let expected = format!(
            "1\r\n00:00:01,000 --> 00:00:15,423\r\n{}，\r\n\r\n2\r\n00:00:15,423 --> 00:00:26,000\r\n{}。\r\n\r\n3\r\n01:00:00,000 --> 01:00:02,500\r\nHello.\r\n\r\n",
            "甲".repeat(14),
            "乙".repeat(10)
        );
        assert_eq!(out, expected);
        assert_eq!(srt(&[], SegmentText::Raw), "");
        assert_eq!(timestamp(3_723_004), "01:02:03,004");
    }

    fn polished(text: &str, refined: &str, start_ms: u64, end_ms: u64) -> Segment {
        Segment { refined: Some(refined.into()), ..seg(text, start_ms, end_ms) }
    }

    /// The cues' times, in order.
    fn times(srt: &str) -> Vec<&str> {
        srt.lines().filter(|l| l.contains(" --> ")).collect()
    }

    /// docs/dictation.md §31: an imported file polished segment by segment exports its subtitles
    /// with either text in the same segment times, so either file lines up with the video.
    #[test]
    fn regression_srt_with_refined_text_keeps_the_segment_timings() {
        let segments = [
            polished("嗯那个今天我们讨论一下进度", "今天我们讨论一下进度。", 0, 4_000),
            polished("然后呢就是下周发布", "下周发布。", 4_000, 9_500),
            seg("这一段的润色失败了", 9_500, 12_000),
        ];
        let raw = srt(&segments, SegmentText::Raw);
        let refined = srt(&segments, SegmentText::Refined);
        assert_eq!(times(&raw), times(&refined), "{raw}\n{refined}");
        assert_eq!(times(&raw), ["00:00:00,000 --> 00:00:04,000", "00:00:04,000 --> 00:00:09,500", "00:00:09,500 --> 00:00:12,000"]);
        assert!(refined.contains("今天我们讨论一下进度。") && refined.contains("下周发布。"), "{refined}");
        assert!(!refined.contains("嗯那个") && raw.contains("嗯那个"), "{raw}\n{refined}");
        assert!(refined.contains("这一段的润色失败了"), "a segment without a polished text keeps what was recognised");

        // A segment whose text needs several lines shares its time among them, and still starts and
        // ends where the segment does with either text.
        let long = [polished(&"字".repeat(30), &"润".repeat(50), 1_000, 7_000)];
        let (raw, refined) = (srt(&long, SegmentText::Raw), srt(&long, SegmentText::Refined));
        for subtitles in [&raw, &refined] {
            let t = times(subtitles);
            assert!(t.first().unwrap().starts_with("00:00:01,000 -->") && t.last().unwrap().ends_with("--> 00:00:07,000"), "{subtitles}");
        }
        assert_eq!((times(&raw).len(), times(&refined).len()), (2, 3));
    }

    /// An entry from before §31 has no polished segments: its polished subtitles are the
    /// recognised ones.
    #[test]
    fn subtitles_of_an_entry_without_polished_segments_are_the_recognised_text() {
        let segments = [seg("第一段。", 0, 1_500), seg("第二段。", 1_500, 3_000)];
        assert_eq!(srt(&segments, SegmentText::Refined), srt(&segments, SegmentText::Raw));
        let mut entry = crate::history::tests::entry("原文");
        entry.segments = Some(segments.to_vec());
        assert_eq!(render(&entry, ExportFormat::Srt, SegmentText::Refined), render(&entry, ExportFormat::Srt, SegmentText::Raw));
        assert_eq!(serde_json::to_value(SegmentText::Refined).unwrap(), "refined");
        assert_eq!(SegmentText::default(), SegmentText::Refined);
        // The field is left out of the JSON when absent: entries and the wire stay as they were.
        assert_eq!(serde_json::to_value(&segments[0]).unwrap(), serde_json::json!({ "text": "第一段。", "start_ms": 0, "end_ms": 1_500 }));
        let back: Segment = serde_json::from_value(serde_json::json!({ "text": "x", "start_ms": 1, "end_ms": 2 })).unwrap();
        assert_eq!(back.refined, None);
    }

    #[test]
    fn a_file_name_is_safe_on_every_system() {
        assert_eq!(file_name("Voltip 2026-09-30 15.30", ExportFormat::Srt), "Voltip 2026-09-30 15.30.srt");
        assert_eq!(file_name("../a/b:c*?", ExportFormat::Txt), "a b c.txt");
        assert_eq!(file_name("  ..  ", ExportFormat::Txt), "voltip.txt");
        assert_eq!(file_name(&"字".repeat(200), ExportFormat::Srt).chars().count(), 124);
        assert_eq!(ExportFormat::Srt.extension(), "srt");
    }

    #[test]
    fn subtitles_need_segments() {
        let mut entry = crate::history::tests::entry("原文");
        assert_eq!(render(&entry, ExportFormat::Srt, SegmentText::Refined), None);
        assert_eq!(render(&entry, ExportFormat::Txt, SegmentText::Refined).as_deref(), Some("原文。\n"));
        entry.segments = Some(vec![seg("  ", 0, 10)]);
        assert_eq!(render(&entry, ExportFormat::Srt, SegmentText::Refined), None, "no segment has text");
        entry.segments = Some(vec![seg("第一段。", 0, 1_500)]);
        assert_eq!(render(&entry, ExportFormat::Srt, SegmentText::Refined).as_deref(), Some("1\r\n00:00:00,000 --> 00:00:01,500\r\n第一段。\r\n\r\n"));
    }

    #[test]
    fn the_text_export_prefers_the_processed_text() {
        let mut entry = crate::history::tests::entry("原文");
        assert_eq!(txt(&entry), "原文。\n");
        entry.processed =
            Some(Box::new(super::super::ProcessedText { text: "处理后\n\n".into(), preset: crate::presets::TakePreset::default().to_ref(), at_ms: 2 }));
        assert_eq!(txt(&entry), "处理后\n");
    }
}
