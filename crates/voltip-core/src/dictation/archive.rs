//! The kept copy of a long take (docs/dictation.md §26.2): its audio in parts of 30 s, each an Ogg
//! Opus file written whole while the take is recorded, so a crash, a power cut or a damaged file
//! costs at most the part it hit.
//!
//! - **Parts** `part-00000.opus` …: Ogg Opus (RFC 7845) at 16 kHz, VoIP, 20 ms frames on the upload
//!   codec's 24 kbit/s budget (`voltip-asr` `ogg_opus.rs`), about 10 MB an hour. Each part has its
//!   own encoder, so it decodes on its own; every Ogg page carries a CRC-32. A part is written to
//!   `part-….opus.tmp`, flushed to the disk, renamed, and its directory flushed (Unix): a part on
//!   disk is complete.
//! - **Manifest** `manifest.json`, written the same way when the take ends: its length, the part
//!   length, the segment cuts its recording thread made and the spans that went missing. A take
//!   whose app stopped before that has none: [`recover`] puts it together from its parts and the
//!   take's raw recording file.
//!
//! The take's raw recording file (`long.rs`, 16-bit PCM, appended as it is spoken) stays the source
//! of the segments recognised while it records; the parts are what remains of it afterwards.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use opus_rs::{Application, OpusDecoder, OpusEncoder};
use serde::{Deserialize, Serialize};

use super::long::RATE;
use super::ports::PCM_SAMPLE_RATE_HZ;
use super::wav;
use crate::ogg::{self, FIRST_PAGE, LAST_PAGE, OggWriter, PRE_SKIP};

/// A part's length: 30 s.
pub const PART_SECONDS: u64 = 30;
/// A part's length in samples.
pub const PART_SAMPLES: u64 = PART_SECONDS * RATE;
/// The manifest's name, in the parts' directory.
pub const MANIFEST: &str = "manifest.json";
/// The suffix of a take's parts directory, next to its raw recording file.
pub const PARTS_SUFFIX: &str = "parts";
/// Schema version of the manifest.
const MANIFEST_SCHEMA: u16 = 1;
/// A 20 ms frame at 16 kHz.
const FRAME: usize = 320;
/// The space a frame's packet is given: 24 kbit/s and the TOC byte. In VBR `opus-rs` (0.1.34)
/// takes the SILK layer's bitrate from this space, not from `bitrate_bps` (see `voltip-asr`
/// `ogg_opus.rs`); measured 21.5 kbit/s on speech-like audio, 57 times faster than real time.
const PACKET_BYTES: usize = 24_000 / 8 / 50 + 1;
/// One lacing value a packet.
const _: () = assert!(PACKET_BYTES < 255);
/// Packets on one page: a second of frames.
const PACKETS_PER_PAGE: usize = 50;
/// 48 kHz granule units per 16 kHz sample.
const SCALE: u64 = (ogg::GRANULE_RATE / PCM_SAMPLE_RATE_HZ) as u64;
/// The encoder's lookahead at 16 kHz: the samples a decoder drops at the start of a part.
const LOOKAHEAD: usize = PRE_SKIP as usize / SCALE as usize;

/// What a finished archive holds (`manifest.json`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Schema version.
    pub schema: u16,
    /// The take's length in samples.
    pub total: u64,
    /// A part's length in samples (the last one may be shorter).
    pub part_samples: u64,
    /// How many parts there are, `part-00000.opus` on.
    pub parts: u32,
    /// Where the take's segments end, as its recording thread cut them; empty when unknown (a
    /// recovered take): they are cut again when it is recognised.
    #[serde(default)]
    pub cuts: Vec<u64>,
    /// `[start, end)` spans with no audio: samples the capture lost, a part that is missing.
    #[serde(default)]
    pub gaps: Vec<(u64, u64)>,
}

/// What [`ArchiveWriter::finish`] (or [`recover`]) left in a parts directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveReport {
    /// The directory.
    pub dir: PathBuf,
    /// Its manifest.
    pub manifest: Manifest,
    /// The parts' size in all.
    pub bytes: u64,
    /// Why parts could not be written, if they could not: the archive is incomplete from there on.
    pub error: Option<String>,
}

/// `take-….pcm` → `take-….parts`.
pub fn parts_dir(pcm: &Path) -> PathBuf {
    pcm.with_extension(PARTS_SUFFIX)
}

/// `<dir>/part-00042.opus`.
pub fn part_path(dir: &Path, index: u32) -> PathBuf {
    dir.join(format!("part-{index:05}.opus"))
}

/// Writes a take's parts as it is recorded (on its recording thread): samples in, a part on disk
/// every 30 s.
pub struct ArchiveWriter {
    dir: PathBuf,
    part: Option<PartEncoder>,
    /// Parts written.
    written: u32,
    /// Samples taken.
    total: u64,
    bytes: u64,
    error: Option<String>,
}

impl ArchiveWriter {
    /// A new, empty parts directory `dir` (this user's only, on Unix).
    pub fn create(dir: &Path) -> std::io::Result<Self> {
        create_private_dir(dir)?;
        Ok(Self { dir: dir.to_path_buf(), part: None, written: 0, total: 0, bytes: 0, error: None })
    }

    /// Append `samples`; a part that fills is written. After a failed write nothing more is
    /// written (the error is reported at the end); the samples are still counted.
    pub fn push(&mut self, samples: &[i16]) {
        let mut rest = samples;
        while !rest.is_empty() {
            let in_part = self.total % PART_SAMPLES;
            let room = usize::try_from(PART_SAMPLES - in_part).unwrap_or(usize::MAX);
            let (now, later) = rest.split_at(rest.len().min(room));
            if self.error.is_none() {
                match self.part.as_mut() {
                    Some(part) => part.push(now),
                    None => match PartEncoder::new() {
                        Ok(mut part) => {
                            part.push(now);
                            self.part = Some(part);
                        }
                        Err(e) => self.error = Some(e),
                    },
                }
            }
            self.total += now.len() as u64;
            rest = later;
            if self.total.is_multiple_of(PART_SAMPLES) {
                self.close_part();
            }
        }
    }

    /// Write the part being filled, if any.
    fn close_part(&mut self) {
        let Some(part) = self.part.take() else { return };
        if self.error.is_some() {
            return;
        }
        let bytes = part.finish();
        match write_durable(&part_path(&self.dir, self.written), &bytes) {
            Ok(()) => {
                self.written += 1;
                self.bytes += bytes.len() as u64;
            }
            Err(e) => {
                tracing::warn!(dir = %self.dir.display(), part = self.written, error = %e, "a part of the long take's kept copy could not be written");
                self.error = Some(e.to_string());
            }
        }
    }

    /// The take ended: its last part, then the manifest with `cuts` and `gaps`.
    pub fn finish(mut self, cuts: Vec<u64>, gaps: Vec<(u64, u64)>) -> ArchiveReport {
        self.close_part();
        let covered = u64::from(self.written) * PART_SAMPLES;
        let mut gaps = gaps;
        if covered < self.total {
            // Parts that could not be written: nothing of them remains.
            gaps.push((covered, self.total));
        }
        let manifest = Manifest { schema: MANIFEST_SCHEMA, total: self.total, part_samples: PART_SAMPLES, parts: self.written, cuts, gaps: merge(gaps) };
        // Written after a failed part too: the copy then says what it holds and where it stops.
        if let Err(e) = write_manifest(&self.dir, &manifest) {
            tracing::warn!(dir = %self.dir.display(), error = %e, "the long take's kept copy has no manifest; the next start puts it together");
            self.error.get_or_insert_with(|| e.to_string());
        }
        ArchiveReport { dir: self.dir, manifest, bytes: self.bytes, error: self.error }
    }
}

/// One part being encoded: its Ogg pages so far.
struct PartEncoder {
    encoder: OpusEncoder,
    ogg: OggWriter,
    /// Packets not yet on a page; a full page is written when the next packet comes, so the last
    /// page always has packets.
    page: Vec<Vec<u8>>,
    /// Frames already on pages.
    paged: u64,
    /// Samples of a frame not yet complete.
    pending: Vec<i16>,
    /// Samples taken.
    samples: u64,
}

impl PartEncoder {
    fn new() -> Result<Self, String> {
        let mut encoder = OpusEncoder::new(PCM_SAMPLE_RATE_HZ as i32, 1, Application::Voip).map_err(str::to_owned)?;
        encoder.bitrate_bps = 24_000;
        encoder.use_cbr = false;
        let mut ogg = OggWriter::default();
        ogg.page(&[ogg::opus_head(PCM_SAMPLE_RATE_HZ)], 0, FIRST_PAGE);
        ogg.page(&[ogg::opus_tags()], 0, 0);
        Ok(Self { encoder, ogg, page: Vec::with_capacity(PACKETS_PER_PAGE), paged: 0, pending: Vec::with_capacity(FRAME), samples: 0 })
    }

    fn push(&mut self, samples: &[i16]) {
        self.samples += samples.len() as u64;
        self.extend(samples.iter().copied());
    }

    /// Frame `samples` and encode every frame they complete.
    fn extend(&mut self, samples: impl Iterator<Item = i16>) {
        for sample in samples {
            self.pending.push(sample);
            if self.pending.len() == FRAME {
                self.encode_pending();
            }
        }
    }

    /// Encode the complete frame in `pending`. A frame the encoder refuses becomes an empty packet,
    /// which reads back as 20 ms of silence: the part keeps its timeline.
    fn encode_pending(&mut self) {
        if self.page.len() == PACKETS_PER_PAGE {
            self.paged += PACKETS_PER_PAGE as u64;
            // A page's granule position counts the samples its packets and the earlier ones
            // decode to, the pre-skip included (RFC 7845 §4).
            self.ogg.page(&std::mem::take(&mut self.page), self.paged * FRAME as u64 * SCALE, 0);
        }
        let mut packet = [0u8; PACKET_BYTES];
        let len = self.encoder.encode_i16(&self.pending, FRAME, &mut packet).unwrap_or(0);
        self.pending.clear();
        self.page.push(packet[..len].to_vec());
    }

    /// The part's bytes. The encoder holds its lookahead back, so frames of silence after the last
    /// sample carry it out; the last page's granule position marks where the part ends, and a
    /// decoder trims what follows.
    fn finish(mut self) -> Vec<u8> {
        self.extend(std::iter::repeat_n(0, LOOKAHEAD));
        if !self.pending.is_empty() {
            let pad = FRAME - self.pending.len();
            self.extend(std::iter::repeat_n(0, pad));
        }
        let end = u64::from(PRE_SKIP) + self.samples * SCALE;
        self.ogg.page(&std::mem::take(&mut self.page), end, LAST_PAGE);
        self.ogg.bytes
    }
}

/// Why a part could not be read back.
#[derive(Debug)]
pub enum PartError {
    /// The file is not there or could not be read.
    Io(std::io::Error),
    /// Its pages are damaged or not this layout.
    Damaged(String),
}

impl std::fmt::Display for PartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "a part of the kept recording could not be read: {e}"),
            Self::Damaged(why) => write!(f, "a part of the kept recording is damaged: {why}"),
        }
    }
}

/// The samples of the part at `path`.
pub fn read_part(path: &Path) -> Result<Vec<i16>, PartError> {
    let bytes = std::fs::read(path).map_err(PartError::Io)?;
    decode_part(&bytes)
}

/// The samples of a part's bytes: every page checked, every packet decoded, the encoder's lookahead
/// dropped at the start and the padding at the end.
fn decode_part(bytes: &[u8]) -> Result<Vec<i16>, PartError> {
    let pages = ogg::read_pages(bytes).map_err(|e| PartError::Damaged(e.to_string()))?;
    let (Some(head), Some(last)) = (pages.first(), pages.last()) else { return Err(PartError::Damaged("no pages".into())) };
    if !head.packets.first().is_some_and(|p| p.starts_with(b"OpusHead")) || last.flags & LAST_PAGE == 0 {
        return Err(PartError::Damaged("not a complete Opus part".into()));
    }
    let samples = usize::try_from(last.granule.saturating_sub(u64::from(PRE_SKIP)) / SCALE).map_err(|e| PartError::Damaged(e.to_string()))?;
    let mut decoder = OpusDecoder::new(PCM_SAMPLE_RATE_HZ as i32, 1).map_err(|e| PartError::Damaged(e.to_owned()))?;
    let mut out = Vec::with_capacity(samples + LOOKAHEAD + FRAME);
    let mut frame = vec![0f32; FRAME];
    for packet in pages.iter().skip(2).flat_map(|p| p.packets.iter()) {
        let n = if packet.is_empty() { 0 } else { decoder.decode(packet, FRAME, &mut frame).unwrap_or(0) };
        let n = if n == 0 {
            // A packet that did not decode: its 20 ms as silence, so the timeline holds.
            frame.fill(0.0);
            FRAME
        } else {
            n
        };
        out.extend(frame[..n].iter().map(|&s| (s.clamp(-1.0, 1.0) * 32_767.0).round() as i16));
    }
    let start = LOOKAHEAD.min(out.len());
    let mut audio = out.split_off(start);
    audio.resize(samples, 0);
    Ok(audio)
}

/// Check every page of the part at `path` without decoding it; its length in samples.
pub fn check_part(path: &Path) -> Result<u64, PartError> {
    let bytes = std::fs::read(path).map_err(PartError::Io)?;
    let pages = ogg::read_pages(&bytes).map_err(|e| PartError::Damaged(e.to_string()))?;
    match (pages.first(), pages.last()) {
        (Some(head), Some(last)) if head.packets.first().is_some_and(|p| p.starts_with(b"OpusHead")) && last.flags & LAST_PAGE != 0 => {
            Ok(last.granule.saturating_sub(u64::from(PRE_SKIP)) / SCALE)
        }
        _ => Err(PartError::Damaged("not a complete Opus part".into())),
    }
}

/// A kept long recording read back (a recognition again, docs/dictation.md §26.4).
#[derive(Clone, Debug)]
pub struct KeptLong {
    /// Its directory.
    pub dir: PathBuf,
    /// Its manifest.
    pub manifest: Manifest,
}

impl KeptLong {
    /// The recording in `dir`, by its manifest.
    pub fn open(dir: &Path) -> std::io::Result<Self> {
        let manifest = read_manifest(dir)?;
        Ok(Self { dir: dir.to_path_buf(), manifest })
    }

    /// The samples of part `index`; a part that cannot be read is silence of its length, and
    /// `damaged` says so.
    fn part_samples(&self, index: u32) -> (Vec<i16>, bool) {
        let length = self.part_length(index);
        match read_part(&part_path(&self.dir, index)) {
            Ok(mut samples) => {
                samples.resize(length, 0);
                (samples, false)
            }
            Err(e) => {
                tracing::warn!(dir = %self.dir.display(), part = index, error = %e, "a part of a kept recording reads as silence");
                (vec![0; length], true)
            }
        }
    }

    /// How many samples part `index` covers.
    fn part_length(&self, index: u32) -> usize {
        let start = u64::from(index) * self.manifest.part_samples;
        usize::try_from(self.manifest.total.saturating_sub(start).min(self.manifest.part_samples)).unwrap_or(0)
    }

    /// Samples `start..end` as a 16-bit mono WAV at 16 kHz (a segment for the recogniser).
    pub fn read_segment(&self, start: u64, end: u64) -> Vec<u8> {
        let end = end.min(self.manifest.total);
        let mut pcm = Vec::with_capacity(usize::try_from(end.saturating_sub(start)).unwrap_or(0));
        if end > start && self.manifest.part_samples > 0 {
            let first = start / self.manifest.part_samples;
            let last = (end - 1) / self.manifest.part_samples;
            for index in first..=last {
                let index = u32::try_from(index).unwrap_or(u32::MAX);
                let (samples, _) = self.part_samples(index);
                let part_start = u64::from(index) * self.manifest.part_samples;
                let from = usize::try_from(start.saturating_sub(part_start)).unwrap_or(0).min(samples.len());
                let to = usize::try_from(end - part_start).unwrap_or(usize::MAX).min(samples.len());
                pcm.extend_from_slice(&samples[from..to.max(from)]);
            }
        }
        wav::encode_pcm16(&pcm, PCM_SAMPLE_RATE_HZ)
    }

    /// Every sample, part by part, in order (to cut the recording again); `stop` ends the walk.
    /// The spans of parts that could not be read come back too: they read as silence.
    pub fn for_each_part(&self, stop: &AtomicBool, mut each: impl FnMut(&[i16])) -> Vec<(u64, u64)> {
        let mut damaged = Vec::new();
        for index in 0..self.manifest.parts {
            if stop.load(Ordering::SeqCst) {
                break;
            }
            let (samples, bad) = self.part_samples(index);
            if bad {
                let start = u64::from(index) * self.manifest.part_samples;
                damaged.push((start, start + samples.len() as u64));
            }
            each(&samples);
        }
        damaged
    }

    /// The spans with no audio: the manifest's, and the parts that fail their checks.
    pub fn gaps(&self) -> Vec<(u64, u64)> {
        let mut gaps = self.manifest.gaps.clone();
        for index in 0..self.manifest.parts {
            if check_part(&part_path(&self.dir, index)).is_err() {
                let start = u64::from(index) * self.manifest.part_samples;
                gaps.push((start, start + self.part_length(index) as u64));
            }
        }
        merge(gaps)
    }
}

/// A take an earlier run left behind (it stopped while recording, or before the take's copy was
/// handed to the cache): its raw recording file, its parts directory, and when it started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Leftover {
    /// `take-….pcm`, when it is still there.
    pub pcm: Option<PathBuf>,
    /// `take-….parts`, when the take kept a copy.
    pub dir: Option<PathBuf>,
    /// When the take started, Unix milliseconds (from the file's name), if it says.
    pub at_ms: Option<u64>,
}

/// Every take left behind in `recordings`, by name: `take-<ms>-<session>.pcm` and its `.parts`.
pub fn leftovers(recordings: &Path) -> Vec<Leftover> {
    let Ok(entries) = std::fs::read_dir(recordings) else { return Vec::new() };
    let mut by_stem: std::collections::BTreeMap<String, Leftover> = std::collections::BTreeMap::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let (Some(stem), Some(ext)) = (path.file_stem().and_then(|s| s.to_str()), path.extension().and_then(|e| e.to_str())) else { continue };
        if !stem.starts_with("take-") {
            continue;
        }
        let slot =
            by_stem.entry(stem.to_owned()).or_insert_with(|| Leftover { pcm: None, dir: None, at_ms: stem.split('-').nth(1).and_then(|ms| ms.parse().ok()) });
        match ext {
            "pcm" => slot.pcm = Some(path),
            PARTS_SUFFIX if path.is_dir() => slot.dir = Some(path),
            _ => {}
        }
    }
    by_stem.into_values().filter(|l| l.pcm.is_some() || l.dir.is_some()).collect()
}

/// Put a leftover take's copy together (docs/dictation.md §26.3): its parts that check out stay,
/// the rest of its raw recording file is encoded into parts, the manifest is written and the raw
/// file goes. `None` when it kept no copy (it is removed then) or there is nothing in it. Safe to
/// run again on what an interrupted run left: parts on disk are complete or absent.
pub fn recover(leftover: &Leftover, stop: &AtomicBool) -> Option<ArchiveReport> {
    let Some(dir) = &leftover.dir else {
        if let Some(pcm) = &leftover.pcm {
            super::long::remove(pcm);
        }
        return None;
    };
    if let Ok(manifest) = read_manifest(dir) {
        // The take finished; only handing it over did not.
        if let Some(pcm) = &leftover.pcm {
            super::long::remove(pcm);
        }
        let bytes = parts_bytes(dir, manifest.parts);
        return Some(ArchiveReport { dir: dir.clone(), manifest, bytes, error: None });
    }
    let pcm_samples = leftover.pcm.as_ref().and_then(|p| std::fs::metadata(p).ok()).map_or(0, |m| m.len() / 2);
    // The parts that check out, in order; a part's index fixes where it starts.
    let mut lengths: Vec<Option<u64>> = Vec::new();
    for index in 0.. {
        let path = part_path(dir, index);
        if !path.exists() {
            break;
        }
        lengths.push(check_part(&path).ok());
    }
    remove_tmp_files(dir);
    let total_parts = u64::max(lengths.len() as u64, pcm_samples.div_ceil(PART_SAMPLES));
    let mut gaps = Vec::new();
    let mut total = 0u64;
    let mut parts = 0u32;
    let mut error = None;
    for index in 0..total_parts {
        if stop.load(Ordering::SeqCst) {
            return None;
        }
        let start = index * PART_SAMPLES;
        let i = u32::try_from(index).unwrap_or(u32::MAX);
        let on_disk = lengths.get(usize::try_from(index).unwrap_or(usize::MAX)).copied().flatten();
        let length = match on_disk {
            Some(length) => length,
            None if start < pcm_samples => {
                let end = pcm_samples.min(start + PART_SAMPLES);
                match leftover.pcm.as_deref().map(|p| encode_from_pcm(p, dir, i, start, end)) {
                    Some(Ok(())) => end - start,
                    Some(Err(e)) => {
                        error = Some(e.to_string());
                        gaps.push((start, end));
                        end - start
                    }
                    None => 0,
                }
            }
            None => {
                // Damaged, and the raw file does not reach it: a full part of nothing.
                let _ = std::fs::remove_file(part_path(dir, i));
                gaps.push((start, start + PART_SAMPLES));
                PART_SAMPLES
            }
        };
        total = start + length;
        parts = i + 1;
    }
    if total == 0 {
        let _ = std::fs::remove_dir_all(dir);
        if let Some(pcm) = &leftover.pcm {
            super::long::remove(pcm);
        }
        return None;
    }
    let manifest = Manifest { schema: MANIFEST_SCHEMA, total, part_samples: PART_SAMPLES, parts, cuts: Vec::new(), gaps: merge(gaps) };
    if let Err(e) = write_manifest(dir, &manifest) {
        tracing::warn!(dir = %dir.display(), error = %e, "a recovered take's manifest could not be written; trying again at the next start");
        return None;
    }
    if let Some(pcm) = &leftover.pcm {
        super::long::remove(pcm);
    }
    tracing::info!(dir = %dir.display(), seconds = total / RATE, parts, "recovered a take an earlier run left behind");
    Some(ArchiveReport { dir: dir.clone(), bytes: parts_bytes(dir, parts), manifest, error })
}

/// Encode samples `start..end` of the raw recording file `pcm` as part `index` of `dir`.
fn encode_from_pcm(pcm: &Path, dir: &Path, index: u32, start: u64, end: u64) -> std::io::Result<()> {
    let mut file = File::open(pcm)?;
    std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(start * 2))?;
    let mut raw = vec![0u8; usize::try_from((end - start) * 2).map_err(std::io::Error::other)?];
    let read = read_up_to(&mut file, &mut raw)?;
    raw.truncate(read - read % 2);
    let samples: Vec<i16> = raw.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();
    let mut part = PartEncoder::new().map_err(std::io::Error::other)?;
    part.push(&samples);
    write_durable(&part_path(dir, index), &part.finish())
}

/// Fill `buf` as far as the file goes; the bytes read.
fn read_up_to(file: &mut File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match file.read(&mut buf[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

/// The parts' size in all.
pub fn parts_bytes(dir: &Path, parts: u32) -> u64 {
    (0..parts).filter_map(|i| std::fs::metadata(part_path(dir, i)).ok()).map(|m| m.len()).sum()
}

/// Remove what an interrupted write left: `*.tmp`.
fn remove_tmp_files(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if entry.path().extension().is_some_and(|e| e == "tmp") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// `dir`'s manifest.
pub fn read_manifest(dir: &Path) -> std::io::Result<Manifest> {
    let bytes = std::fs::read(dir.join(MANIFEST))?;
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
    if manifest.schema != MANIFEST_SCHEMA || manifest.part_samples == 0 {
        return Err(std::io::Error::other(format!("unsupported manifest (schema {})", manifest.schema)));
    }
    Ok(manifest)
}

fn write_manifest(dir: &Path, manifest: &Manifest) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(manifest).map_err(std::io::Error::other)?;
    write_durable(&dir.join(MANIFEST), &bytes)
}

/// Adjacent or overlapping spans as one.
fn merge(mut spans: Vec<(u64, u64)>) -> Vec<(u64, u64)> {
    spans.retain(|s| s.1 > s.0);
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

/// A directory readable by this user only (Unix).
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// Write `bytes` as `path` so that the file on disk is either complete or absent: a temporary
/// file, flushed to the disk, renamed over `path`, and the directory flushed so the rename stays
/// (Unix; Windows flushes it with the file). This user's only, on Unix.
pub(crate) fn write_durable(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let _ = std::fs::remove_file(&tmp);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = options.open(&tmp).and_then(|mut file| {
        file.write_all(bytes)?;
        file.sync_data()
    });
    if let Err(e) = written.and_then(|()| std::fs::rename(&tmp, path)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    sync_dir(path.parent());
    Ok(())
}

/// Flush a directory's entries to the disk (Unix; elsewhere there is nothing to flush).
pub(crate) fn sync_dir(dir: Option<&Path>) {
    #[cfg(unix)]
    if let Some(dir) = dir
        && let Err(e) = File::open(dir).and_then(|d| d.sync_all())
    {
        tracing::debug!(dir = %dir.display(), error = %e, "a directory could not be flushed");
    }
    #[cfg(not(unix))]
    let _ = dir;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Speech-like: a voiced tone with a wandering pitch in syllables of 250 ms, `seconds` long.
    fn speech(seconds: f32) -> Vec<i16> {
        let n = (seconds * RATE as f32) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / RATE as f32;
                let f0 = 140.0 + 30.0 * (t * 0.7).sin();
                let syllable = (0.5 + 0.5 * (t * std::f32::consts::TAU * 4.0).sin()).powi(2);
                let v = syllable * ((t * f0 * std::f32::consts::TAU).sin() * 0.5 + (t * f0 * 2.0 * std::f32::consts::TAU).sin() * 0.25);
                (v * 12_000.0) as i16
            })
            .collect()
    }

    /// Whether `decoded` is `original` as a speech codec gives it back: the same loudness in every
    /// 20 ms, in place. Opus at this rate (SILK) keeps the spectrum, not the waveform, so the
    /// check is the envelope's correlation and the overall level, not a sample-by-sample error.
    fn sounds_like(original: &[i16], decoded: &[i16]) -> bool {
        let envelope =
            |x: &[i16]| -> Vec<f64> { x.chunks(FRAME).map(|f| (f.iter().map(|&v| f64::from(v).powi(2)).sum::<f64>() / f.len() as f64).sqrt()).collect() };
        let (a, b) = (envelope(original), envelope(decoded));
        let n = a.len().min(b.len());
        let mean = |x: &[f64]| x[..n].iter().sum::<f64>() / n as f64;
        let (ma, mb) = (mean(&a), mean(&b));
        let cov: f64 = (0..n).map(|i| (a[i] - ma) * (b[i] - mb)).sum();
        let var = |x: &[f64], m: f64| (0..n).map(|i| (x[i] - m).powi(2)).sum::<f64>();
        let correlation = cov / (var(&a, ma) * var(&b, mb)).sqrt().max(f64::MIN_POSITIVE);
        let level = mb / ma.max(f64::MIN_POSITIVE);
        correlation > 0.9 && (0.6..1.4).contains(&level)
    }

    #[test]
    fn a_take_is_kept_in_parts_that_read_back_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let parts = dir.path().join("take-1-1.parts");
        let audio = speech(65.5);
        let mut writer = ArchiveWriter::create(&parts).unwrap();
        // Pushed in uneven blocks, as the recording thread reads them.
        for block in audio.chunks(4093) {
            writer.push(block);
        }
        let report = writer.finish(vec![480_000, 1_048_000], vec![(100, 200)]);
        assert!(report.error.is_none(), "{report:?}");
        assert_eq!(
            report.manifest,
            Manifest { schema: 1, total: 1_048_000, part_samples: PART_SAMPLES, parts: 3, cuts: vec![480_000, 1_048_000], gaps: vec![(100, 200)] }
        );
        assert_eq!(report.bytes, parts_bytes(&parts, 3));
        // About 22 kbit/s: a tenth of the 256 kbit/s raw audio.
        assert!(report.bytes < 65 * 3_300 && report.bytes > 65 * 2_000, "{} bytes", report.bytes);
        assert_eq!(read_manifest(&parts).unwrap(), report.manifest);
        let mut names: Vec<String> = std::fs::read_dir(&parts).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["manifest.json", "part-00000.opus", "part-00001.opus", "part-00002.opus"], "no temporary file is left");
        // Each part decodes on its own to its exact length, and close to the original.
        for (index, expected) in [(0, PART_SAMPLES), (1, PART_SAMPLES), (2, 88_000)] {
            let samples = read_part(&part_path(&parts, index)).unwrap();
            assert_eq!(samples.len() as u64, expected, "part {index}");
            assert_eq!(check_part(&part_path(&parts, index)).unwrap(), expected);
            let start = usize::try_from(u64::from(index) * PART_SAMPLES).unwrap();
            assert!(sounds_like(&audio[start..start + samples.len()], &samples), "part {index}");
        }
        // A segment across two parts is the original span, as a WAV.
        let kept = KeptLong::open(&parts).unwrap();
        let wav = kept.read_segment(470_000, 500_000);
        let pcm: Vec<i16> = wav::pcm_data(&wav).unwrap().as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();
        assert_eq!(pcm.len(), 30_000);
        assert!(sounds_like(&audio[470_000..500_000], &pcm));
        assert_eq!(wav::pcm_data(&kept.read_segment(1_040_000, 2_000_000)).unwrap().len(), 16_000, "a segment ends with the take");
        assert_eq!(kept.gaps(), vec![(100, 200)]);
        let stop = AtomicBool::new(false);
        let mut seen = 0usize;
        assert!(kept.for_each_part(&stop, |s| seen += s.len()).is_empty());
        assert_eq!(seen, 1_048_000);
    }

    #[cfg(unix)]
    #[test]
    fn the_parts_are_this_users_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let parts = dir.path().join("take-1-1.parts");
        let mut writer = ArchiveWriter::create(&parts).unwrap();
        writer.push(&speech(1.0));
        writer.finish(Vec::new(), Vec::new());
        assert_eq!(std::fs::metadata(&parts).unwrap().permissions().mode() & 0o777, 0o700);
        assert_eq!(std::fs::metadata(part_path(&parts, 0)).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(parts.join(MANIFEST)).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn a_damaged_part_reads_as_silence_and_counts_as_a_gap() {
        let dir = tempfile::tempdir().unwrap();
        let parts = dir.path().join("take-1-1.parts");
        let mut writer = ArchiveWriter::create(&parts).unwrap();
        writer.push(&speech(70.0));
        writer.finish(Vec::new(), Vec::new());
        let path = part_path(&parts, 1);
        let mut bytes = std::fs::read(&path).unwrap();
        let at = bytes.len() / 2;
        bytes[at] ^= 0x55;
        std::fs::write(&path, bytes).unwrap();
        assert!(matches!(check_part(&path), Err(PartError::Damaged(_))));
        let kept = KeptLong::open(&parts).unwrap();
        assert_eq!(kept.gaps(), vec![(PART_SAMPLES, 2 * PART_SAMPLES)]);
        let wav = kept.read_segment(PART_SAMPLES, PART_SAMPLES + 16_000);
        assert!(wav::is_silent(&wav), "the damaged part is silence");
        assert!(!wav::is_silent(&kept.read_segment(0, 16_000)), "the others still read");
        let stop = AtomicBool::new(false);
        assert_eq!(kept.for_each_part(&stop, |_| {}), vec![(PART_SAMPLES, 2 * PART_SAMPLES)]);
        std::fs::remove_file(&path).unwrap();
        assert!(matches!(read_part(&path), Err(PartError::Io(_))));
        assert!(read_part(&path).unwrap_err().to_string().contains("could not be read"));
    }

    /// An app that stopped mid-take left the raw file and the parts written so far (and possibly a
    /// part's temporary file): the complete parts stay, the rest comes from the raw file.
    #[test]
    fn a_take_left_behind_is_put_together_from_its_parts_and_its_raw_file() {
        let dir = tempfile::tempdir().unwrap();
        let recordings = dir.path().join("recordings");
        std::fs::create_dir_all(&recordings).unwrap();
        let pcm = recordings.join("take-1758700000000-3.pcm");
        let audio = speech(75.0);
        let raw: Vec<u8> = audio.iter().flat_map(|s| s.to_le_bytes()).collect();
        // A torn last write: an odd byte at the end.
        std::fs::write(&pcm, [&raw[..], &[7u8]].concat()).unwrap();
        let parts = parts_dir(&pcm);
        let mut writer = ArchiveWriter::create(&parts).unwrap();
        writer.push(&audio[..usize::try_from(PART_SAMPLES).unwrap() + 1000]);
        // Dropped without `finish`: one complete part on disk, as when the app stops.
        drop(writer);
        std::fs::write(parts.join("part-00001.opus.tmp"), b"half a part").unwrap();
        std::fs::write(recordings.join("notes.txt"), "not a take").unwrap();

        let found = leftovers(&recordings);
        assert_eq!(found, vec![Leftover { pcm: Some(pcm.clone()), dir: Some(parts.clone()), at_ms: Some(1_758_700_000_000) }]);
        let stop = AtomicBool::new(false);
        let report = recover(&found[0], &stop).unwrap();
        assert_eq!(report.manifest.total, 1_200_000);
        assert_eq!(report.manifest.parts, 3);
        assert!(report.manifest.cuts.is_empty() && report.manifest.gaps.is_empty(), "{:?}", report.manifest);
        assert!(report.error.is_none());
        assert!(!pcm.exists(), "the raw file goes once the copy is complete");
        assert!(!parts.join("part-00001.opus.tmp").exists());
        assert!(recordings.join("notes.txt").exists(), "only takes are touched");
        let kept = KeptLong::open(&parts).unwrap();
        let tail = kept.read_segment(1_100_000, 1_200_000);
        let pcm_tail: Vec<i16> = wav::pcm_data(&tail).unwrap().as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();
        assert!(sounds_like(&audio[1_100_000..1_200_000], &pcm_tail));
        // Running it again finds the finished copy.
        let again = recover(&Leftover { pcm: None, dir: Some(parts.clone()), at_ms: None }, &stop).unwrap();
        assert_eq!(again.manifest, report.manifest);
    }

    #[test]
    fn a_take_without_a_copy_or_without_audio_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let pcm = dir.path().join("take-5-1.pcm");
        std::fs::write(&pcm, [0u8; 64]).unwrap();
        let stop = AtomicBool::new(false);
        assert!(recover(&Leftover { pcm: Some(pcm.clone()), dir: None, at_ms: Some(5) }, &stop).is_none());
        assert!(!pcm.exists(), "a take that kept no copy goes, as before");
        let parts = dir.path().join("take-6-1.parts");
        std::fs::create_dir_all(&parts).unwrap();
        assert!(recover(&Leftover { pcm: None, dir: Some(parts.clone()), at_ms: Some(6) }, &stop).is_none());
        assert!(!parts.exists(), "an empty copy goes");
    }

    /// A damaged part the raw file no longer covers (a power cut lost the file's tail) is a gap of
    /// a full part; the take goes on after it.
    #[test]
    fn a_damaged_part_beyond_the_raw_file_is_a_gap() {
        let dir = tempfile::tempdir().unwrap();
        let parts = dir.path().join("take-9-2.parts");
        let mut writer = ArchiveWriter::create(&parts).unwrap();
        writer.push(&speech(95.0));
        drop(writer);
        std::fs::write(part_path(&parts, 1), b"OggS damaged").unwrap();
        let stop = AtomicBool::new(false);
        let report = recover(&Leftover { pcm: None, dir: Some(parts.clone()), at_ms: Some(9) }, &stop).unwrap();
        assert_eq!(report.manifest.parts, 3);
        assert_eq!(report.manifest.total, 3 * PART_SAMPLES);
        assert_eq!(report.manifest.gaps, vec![(PART_SAMPLES, 2 * PART_SAMPLES)]);
        assert!(!part_path(&parts, 1).exists());
        let kept = KeptLong::open(&parts).unwrap();
        assert_eq!(kept.gaps(), vec![(PART_SAMPLES, 2 * PART_SAMPLES)]);
    }

    #[test]
    fn a_stopped_recovery_writes_nothing_final() {
        let dir = tempfile::tempdir().unwrap();
        let pcm = dir.path().join("take-7-1.pcm");
        let raw: Vec<u8> = speech(40.0).iter().flat_map(|s| s.to_le_bytes()).collect();
        std::fs::write(&pcm, raw).unwrap();
        let parts = parts_dir(&pcm);
        std::fs::create_dir_all(&parts).unwrap();
        let stop = AtomicBool::new(true);
        assert!(recover(&Leftover { pcm: Some(pcm.clone()), dir: Some(parts.clone()), at_ms: Some(7) }, &stop).is_none());
        assert!(pcm.exists() && !parts.join(MANIFEST).exists(), "left for the next start");
    }

    /// A part that cannot be written ends the copy there; its manifest still says what it holds.
    #[test]
    fn a_part_that_cannot_be_written_ends_the_copy_with_a_gap() {
        let dir = tempfile::tempdir().unwrap();
        let parts = dir.path().join("take-3-1.parts");
        let mut writer = ArchiveWriter::create(&parts).unwrap();
        writer.push(&speech(31.0));
        // Where the second part goes, a directory: renaming it into place fails.
        std::fs::create_dir_all(part_path(&parts, 1).join("in-the-way")).unwrap();
        writer.push(&speech(40.0));
        let report = writer.finish(vec![PART_SAMPLES], Vec::new());
        assert!(report.error.is_some(), "{report:?}");
        assert_eq!((report.manifest.parts, report.manifest.total), (1, 71 * RATE));
        assert_eq!(report.manifest.gaps, vec![(PART_SAMPLES, 71 * RATE)]);
        assert_eq!(report.bytes, parts_bytes(&parts, 1));
        assert!(!parts.join("part-00001.opus.tmp").exists(), "no temporary file is left");
        assert_eq!(read_manifest(&parts).unwrap(), report.manifest);
        let kept = KeptLong::open(&parts).unwrap();
        assert_eq!(kept.gaps(), vec![(PART_SAMPLES, 71 * RATE)]);
        assert!(!wav::is_silent(&kept.read_segment(0, 16_000)), "the part written still reads");
    }

    #[test]
    fn an_unknown_manifest_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(MANIFEST), br#"{"schema":9,"total":1,"part_samples":1,"parts":1}"#).unwrap();
        assert!(KeptLong::open(dir.path()).is_err());
        std::fs::write(dir.path().join(MANIFEST), b"not json").unwrap();
        assert!(read_manifest(dir.path()).is_err());
    }
}
