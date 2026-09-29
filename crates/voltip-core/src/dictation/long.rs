//! Long takes (docs/dictation.md §22): a take that may run past two minutes is written to a
//! recording file as it is spoken and cut into segments of 20–30 s, each recognised while the take
//! goes on. This module holds what the engine drives: the recording thread, the fallback
//! segmenter, reading a segment back, the text of a finished take, and the recording directory.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::sync::mpsc;

use super::engine::Internal;
use super::inject_separator;
use super::ports::{PCM_SAMPLE_RATE_HZ, PcmStream, Segmenter};
use super::wav;

/// Samples per second of a recording file.
pub const RATE: u64 = PCM_SAMPLE_RATE_HZ as u64;
/// The fallback segmenter cuts once a segment is this long (30 s)…
pub const FALLBACK_CUT_AFTER: u64 = 30 * RATE;
/// …in the quietest block of its last 5 s…
pub const FALLBACK_SEARCH: u64 = 5 * RATE;
/// …a block being 200 ms.
pub const BLOCK: u64 = RATE / 5;
/// Directory of the recording files, under the data directory.
pub const RECORDINGS_DIR: &str = "recordings";
/// A long take's in-memory part (the whole-take cap): a take no longer than this is a short take.
pub const IN_MEMORY: u64 = 120 * RATE;
/// A long take's text is cleaned up by the AI only up to this many characters (docs/dictation.md
/// §22); a longer one is inserted as recognised, and 用 AI 预设处理 in the history takes it in parts.
pub const REFINE_MAX_CHARS: usize = 2_000;
/// A long take's text is pasted only up to this many characters; a longer one waits on the
/// clipboard.
pub const PASTE_MAX_CHARS: usize = 5_000;
/// Why a long take was not cleaned up (`Done.refine_error`, like any clean-up that did not run).
pub const REFINE_SKIPPED: &str = "全文超过 2000 字，本次未进行 AI 润色。可在历史记录中用 AI 预设分段处理。";
/// Why a long take's text was left on the clipboard.
pub const TOO_LONG_TO_PASTE: &str = "全文超过 5000 字，已复制到剪贴板，未直接粘贴。";
/// What the recording thread waits between reads when nothing arrived: the stream holds a minute,
/// so waiting loses nothing (a pause, not a wait for an event).
const WRITER_IDLE: Duration = Duration::from_millis(20);

/// The core's own cutter (docs/dictation.md §22), used when the shell has none or it cannot run
/// (its model is missing): once a segment reaches 30 s it is cut in the middle of the quietest
/// 200 ms of its last 5 s, so segments run 25–30 s and are cut where the speech pauses if it does.
#[derive(Debug, Default)]
pub struct EnergySegmenter {
    /// Where the current segment starts.
    start: u64,
    /// Samples pushed so far.
    pos: u64,
    /// The whole blocks of the last [`FALLBACK_SEARCH`]: where each starts and its energy.
    blocks: VecDeque<(u64, f64)>,
    /// The block being filled: its sum of squares and how many samples it has.
    sum: f64,
    filled: u64,
}

impl EnergySegmenter {
    /// A cutter for one take.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Segmenter for EnergySegmenter {
    fn push(&mut self, samples: &[f32]) -> Vec<u64> {
        let mut cuts = Vec::new();
        for &sample in samples {
            self.sum += f64::from(sample) * f64::from(sample);
            self.filled += 1;
            self.pos += 1;
            if self.filled < BLOCK {
                continue;
            }
            self.blocks.push_back((self.pos - BLOCK, self.sum));
            (self.sum, self.filled) = (0.0, 0);
            while self.blocks.front().is_some_and(|&(at, _)| at + FALLBACK_SEARCH < self.pos) {
                self.blocks.pop_front();
            }
            if self.pos - self.start < FALLBACK_CUT_AFTER {
                continue;
            }
            let quietest = self.blocks.iter().filter(|&&(at, _)| at >= self.start).min_by(|a, b| a.1.total_cmp(&b.1));
            let cut = quietest.map_or(self.pos, |&(at, _)| at + BLOCK / 2);
            cuts.push(cut);
            self.start = cut;
            self.blocks.retain(|&(at, _)| at >= cut);
        }
        cuts
    }

    fn finish(&mut self) -> Option<u64> {
        (self.pos > self.start).then_some(self.pos)
    }
}

/// What the recording thread needs.
pub struct WriterJob {
    /// The take.
    pub session: u64,
    /// The capture's stream.
    pub stream: Box<dyn PcmStream>,
    /// The take's recording file, open for writing, and where it is.
    pub file: File,
    /// The file's path.
    pub path: PathBuf,
    /// Cuts the take into segments.
    pub segmenter: Box<dyn Segmenter>,
    /// Back to the engine.
    pub tx: mpsc::Sender<Internal>,
}

/// The recording thread (docs/dictation.md §22), on a blocking thread: read the take's stream,
/// write it to the recording file as 16-bit samples, fill a gap with silence and note it, hand every
/// chunk to the segmenter and report each segment it cuts ([`Internal::LongSegment`]). When the
/// stream closes: the last segment, then [`Internal::LongClosed`] with the length, the gaps and a
/// write error if there was one (the file is incomplete from then on).
pub fn run_writer(job: WriterJob) {
    let WriterJob { session, mut stream, file, path, segmenter, tx } = job;
    let mut out = TakeFile { session, file, path, segmenter, tx, bytes: Vec::new(), total: 0, error: None };
    let mut samples = vec![0.0f32; 4096];
    let mut gaps = Vec::new();
    loop {
        if let Some(missing) = stream.gap() {
            // The shell's buffer overflowed: silence keeps the timeline, and the span is noted.
            // The whole gap goes in now: the stream reports it once.
            gaps.push((out.total, out.total + missing));
            samples.fill(0.0);
            let mut left = missing;
            while left > 0 {
                let n = usize::try_from(left.min(samples.len() as u64)).unwrap_or(samples.len());
                out.take(&samples[..n]);
                left -= n as u64;
            }
            continue;
        }
        let n = stream.read(&mut samples);
        if n == 0 {
            if stream.is_closed() {
                break;
            }
            std::thread::sleep(WRITER_IDLE);
            continue;
        }
        out.take(&samples[..n]);
    }
    out.finish(merge_spans(gaps));
}

/// The recording thread's side of a take: its file, its segmenter, how far it got.
struct TakeFile {
    session: u64,
    file: File,
    path: PathBuf,
    segmenter: Box<dyn Segmenter>,
    tx: mpsc::Sender<Internal>,
    bytes: Vec<u8>,
    total: u64,
    error: Option<String>,
}

impl TakeFile {
    /// Write `samples`, cut, and report every segment they end.
    fn take(&mut self, samples: &[f32]) {
        if self.error.is_none()
            && let Err(e) = write_samples(&mut self.file, samples, &mut self.bytes)
        {
            tracing::warn!(session = self.session, error = %e, "the recording file could not be written");
            self.error = Some(e.to_string());
        }
        self.total += samples.len() as u64;
        for end in self.segmenter.push(samples) {
            let _ = self.tx.blocking_send(Internal::LongSegment { session: self.session, end });
        }
    }

    /// The last segment, then the take's length, gaps and write error.
    fn finish(mut self, gaps: Vec<(u64, u64)>) {
        if let Some(end) = self.segmenter.finish() {
            let _ = self.tx.blocking_send(Internal::LongSegment { session: self.session, end });
        }
        tracing::info!(session = self.session, seconds = self.total / RATE, gaps = gaps.len(), "long take recorded");
        let _ = self.tx.blocking_send(Internal::LongClosed { session: self.session, path: self.path, total: self.total, gaps, error: self.error });
    }
}

/// `samples` as 16-bit little-endian PCM, appended to `file` in one write.
fn write_samples(file: &mut File, samples: &[f32], bytes: &mut Vec<u8>) -> std::io::Result<()> {
    bytes.clear();
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32_767.0).round() as i16;
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    file.write_all(bytes)
}

/// Samples `start..end` of a recording file as a 16-bit mono WAV at 16 kHz.
pub fn read_segment(path: &Path, start: u64, end: u64) -> std::io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(start * 2))?;
    let len = usize::try_from(end.saturating_sub(start) * 2).map_err(std::io::Error::other)?;
    let mut raw = vec![0u8; len];
    file.read_exact(&mut raw)?;
    let pcm: Vec<i16> = raw.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();
    Ok(wav::encode_pcm16(&pcm, PCM_SAMPLE_RATE_HZ))
}

/// Adjacent or overlapping spans as one.
fn merge_spans(mut spans: Vec<(u64, u64)>) -> Vec<(u64, u64)> {
    spans.sort_unstable();
    let mut out: Vec<(u64, u64)> = Vec::with_capacity(spans.len());
    for (start, end) in spans {
        match out.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => out.push((start, end)),
        }
    }
    out
}

/// `hh:mm:ss` of a sample position.
pub fn clock(sample: u64) -> String {
    let seconds = sample / RATE;
    format!("{:02}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60)
}

/// The placeholder for a span nothing was recognised in (docs/dictation.md §22).
pub fn unrecognised(start: u64, end: u64) -> String {
    format!("[未识别 {}–{}]", clock(start), clock(end))
}

/// The text of a finished long take: each segment's text in order (`None`: it failed twice),
/// joined as sentences are (no space after a CJK character), and every span nothing was recognised
/// in — a failed segment, samples that went missing — as 「[未识别 hh:mm:ss–hh:mm:ss]」, adjacent
/// ones as one.
pub fn assemble(segments: &[(u64, u64, Option<String>)], gaps: &[(u64, u64)]) -> String {
    let failed = segments.iter().filter(|s| s.2.is_none()).map(|s| (s.0, s.1));
    let unrecognised_spans = merge_spans(failed.chain(gaps.iter().copied()).collect());
    let mut pieces: Vec<(u64, String)> = segments.iter().filter_map(|(start, _, text)| text.as_ref().map(|t| (*start, t.trim().to_owned()))).collect();
    pieces.extend(unrecognised_spans.into_iter().map(|(start, end)| (start, unrecognised(start, end))));
    pieces.sort_by_key(|p| p.0);
    let mut text = String::new();
    for (_, piece) in pieces {
        if piece.is_empty() {
            continue;
        }
        if !text.is_empty() {
            let separator = inject_separator(&text);
            text.push_str(separator);
        }
        text.push_str(&piece);
    }
    text
}

/// `<data_dir>/recordings`.
pub fn recordings_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(RECORDINGS_DIR)
}

/// Create a take's recording file in `dir` (readable and writable by this user only on Unix).
pub fn create_file(dir: &Path, session: u64, now_ms: u64) -> std::io::Result<(File, PathBuf)> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("take-{now_ms}-{session}.pcm"));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok((options.open(&path)?, path))
}

/// Remove the recording files an earlier run left behind (it ended while recording); returns how
/// many. A missing directory is nothing to clear.
pub fn clear_leftovers(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "pcm") && std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Delete a take's recording file; a file that is already gone is not an error.
pub fn remove(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => tracing::debug!(path = %path.display(), "recording file removed"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!(path = %path.display(), error = %e, "recording file could not be removed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A signal of `seconds`, loud except a 300 ms pause at the end of every 30 s.
    fn paced(seconds: u64) -> Vec<f32> {
        (0..seconds * RATE)
            .map(|i| {
                let into = i % (30 * RATE);
                if into >= 30 * RATE - 3 * BLOCK / 2 { 0.0 } else { 0.3 }
            })
            .collect()
    }

    #[test]
    fn the_fallback_cuts_in_the_pause_of_every_30_seconds() {
        let mut segmenter = EnergySegmenter::new();
        let signal = paced(95);
        let mut cuts = Vec::new();
        for chunk in signal.chunks(1234) {
            cuts.extend(segmenter.push(chunk));
        }
        // In the middle of the quietest block: the pause at 29.7–30.0 s.
        assert_eq!(cuts.len(), 3, "{cuts:?}");
        for (i, cut) in cuts.iter().enumerate() {
            let expected = (i as u64 + 1) * 30 * RATE - BLOCK / 2;
            assert_eq!(*cut, expected, "cut {i}");
        }
        assert_eq!(segmenter.finish(), Some(95 * RATE), "the rest ends the take");
        assert_eq!(segmenter.finish(), Some(95 * RATE));
    }

    /// Without any pause the cut still comes within the last 5 s of 30.
    #[test]
    fn a_constant_signal_is_cut_between_25_and_30_seconds() {
        let mut segmenter = EnergySegmenter::new();
        let cuts = segmenter.push(&vec![0.2f32; (61 * RATE) as usize]);
        assert_eq!(cuts.len(), 2);
        assert!((25 * RATE..=30 * RATE).contains(&cuts[0]), "{}", cuts[0] as f64 / RATE as f64);
        assert!((cuts[0] + 25 * RATE..=cuts[0] + 30 * RATE).contains(&cuts[1]));
        let mut empty = EnergySegmenter::new();
        assert_eq!(empty.finish(), None, "nothing recorded, no segment");
    }

    /// Two hours of the paced signal: a cut in every 30 s, 240 of them (docs/dictation.md §22),
    /// and the last 100 ms after the last pause close the take.
    #[test]
    fn two_hours_make_240_segments() {
        let mut segmenter = EnergySegmenter::new();
        let mut cuts = 0;
        let period = paced(30);
        for _ in 0..240 {
            cuts += segmenter.push(&period).len();
        }
        assert_eq!(cuts, 240);
        assert_eq!(segmenter.finish(), Some(7200 * RATE));
    }

    #[test]
    fn clocks_and_placeholders() {
        assert_eq!(clock(0), "00:00:00");
        assert_eq!(clock(RATE * 3723), "01:02:03");
        assert_eq!(unrecognised(RATE * 60, RATE * 90), "[未识别 00:01:00–00:01:30]");
    }

    /// Segments join as sentences do; a failed segment and a gap next to it become one placeholder;
    /// a gap inside a recognised segment follows its text.
    #[test]
    fn the_text_joins_segments_and_marks_what_was_not_recognised() {
        let s = |a: u64, b: u64, t: Option<&str>| (a * RATE, b * RATE, t.map(str::to_owned));
        let segments = [s(0, 30, Some("第一段。")), s(30, 60, None), s(60, 90, Some("Second part.")), s(90, 120, Some("第四段"))];
        let gaps = [(60 * RATE, 62 * RATE), (100 * RATE, 101 * RATE)];
        assert_eq!(assemble(&segments, &gaps), "第一段。[未识别 00:00:30–00:01:02] Second part. 第四段[未识别 00:01:40–00:01:41]");
        assert_eq!(assemble(&[s(0, 5, Some("  "))], &[]), "", "an empty segment adds nothing");
        assert_eq!(merge_spans(vec![(5, 9), (1, 3), (3, 4), (8, 12)]), vec![(1, 4), (5, 12)]);
    }

    /// The file keeps 16-bit samples; a segment reads back as a WAV of exactly its samples.
    #[test]
    fn segments_read_back_from_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let (mut file, path) = create_file(dir.path(), 7, 1).unwrap();
        let samples: Vec<f32> = (0..1000).map(|i| i as f32 / 1000.0).collect();
        write_samples(&mut file, &samples, &mut Vec::new()).unwrap();
        let wav_bytes = read_segment(&path, 100, 200).unwrap();
        let pcm = wav::pcm_data(&wav_bytes).unwrap();
        assert_eq!(pcm.len(), 200);
        assert_eq!(i16::from_le_bytes([pcm[0], pcm[1]]), (0.1f32 * 32_767.0).round() as i16);
        assert!(read_segment(&path, 900, 1100).is_err(), "past the end");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert!(create_file(dir.path(), 7, 1).is_err(), "never overwrites");
        std::fs::write(dir.path().join("keep.txt"), "x").unwrap();
        assert_eq!(clear_leftovers(dir.path()), 1);
        assert!(dir.path().join("keep.txt").exists(), "only recordings go");
        assert_eq!(clear_leftovers(&dir.path().join("none")), 0);
        remove(&path);
    }
}
