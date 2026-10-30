#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Every container and codec the import reads (docs/dictation.md §31), on the files in
//! `tests/fixtures` (how they were made: `tests/fixtures/README.md`): 1.5 s of a 440 Hz tone, then
//! 0.5 s of silence. Each must come out as 16 kHz mono 16-bit samples, two seconds long within one
//! frame of its codec, the silence where it was and the tone at its pitch; the errors name what is
//! wrong.

use std::cell::Cell;
use std::path::{Path, PathBuf};

use voltip_core::CancelToken;
use voltip_core::import::{MediaDecoder, MediaError, PcmInfo};
use voltip_media::Decoder;

/// Samples per second of the output.
const RATE: usize = 16_000;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn read_pcm(path: &Path) -> Vec<f32> {
    std::fs::read(path).unwrap().as_chunks::<2>().0.iter().map(|b| f32::from(i16::from_le_bytes(*b)) / 32_768.0).collect()
}

fn decode_path(path: &Path) -> (Result<PcmInfo, MediaError>, Option<Vec<f32>>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.pcm");
    let result = Decoder.decode_to_pcm(path, &out, &|_, _| {}, &CancelToken::new());
    let samples = out.exists().then(|| read_pcm(&out));
    (result, samples, dir)
}

fn decode(name: &str) -> (PcmInfo, Vec<f32>) {
    let (result, samples, _dir) = decode_path(&fixture(name));
    let info = result.unwrap_or_else(|e| panic!("{name}: {e}"));
    let samples = samples.unwrap_or_else(|| panic!("{name}: no output"));
    (info, samples)
}

fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len().max(1) as f32).sqrt()
}

/// Where the tone stops: the last 10 ms block louder than a tenth of the tone's level, in seconds.
fn tone_end(samples: &[f32]) -> f64 {
    let block = RATE / 100;
    let last = samples.chunks(block).rposition(|b| rms(b) > 0.035).expect("some tone");
    (last + 1) as f64 * block as f64 / RATE as f64
}

/// The tone's pitch from its zero crossings between 0.2 s and 1.3 s.
fn pitch(samples: &[f32]) -> f64 {
    let span = &samples[RATE / 5..RATE * 13 / 10];
    let crossings = span.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
    crossings as f64 / 2.0 / (span.len() as f64 / RATE as f64)
}

/// `(file, container, codec, one frame of the codec in seconds)`.
const FILES: [(&str, &str, &str, f64); 10] = [
    ("tone.wav", "wav", "pcm", 1.0 / 12_000.0),
    ("tone.mp3", "mp3", "mp3", 1152.0 / 44_100.0),
    ("tone.m4a", "mp4", "aac", 1024.0 / 48_000.0),
    ("tone.flac", "flac", "flac", 1.0 / 16_000.0),
    ("tone.ogg", "ogg", "vorbis", 2048.0 / 48_000.0),
    ("tone.opus", "ogg", "opus", 960.0 / 48_000.0),
    ("tone.mp4", "mp4", "aac", 1024.0 / 48_000.0),
    ("tone.mov", "mov", "aac", 1024.0 / 44_100.0),
    ("tone.mkv", "mkv", "flac", 1.0 / 16_000.0),
    ("tone.webm", "webm", "opus", 960.0 / 48_000.0),
];

#[test]
fn every_container_and_codec_comes_out_as_two_seconds_of_16_khz_mono() {
    for (name, container, codec, frame) in FILES {
        let probed = Decoder.probe(&fixture(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!((probed.container.as_str(), probed.codec.as_str(), probed.audio_tracks), (container, codec, 1), "{name}: {probed:?}");
        let length = probed.duration_ms.unwrap_or_else(|| panic!("{name}: no length")) as f64 / 1000.0;
        assert!((length - 2.0).abs() <= frame + 0.04, "{name}: probed length {length}");

        let (info, samples) = decode(name);
        assert_eq!(info.info, probed, "{name}: the decode reports what the probe saw");
        assert_eq!(info.samples, samples.len() as u64, "{name}");
        let seconds = samples.len() as f64 / RATE as f64;
        assert!((seconds - 2.0).abs() <= frame, "{name}: {seconds} s, one frame is {frame} s");
        let level = rms(&samples[RATE / 10..RATE * 14 / 10]);
        assert!((0.3..0.42).contains(&level), "{name}: the tone's level {level} (a half-scale sine is 0.354)");
        let quiet = rms(&samples[RATE * 16 / 10..RATE * 19 / 10]);
        assert!(quiet < 0.01, "{name}: the silence's level {quiet}");
        let end = tone_end(&samples);
        assert!((end - 1.5).abs() <= frame.max(0.01) + 0.01, "{name}: the tone ends at {end} s");
        let hz = pitch(&samples);
        assert!((hz - 440.0).abs() < 440.0 * 0.02, "{name}: {hz} Hz");
    }
}

/// A raw AAC stream (ADTS, `.aac`) has no edit list to drop the encoder's priming by: the tone
/// starts up to two frames late and the stream ends padded, which a subtitle's time hardly notices.
/// Its length is not in the file: the reader's estimate (2.6 s for this one) is no proof that it was
/// cut off, nor is an MP3's without an encoder header (here: the file cut in half).
#[test]
fn regression_a_length_the_reader_estimates_is_no_proof_of_damage() {
    let probed = Decoder.probe(&fixture("tone.aac")).unwrap();
    assert_eq!((probed.container.as_str(), probed.codec.as_str(), probed.audio_tracks, probed.sample_rate), ("adts", "aac", 1, 22_050));
    let (_, samples) = decode("tone.aac");
    let seconds = samples.len() as f64 / RATE as f64;
    let frame = 1024.0 / 22_050.0;
    assert!((2.0..=2.0 + 3.0 * frame).contains(&seconds), "{seconds} s");
    let end = tone_end(&samples);
    assert!((1.5..=1.5 + 2.0 * frame + 0.01).contains(&end), "the tone ends at {end} s");
    let hz = pitch(&samples);
    assert!((hz - 440.0).abs() < 440.0 * 0.02, "{hz} Hz");

    let dir = tempfile::tempdir().unwrap();
    let cut = dir.path().join("cut.mp3");
    let bytes = std::fs::read(fixture("tone.mp3")).unwrap();
    std::fs::write(&cut, &bytes[..bytes.len() / 2]).unwrap();
    let (result, samples, _out) = decode_path(&cut);
    let info = result.unwrap();
    let seconds = info.samples as f64 / RATE as f64;
    assert!((0.5..1.5).contains(&seconds) && samples.is_some(), "what is there is decoded: {seconds} s");
}

/// The open dialog and a drop take the files of these extensions: every fixture's is one, and the
/// list says each once, lowercase.
#[test]
fn every_file_the_decoder_reads_has_an_extension_the_import_takes() {
    let extensions = Decoder.extensions();
    assert_eq!(extensions, voltip_media::EXTENSIONS);
    for (name, ..) in FILES.iter().chain(&[("tone.aac", "", "", 0.0)]) {
        let ext = Path::new(name).extension().unwrap().to_str().unwrap();
        assert!(extensions.contains(&ext), "{name}");
    }
    let mut sorted = extensions.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), extensions.len(), "{extensions:?}");
    assert!(extensions.iter().all(|e| !e.is_empty() && !e.starts_with('.') && e.to_lowercase() == *e), "{extensions:?}");
}

/// Two channels are mixed by their average: the same signal on both keeps its level, a signal and
/// its inverse cancel out.
#[test]
fn stereo_is_mixed_by_averaging_the_channels() {
    let (info, samples) = decode("tone.m4a");
    assert_eq!(info.info.channels, 2);
    let level = rms(&samples[RATE / 10..RATE * 14 / 10]);
    assert!((0.3..0.42).contains(&level), "{level}: not summed, not one channel dropped");

    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("inverse.wav");
    let frames: Vec<i16> = (0..RATE).map(|i| ((i as f64 * 440.0 * std::f64::consts::TAU / RATE as f64).sin() * 16_000.0) as i16).collect();
    let mut bytes = wav_header(2, RATE as u32, frames.len() * 4);
    for v in frames {
        bytes.extend_from_slice(&v.to_le_bytes());
        bytes.extend_from_slice(&(-v).to_le_bytes());
    }
    std::fs::write(&wav, bytes).unwrap();
    let (result, samples, _out) = decode_path(&wav);
    assert_eq!(result.unwrap().info.channels, 2);
    let samples = samples.unwrap();
    assert_eq!(samples.len(), RATE);
    assert!(rms(&samples) < 0.001, "{}: the channels cancel", rms(&samples));
}

/// A 16-bit PCM WAV header for `data_bytes` of samples.
fn wav_header(channels: u16, rate: u32, data_bytes: usize) -> Vec<u8> {
    let data = u32::try_from(data_bytes).unwrap();
    let mut h = Vec::with_capacity(44);
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&(36 + data).to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes());
    h.extend_from_slice(&channels.to_le_bytes());
    h.extend_from_slice(&rate.to_le_bytes());
    h.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
    h.extend_from_slice(&(channels * 2).to_le_bytes());
    h.extend_from_slice(&16u16.to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&data.to_le_bytes());
    h
}

#[test]
fn what_cannot_be_decoded_says_why() {
    // No audio track at all.
    assert_eq!(Decoder.probe(&fixture("video-only.mp4")), Err(MediaError::NoAudio));
    assert_eq!(decode_path(&fixture("video-only.mp4")).0, Err(MediaError::NoAudio));
    // A codec there is no decoder for: named.
    assert_eq!(Decoder.probe(&fixture("ac3.mkv")), Err(MediaError::UnsupportedCodec("AC-3".into())));
    let (result, samples, _dir) = decode_path(&fixture("ac3.mkv"));
    assert_eq!(result, Err(MediaError::UnsupportedCodec("AC-3".into())));
    assert!(samples.is_none(), "nothing written");
    // One the reader knows but no decoder is built in for (Symphonia's ADPCM is left out).
    let (result, samples, _dir) = decode_path(&fixture("adpcm.wav"));
    assert_eq!(result, Err(MediaError::UnsupportedCodec("ADPCM".into())));
    assert!(samples.is_none(), "nothing written");
    // Not a media file.
    let dir = tempfile::tempdir().unwrap();
    let text = dir.path().join("notes.mp3");
    std::fs::write(&text, "这不是音频文件。".repeat(200)).unwrap();
    assert_eq!(Decoder.probe(&text), Err(MediaError::UnsupportedFormat));
    assert_eq!(decode_path(&text).0, Err(MediaError::UnsupportedFormat));
    // Gone.
    assert!(matches!(Decoder.probe(&dir.path().join("missing.wav")), Err(MediaError::Unreadable(_))));
    assert!(matches!(decode_path(&dir.path().join("missing.wav")).0, Err(MediaError::Unreadable(_))));
}

/// A file cut off is damaged, whether what is missing is the index (an MP4's `moov` at its end)
/// or the samples a header announced.
#[test]
fn a_file_cut_off_is_damaged_and_leaves_nothing_behind() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["tone.m4a", "tone.wav", "tone.flac"] {
        let bytes = std::fs::read(fixture(name)).unwrap();
        let cut = dir.path().join(format!("cut-{name}"));
        std::fs::write(&cut, &bytes[..bytes.len() / 2]).unwrap();
        let out = dir.path().join(format!("{name}.pcm"));
        let result = Decoder.decode_to_pcm(&cut, &out, &|_, _| {}, &CancelToken::new());
        assert!(matches!(result, Err(MediaError::Corrupt(_))), "{name}: {result:?}");
        assert!(!out.exists() && !part(&out).exists(), "{name}: nothing left behind");
    }
}

fn part(out: &Path) -> PathBuf {
    let mut name = out.file_name().unwrap().to_os_string();
    name.push(".part");
    out.with_file_name(name)
}

/// The decode reports its way through the file's time and stops at a cancel, leaving nothing.
#[test]
fn progress_follows_the_file_and_a_cancel_stops_it_leaving_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out.pcm");
    let reports = std::cell::RefCell::new(Vec::new());
    Decoder.decode_to_pcm(&fixture("tone.mp3"), &out, &|done, total| reports.borrow_mut().push((done, total)), &CancelToken::new()).unwrap();
    let reports = reports.into_inner();
    assert!(!reports.is_empty());
    assert!(reports.windows(2).all(|w| w[0].0 <= w[1].0), "{reports:?}");
    let (last, total) = *reports.last().unwrap();
    let total = total.expect("the length is known");
    assert!(last <= total && total.abs_diff(2_000) <= 40, "{reports:?}");
    assert!(last + 100 >= total, "the end is reported: {reports:?}");

    let cancel = CancelToken::new();
    let seen = Cell::new(0);
    let stop = |_: u64, _: Option<u64>| {
        seen.set(seen.get() + 1);
        cancel.cancel();
    };
    let out = dir.path().join("cancelled.pcm");
    assert_eq!(Decoder.decode_to_pcm(&fixture("tone.wav"), &out, &stop, &cancel), Err(MediaError::Cancelled));
    assert!(seen.get() >= 1);
    assert!(!out.exists() && !part(&out).exists());
    // Cancelled before it starts.
    assert_eq!(Decoder.decode_to_pcm(&fixture("tone.ogg"), &out, &|_, _| {}, &cancel), Err(MediaError::Cancelled));
    assert!(!out.exists() && !part(&out).exists());
}

/// Opus streams start with the encoder's pre-skip (312 samples, 6.5 ms at 48 kHz, in both files):
/// dropped, the tone ends where it did, and the Ogg file, which also says where its last packet
/// ends, is exactly two seconds long.
#[test]
fn opus_drops_its_pre_skip() {
    for name in ["tone.opus", "tone.webm"] {
        let (_, samples) = decode(name);
        let end = tone_end(&samples);
        assert!((end - 1.5).abs() < 0.005, "{name}: the tone ends at {end} s");
    }
    assert_eq!(decode("tone.opus").1.len(), 2 * RATE);
}

/// The original file is only read.
#[test]
fn the_original_file_is_left_as_it_was() {
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("tone.webm");
    std::fs::copy(fixture("tone.webm"), &copy).unwrap();
    let before = std::fs::read(&copy).unwrap();
    let modified = std::fs::metadata(&copy).unwrap().modified().unwrap();
    let (result, _, _out) = decode_path(&copy);
    result.unwrap();
    assert_eq!(std::fs::read(&copy).unwrap(), before);
    assert_eq!(std::fs::metadata(&copy).unwrap().modified().unwrap(), modified);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "nothing written beside it");
}

/// Two Ogg streams one after the other (a chained file, as some recorders and streams write it):
/// the reader starts the second one with a new decoder and its sound follows the first's.
#[test]
fn a_chained_ogg_file_goes_on_into_its_next_stream() {
    let dir = tempfile::tempdir().unwrap();
    let one = std::fs::read(fixture("tone.ogg")).unwrap();
    let chained = dir.path().join("chained.ogg");
    std::fs::write(&chained, [one.as_slice(), one.as_slice()].concat()).unwrap();
    let (result, samples, _out) = decode_path(&chained);
    let info = result.unwrap();
    let samples = samples.unwrap();
    let seconds = samples.len() as f64 / RATE as f64;
    assert_eq!(info.samples, samples.len() as u64);
    assert!((3.9..=4.1).contains(&seconds), "{seconds:.3} s");
    // The second stream's tone starts where the first one ended.
    let second = &samples[2 * RATE..];
    assert!(rms(&second[RATE / 5..RATE]) > 0.2, "the second stream's tone is there");
    assert!((pitch(second) - 440.0).abs() < 5.0, "{}", pitch(second));
}

/// Packets that do not decode in the middle of a file are left out and the rest is kept.
#[test]
fn packets_that_do_not_decode_are_left_out() {
    let dir = tempfile::tempdir().unwrap();
    let mut bytes = std::fs::read(fixture("tone.mp3")).unwrap();
    // Every byte of the middle third but the frames' sync words: frames whose contents are noise.
    let (from, to) = (bytes.len() / 3, bytes.len() * 2 / 3);
    for (i, byte) in bytes[from..to].iter_mut().enumerate() {
        if *byte != 0xff {
            *byte = (i as u8).wrapping_mul(37) ^ 0x5a;
        }
    }
    let damaged = dir.path().join("damaged.mp3");
    std::fs::write(&damaged, &bytes).unwrap();
    let (result, samples, _out) = decode_path(&damaged);
    let info = result.unwrap();
    let samples = samples.unwrap();
    assert_eq!(info.samples, samples.len() as u64);
    let (_, whole) = decode("tone.mp3");
    assert!(samples.len() < whole.len(), "the damaged frames are left out: {} of {} samples", samples.len(), whole.len());
    assert!(rms(&samples[..RATE / 2]) > 0.2, "the tone before the damage is kept");
}
