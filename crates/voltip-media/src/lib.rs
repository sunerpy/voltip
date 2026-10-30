//! Imported audio and video files in the app's recording format (docs/dictation.md §31): the
//! audio track of a WAV, MP3, AAC/ALAC (MP4, M4A, MOV), FLAC, Ogg (Vorbis, Opus) or Matroska/WebM
//! file is decoded packet by packet, mixed to mono by the channels' average, brought to 16 kHz by
//! the local speech service's resampler and written as 16-bit samples — the format a long take is
//! recorded in. Only one packet and one block of samples are held at a time, however long the
//! file; the file itself is only read.
//!
//! Symphonia reads the containers and decodes every codec but Opus, which `opus-rs` decodes. Of
//! several audio tracks the reader's default one is used, else the first; video tracks are passed
//! over. A track Symphonia has no decoder for is an error that names its codec ([`sniff`]).

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod mp4;
mod opus;
mod sniff;

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{self, CodecParameters, CodecType, DecoderOptions};
use symphonia::core::errors::Error;
use symphonia::core::formats::{FormatOptions, FormatReader, Packet, Track};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use voltip_core::CancelToken;
use voltip_core::import::{MediaDecoder, MediaError, MediaInfo, PcmInfo};
use voltip_serve::StreamResampler;

/// Samples per second of the output: the recording format.
pub const RATE: u32 = voltip_core::dictation::PCM_SAMPLE_RATE_HZ;
/// Mono samples handed to the resampler at a time.
const BLOCK: usize = 4096;
/// Progress is reported at most this often, in milliseconds of the file.
const PROGRESS_EVERY_MS: u64 = 250;

/// The file extensions of what the decoder reads: the containers above under their usual names.
pub const EXTENSIONS: &[&str] = &["wav", "mp3", "m4a", "m4b", "aac", "flac", "ogg", "oga", "opus", "mp4", "m4v", "mov", "mkv", "mka", "webm"];

/// The decoder the desktop plugs into the core ([`MediaDecoder`]), and that `--transcribe-file`
/// uses.
#[derive(Clone, Copy, Debug, Default)]
pub struct Decoder;

impl MediaDecoder for Decoder {
    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn probe(&self, path: &Path) -> Result<MediaInfo, MediaError> {
        probe(path)
    }

    fn decode_to_pcm(&self, path: &Path, out: &Path, progress: &dyn Fn(u64, Option<u64>), cancel: &CancelToken) -> Result<PcmInfo, MediaError> {
        decode_to_pcm(path, out, progress, cancel)
    }
}

/// What `path` holds ([`MediaDecoder::probe`]).
pub fn probe(path: &Path) -> Result<MediaInfo, MediaError> {
    Ok(Source::open(path)?.info)
}

/// Decode `path` into `out` ([`MediaDecoder::decode_to_pcm`]).
pub fn decode_to_pcm(path: &Path, out: &Path, progress: &dyn Fn(u64, Option<u64>), cancel: &CancelToken) -> Result<PcmInfo, MediaError> {
    if cancel.is_cancelled() {
        return Err(MediaError::Cancelled);
    }
    let mut source = Source::open(path)?;
    let part = part_path(out);
    let written = create(&part).and_then(|file| source.decode_into(file, progress, cancel)).and_then(|samples| {
        std::fs::rename(&part, out).map_err(|e| MediaError::Write(e.to_string()))?;
        Ok(samples)
    });
    match written {
        Ok(samples) => {
            tracing::info!(container = %source.info.container, codec = %source.info.codec, samples, "media file decoded");
            Ok(PcmInfo { samples, info: source.info })
        }
        Err(e) => {
            remove(&part);
            Err(e)
        }
    }
}

/// `<out>.part`.
fn part_path(out: &Path) -> PathBuf {
    let mut name = out.file_name().map(std::ffi::OsStr::to_os_string).unwrap_or_default();
    name.push(".part");
    out.with_file_name(name)
}

/// A new output file, readable and writable by this user only on Unix (like a recording file).
fn create(path: &Path) -> Result<File, MediaError> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path).map_err(|e| MediaError::Write(e.to_string()))
}

fn remove(path: &Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => tracing::warn!(path = %path.display(), error = %e, "a partly decoded file could not be removed"),
    }
}

/// The track's decoder.
enum Codec {
    Symphonia(Box<dyn codecs::Decoder>),
    /// Boxed: the decoder's state is a few kilobytes.
    Opus(Box<opus::Opus>),
}

/// An opened file: its reader, the track that is read and its decoder.
struct Source {
    format: Box<dyn FormatReader>,
    track: u32,
    codec: Codec,
    info: MediaInfo,
    /// The rate of the decoded frames (Opus: always 48 kHz).
    rate: u32,
    /// Decoded frames to drop at the start (an MP4 edit list's priming).
    skip: u64,
    /// Frames to keep after them, when the file says.
    keep: Option<u64>,
    /// Frames the file holds, as it says or as the reader estimates them (progress goes by it).
    expected: Option<u64>,
    /// `expected` is what the file says, not an estimate: fewer frames mean it was cut off. An MP3
    /// has an exact count only in an encoder's header, which may be missing (the reader then
    /// estimates it from the file's size), a raw AAC stream never.
    exact: bool,
    /// The track's first packet, read to learn its channels and not decoded yet.
    pending: Option<Packet>,
}

/// Whether `track` is an audio track: it has a codec Symphonia knows, or a sample rate (an audio
/// track in a codec it does not know). Video tracks have neither.
fn is_audio(track: &Track) -> bool {
    track.codec_params.codec != codecs::CODEC_TYPE_NULL || track.codec_params.sample_rate.is_some()
}

impl Source {
    fn open(path: &Path) -> Result<Self, MediaError> {
        let unreadable = |e: std::io::Error| MediaError::Unreadable(e.to_string());
        // Two handles: the reader's, and one for the reads of our own (the first bytes, an MP4's
        // edit list), which must not move the reader's position (a cloned handle would share it).
        let stream = File::open(path).map_err(unreadable)?;
        let mut file = File::open(path).map_err(unreadable)?;
        let mut head = Vec::with_capacity(64);
        Read::by_ref(&mut file).take(64).read_to_end(&mut head).map_err(unreadable)?;
        let container = sniff::container(&head);
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }
        // Gapless: the readers that know an encoder's delay and padding (MP3, Ogg) trim them.
        let options = FormatOptions { enable_gapless: true, ..FormatOptions::default() };
        let probed = symphonia::default::get_probe()
            .format(&hint, MediaSourceStream::new(Box::new(stream), Default::default()), &options, &MetadataOptions::default())
            .map_err(|e| match (e, container) {
                (Error::Unsupported(_), _) | (_, None) => MediaError::UnsupportedFormat,
                (e, Some(_)) => MediaError::Corrupt(e.to_string()),
            })?;
        let format = probed.format;
        let tracks: Vec<&Track> = format.tracks().iter().filter(|t| is_audio(t)).collect();
        let audio_tracks = u32::try_from(tracks.len()).unwrap_or(u32::MAX);
        let Some(track) = format.default_track().filter(|t| is_audio(t)).or(tracks.first().copied()).cloned() else {
            // The MP4 reader drops a track whose codec it does not know: name it, if one is there.
            return Err(sniff::unsupported_codec(&mut file).map_or(MediaError::NoAudio, |name| MediaError::UnsupportedCodec(name.to_owned())));
        };
        let params = &track.codec_params;
        let mut pre_skip = 0;
        let (codec, rate, channels) = if params.codec == codecs::CODEC_TYPE_NULL {
            let name = sniff::unsupported_codec(&mut file).unwrap_or("unknown");
            return Err(MediaError::UnsupportedCodec(name.to_owned()));
        } else if params.codec == codecs::CODEC_TYPE_OPUS {
            let head = opus::Head::parse(params.extra_data.as_deref().unwrap_or_default())?;
            let opus = opus::Opus::new(head)?;
            // An Ogg stream's granule positions count the pre-skip (RFC 7845 §4).
            pre_skip = if container == Some("ogg") { u64::from(head.pre_skip) } else { 0 };
            (Codec::Opus(Box::new(opus)), opus::RATE, u16::from(head.channels))
        } else {
            let decoder = symphonia::default::get_codecs().make(params, &DecoderOptions::default()).map_err(|e| match e {
                Error::Unsupported(_) => MediaError::UnsupportedCodec(codec_name(params.codec).to_owned()),
                e => MediaError::Corrupt(e.to_string()),
            })?;
            let rate = params.sample_rate.filter(|&r| r > 0).ok_or_else(|| MediaError::Corrupt("the audio track has no sample rate".into()))?;
            (Codec::Symphonia(decoder), rate, channels_of(params))
        };
        let mut source =
            Self { format, track: track.id, codec, info: MediaInfo::default(), rate, skip: 0, keep: None, expected: None, exact: false, pending: None };
        // An MP4's AAC track leaves its channels to the codec's own configuration: the first
        // packet says, and is decoded again from the start.
        let channels = if channels == 0 { source.first_channels() } else { channels };
        let container = container.unwrap_or("unknown").to_owned();
        let edit = if matches!(container.as_str(), "mp4" | "mov") { mp4::edit(&mut file, track.id as usize) } else { None };
        let to_frames = |units: u64| frames_at(params, units, rate);
        let skip = edit.map_or(0, |e| to_frames(e.skip));
        let keep = edit.and_then(|e| e.keep).map(to_frames);
        let expected = keep.or_else(|| params.n_frames.map(to_frames).map(|f| f.saturating_sub(skip + pre_skip)));
        source.info = MediaInfo {
            container,
            codec: codec_name(params.codec).to_owned(),
            channels,
            sample_rate: params.sample_rate.unwrap_or(rate),
            duration_ms: expected.map(|f| f * 1000 / u64::from(rate)),
            audio_tracks,
        };
        source.exact = !matches!(source.info.container.as_str(), "mp3" | "adts");
        (source.skip, source.keep, source.expected) = (skip, keep, expected);
        Ok(source)
    }

    /// The channels of the track's first packet, which is kept for [`Source::decode_into`] and the
    /// decoder reset; `0` when none decodes.
    fn first_channels(&mut self) -> u16 {
        let Codec::Symphonia(decoder) = &mut self.codec else { return 0 };
        loop {
            match self.format.next_packet() {
                Ok(packet) if packet.track_id() != self.track => continue,
                Ok(packet) => {
                    let channels = decoder.decode(&packet).map(|d| d.spec().channels.count()).unwrap_or(0);
                    decoder.reset();
                    self.pending = Some(packet);
                    return u16::try_from(channels).unwrap_or(0);
                }
                Err(_) => return 0,
            }
        }
    }

    /// Decode the track into `file` and return the 16 kHz samples written.
    fn decode_into(&mut self, file: File, progress: &dyn Fn(u64, Option<u64>), cancel: &CancelToken) -> Result<u64, MediaError> {
        let mut writer = BufWriter::with_capacity(64 * 1024, file);
        let mut resampler = StreamResampler::new(self.rate, RATE).map_err(MediaError::Corrupt)?;
        let mut written = 0u64;
        let mut bytes = Vec::with_capacity(BLOCK * 2);
        let mut sink = |block: &[f32]| -> Result<(), String> {
            bytes.clear();
            for &s in block {
                // The recording file's conversion (`voltip_core::dictation::long`).
                let v = (s.clamp(-1.0, 1.0) * 32_767.0).round() as i16;
                bytes.extend_from_slice(&v.to_le_bytes());
            }
            written += block.len() as u64;
            writer.write_all(&bytes).map_err(|e| e.to_string())
        };
        let rate = u64::from(self.rate);
        let total_ms = self.info.duration_ms;
        let mut mono: Vec<f32> = Vec::with_capacity(BLOCK * 2);
        let mut buffer: Option<SampleBuffer<f32>> = None;
        let (mut seen, mut kept, mut reported_ms) = (0u64, 0u64, None::<u64>);
        let (mut packets, mut failures) = (0u64, 0u64);
        loop {
            if cancel.is_cancelled() {
                return Err(MediaError::Cancelled);
            }
            let next = match self.pending.take() {
                Some(packet) => Ok(packet),
                None => self.format.next_packet(),
            };
            let packet = match next {
                Ok(packet) => packet,
                // How a reader says the stream ended, and how a cut-off file ends too: the length
                // check below tells them apart.
                Err(Error::IoError(e)) if e.kind() == ErrorKind::UnexpectedEof => break,
                Err(Error::ResetRequired) => {
                    self.reset()?;
                    continue;
                }
                Err(e) => return Err(MediaError::Corrupt(e.to_string())),
            };
            if packet.track_id() != self.track {
                continue;
            }
            packets += 1;
            let (frames, channels): (&[f32], usize) = match &mut self.codec {
                Codec::Opus(opus) => {
                    let channels = opus.channels();
                    match opus.decode(&packet.data, packet.trim_start as usize, packet.trim_end as usize) {
                        Ok(frames) => (frames, channels),
                        Err(e) => {
                            failures += 1;
                            tracing::debug!(error = %e, "an Opus packet did not decode; it is left out");
                            continue;
                        }
                    }
                }
                Codec::Symphonia(decoder) => match decoder.decode(&packet) {
                    Ok(decoded) => {
                        let spec = *decoded.spec();
                        let needed = decoded.capacity() * spec.channels.count();
                        if buffer.as_ref().is_none_or(|b| b.capacity() < needed) {
                            buffer = Some(SampleBuffer::new(decoded.capacity() as u64, spec));
                        }
                        let samples = buffer.as_mut().map(|b| {
                            b.copy_interleaved_ref(decoded);
                            b.samples()
                        });
                        (samples.unwrap_or_default(), spec.channels.count().max(1))
                    }
                    Err(Error::DecodeError(e)) => {
                        failures += 1;
                        tracing::debug!(error = e, "a packet did not decode; it is left out");
                        continue;
                    }
                    Err(Error::IoError(e)) => {
                        failures += 1;
                        tracing::debug!(error = %e, "a packet could not be read; it is left out");
                        continue;
                    }
                    Err(e) => return Err(MediaError::Corrupt(e.to_string())),
                },
            };
            let mut done = false;
            for frame in frames.chunks_exact(channels) {
                seen += 1;
                if seen <= self.skip {
                    continue;
                }
                if self.keep.is_some_and(|keep| kept >= keep) {
                    done = true;
                    break;
                }
                mono.push(frame.iter().sum::<f32>() / channels as f32);
                kept += 1;
            }
            if mono.len() >= BLOCK {
                resampler.push(&mono, &mut sink).map_err(MediaError::Write)?;
                mono.clear();
            }
            let at_ms = kept * 1000 / rate;
            if reported_ms.is_none_or(|r| at_ms >= r + PROGRESS_EVERY_MS) {
                progress(at_ms, total_ms);
                reported_ms = Some(at_ms);
            }
            if done {
                break;
            }
        }
        resampler.push(&mono, &mut sink).map_err(MediaError::Write)?;
        resampler.flush(&mut sink).map_err(MediaError::Write)?;
        writer.flush().map_err(|e| MediaError::Write(e.to_string()))?;
        drop(writer);
        progress(kept * 1000 / rate, total_ms);
        if kept == 0 && failures > 0 {
            return Err(MediaError::Corrupt(format!("none of its {packets} audio packets could be decoded")));
        }
        if let Some(expected) = self.expected.filter(|_| self.exact)
            && kept + (rate / 2).max(expected / 50) < expected
        {
            return Err(MediaError::Corrupt(format!("it ends after {:.1} s of {:.1} s", kept as f64 / rate as f64, expected as f64 / rate as f64)));
        }
        if failures > 0 {
            tracing::warn!(failures, packets, "some packets of the file did not decode and were left out");
        }
        Ok(written)
    }

    /// The reader started a new stream (a chained Ogg file): the same kind of track goes on with a
    /// new decoder.
    fn reset(&mut self) -> Result<(), MediaError> {
        let track = self.format.default_track().filter(|t| is_audio(t)).or_else(|| self.format.tracks().iter().find(|t| is_audio(t))).cloned();
        let Some(track) = track else { return Err(MediaError::Corrupt("the audio stream ended in a new stream without audio".into())) };
        self.track = track.id;
        if let Codec::Symphonia(decoder) = &mut self.codec {
            *decoder =
                symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default()).map_err(|e| MediaError::Corrupt(e.to_string()))?;
        }
        Ok(())
    }
}

/// `units` of the track's time base as frames at `rate`.
fn frames_at(params: &CodecParameters, units: u64, rate: u32) -> u64 {
    match params.time_base {
        Some(base) if base.denom > 0 => (u128::from(units) * u128::from(base.numer) * u128::from(rate) / u128::from(base.denom)) as u64,
        _ => units,
    }
}

fn channels_of(params: &CodecParameters) -> u16 {
    let count = params.channels.map(|c| c.count()).or_else(|| params.channel_layout.map(|l| l.into_channels().count()));
    count.and_then(|c| u16::try_from(c).ok()).unwrap_or(0)
}

/// A codec's name: the short one of those the import decodes, the usual one of the others.
fn codec_name(codec: CodecType) -> &'static str {
    use codecs::*;
    match codec {
        CODEC_TYPE_MP3 => "mp3",
        CODEC_TYPE_MP2 => "mp2",
        CODEC_TYPE_MP1 => "mp1",
        CODEC_TYPE_AAC => "aac",
        CODEC_TYPE_ALAC => "alac",
        CODEC_TYPE_FLAC => "flac",
        CODEC_TYPE_VORBIS => "vorbis",
        CODEC_TYPE_OPUS => "opus",
        CODEC_TYPE_ADPCM_G722 | CODEC_TYPE_ADPCM_G726 | CODEC_TYPE_ADPCM_G726LE | CODEC_TYPE_ADPCM_MS | CODEC_TYPE_ADPCM_IMA_WAV | CODEC_TYPE_ADPCM_IMA_QT => {
            "ADPCM"
        }
        CODEC_TYPE_EAC3 => "E-AC-3",
        CODEC_TYPE_AC4 => "AC-4",
        CODEC_TYPE_DCA => "DTS",
        CODEC_TYPE_WMA => "WMA",
        CODEC_TYPE_SPEEX => "Speex",
        CODEC_TYPE_MUSEPACK => "Musepack",
        CODEC_TYPE_ATRAC1 | CODEC_TYPE_ATRAC3 | CODEC_TYPE_ATRAC3PLUS | CODEC_TYPE_ATRAC9 => "ATRAC",
        CODEC_TYPE_WAVPACK => "WavPack",
        CODEC_TYPE_MONKEYS_AUDIO => "Monkey's Audio",
        CODEC_TYPE_TTA => "TTA",
        CODEC_TYPE_NULL => "unknown",
        // Every PCM variant, A-law and µ-law included, decodes.
        _ => "pcm",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The codec an error names is the one users know it by (docs/dictation.md §31): what the
    /// import decodes by its short name, the others by their usual one.
    #[test]
    fn codecs_are_named_as_users_know_them() {
        use codecs::*;
        let names: Vec<&str> = [
            CODEC_TYPE_MP3,
            CODEC_TYPE_MP2,
            CODEC_TYPE_MP1,
            CODEC_TYPE_AAC,
            CODEC_TYPE_ALAC,
            CODEC_TYPE_FLAC,
            CODEC_TYPE_VORBIS,
            CODEC_TYPE_OPUS,
            CODEC_TYPE_ADPCM_MS,
            CODEC_TYPE_ADPCM_IMA_WAV,
            CODEC_TYPE_EAC3,
            CODEC_TYPE_AC4,
            CODEC_TYPE_DCA,
            CODEC_TYPE_WMA,
            CODEC_TYPE_SPEEX,
            CODEC_TYPE_MUSEPACK,
            CODEC_TYPE_ATRAC3,
            CODEC_TYPE_WAVPACK,
            CODEC_TYPE_MONKEYS_AUDIO,
            CODEC_TYPE_TTA,
            CODEC_TYPE_NULL,
            CODEC_TYPE_PCM_S16LE,
            CODEC_TYPE_PCM_ALAW,
        ]
        .into_iter()
        .map(codec_name)
        .collect();
        assert_eq!(
            names,
            [
                "mp3",
                "mp2",
                "mp1",
                "aac",
                "alac",
                "flac",
                "vorbis",
                "opus",
                "ADPCM",
                "ADPCM",
                "E-AC-3",
                "AC-4",
                "DTS",
                "WMA",
                "Speex",
                "Musepack",
                "ATRAC",
                "WavPack",
                "Monkey's Audio",
                "TTA",
                "unknown",
                "pcm",
                "pcm"
            ]
        );
    }

    /// A track without a time base counts in frames already.
    #[test]
    fn frames_follow_the_time_base_when_there_is_one() {
        let mut params = CodecParameters::new();
        assert_eq!(frames_at(&params, 4_800, 48_000), 4_800);
        params.with_time_base(symphonia::core::units::TimeBase::new(1, 1000));
        assert_eq!(frames_at(&params, 1_500, 16_000), 24_000);
    }
}
