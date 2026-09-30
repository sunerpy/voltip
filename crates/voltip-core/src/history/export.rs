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

/// The file `entry` exports to in `format`; `None` for subtitles of an entry without segments.
pub fn render(entry: &HistoryEntry, format: ExportFormat) -> Option<String> {
    match format {
        ExportFormat::Srt => entry.segments.as_deref().filter(|s| s.iter().any(|s| !s.text.trim().is_empty())).map(srt),
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

/// Subtitles from `segments`: a cue per line, numbered from 1, CRLF line ends (SubRip's own). A
/// segment without text has no cue.
pub fn srt(segments: &[Segment]) -> String {
    let mut out = String::new();
    let mut n = 0;
    for segment in segments {
        let lines = lines(&segment.text);
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
        Segment { text: text.into(), start_ms, end_ms }
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
        let out = srt(&[seg(&text, 1_000, 26_000), seg("", 26_000, 30_000), seg("Hello.", 3_600_000, 3_602_500)]);
        // 15 and 11 characters: 25 s shared 15:11.
        let expected = format!(
            "1\r\n00:00:01,000 --> 00:00:15,423\r\n{}，\r\n\r\n2\r\n00:00:15,423 --> 00:00:26,000\r\n{}。\r\n\r\n3\r\n01:00:00,000 --> 01:00:02,500\r\nHello.\r\n\r\n",
            "甲".repeat(14),
            "乙".repeat(10)
        );
        assert_eq!(out, expected);
        assert_eq!(srt(&[]), "");
        assert_eq!(timestamp(3_723_004), "01:02:03,004");
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
        assert_eq!(render(&entry, ExportFormat::Srt), None);
        assert_eq!(render(&entry, ExportFormat::Txt).as_deref(), Some("原文。\n"));
        entry.segments = Some(vec![seg("  ", 0, 10)]);
        assert_eq!(render(&entry, ExportFormat::Srt), None, "no segment has text");
        entry.segments = Some(vec![seg("第一段。", 0, 1_500)]);
        assert_eq!(render(&entry, ExportFormat::Srt).as_deref(), Some("1\r\n00:00:00,000 --> 00:00:01,500\r\n第一段。\r\n\r\n"));
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
