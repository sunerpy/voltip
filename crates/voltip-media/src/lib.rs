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

mod mkv;
mod mp4;
mod opus;
mod sniff;

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

use symphonia::core::codecs::CodecParameters;
use symphonia::core::codecs::audio::well_known::CODEC_ID_OPUS;
use symphonia::core::codecs::audio::{AudioCodecId, AudioCodecParameters, AudioDecoder, AudioDecoderOptions, CODEC_ID_NULL_AUDIO};
use symphonia::core::errors::Error;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, Track, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::packet::Packet;
use symphonia::core::units::TimeBase;
use voltip_core::CancelToken;
use voltip_core::import::{MediaDecoder, MediaError, MediaInfo, PcmInfo};
use voltip_serve::StreamResampler;

/// Samples per second of the output: the recording format.
pub const RATE: u32 = voltip_core::dictation::PCM_SAMPLE_RATE_HZ;
/// Mono samples handed to the resampler at a time.
const BLOCK: usize = 4096;
/// Progress is reported at most this often, in milliseconds of the file.
const PROGRESS_EVERY_MS: u64 = 250;
/// Why a file whose end is missing is damaged, in Symphonia 0.5's words: the import shows it
/// after 文件已损坏或不完整, and it did not change with the readers of 0.6, which say "unexpected
/// end of file" (Ogg) or report a format they do not support (MP4, Matroska).
const END_OF_STREAM: &str = "end of stream";

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
    Symphonia(Box<dyn AudioDecoder>),
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

/// The codec parameters of `track` when it is an audio track. A reader that does not know a
/// track's codec gives it no parameters, or audio ones with the null codec (an audio track in a
/// codec it does not know).
fn audio_params(track: &Track) -> Option<&AudioCodecParameters> {
    track.codec_params.as_ref().and_then(CodecParameters::audio)
}

/// The decoders drop the encoder's delay and padding that the readers mark on the packets (an
/// MP3's encoder header, an Ogg stream's granule positions).
fn decoder_options() -> AudioDecoderOptions {
    AudioDecoderOptions::default().gapless(true)
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
        let format = symphonia::default::get_probe()
            .probe(&hint, MediaSourceStream::new(Box::new(stream), Default::default()), FormatOptions::default(), MetadataOptions::default())
            .map_err(|e| match (e, container) {
                // The MP4 and Matroska readers stop at a box or element that runs past the end of
                // the file and then miss the index or the segment: a file cut off, not one of
                // another format.
                (Error::Unsupported(_), Some("mp4" | "mov")) if mp4::cut_off(&mut file) => MediaError::Corrupt(END_OF_STREAM.into()),
                (Error::Unsupported(_), Some("mkv" | "webm")) if mkv::cut_off(&mut file) => MediaError::Corrupt(END_OF_STREAM.into()),
                (Error::Unsupported(_), _) | (_, None) => MediaError::UnsupportedFormat,
                (Error::IoError(e), Some(_)) if e.kind() == ErrorKind::UnexpectedEof => MediaError::Corrupt(END_OF_STREAM.into()),
                (e, Some(_)) => MediaError::Corrupt(e.to_string()),
            })?;
        let tracks: Vec<&Track> = format.tracks().iter().filter(|t| audio_params(t).is_some()).collect();
        let audio_tracks = u32::try_from(tracks.len()).unwrap_or(u32::MAX);
        let selected = format.default_track(TrackType::Audio).or(tracks.first().copied());
        let Some((track, params)) = selected.and_then(|t| audio_params(t).map(|p| (t.clone(), p.clone()))) else {
            // A track whose codec the reader does not know has no parameters: name it, if one is
            // there.
            return Err(sniff::unsupported_codec(&mut file).map_or(MediaError::NoAudio, |name| MediaError::UnsupportedCodec(name.to_owned())));
        };
        // The track's place in the file: an MP4 track's ID is its `tkhd` one, not its position.
        let position = format.tracks().iter().position(|t| t.id == track.id).unwrap_or_default();
        let length = track_length(&track, format.media_info());
        let mut pre_skip = 0;
        let (codec, rate, channels) = if params.codec == CODEC_ID_NULL_AUDIO {
            let name = sniff::unsupported_codec(&mut file).unwrap_or("unknown");
            return Err(MediaError::UnsupportedCodec(name.to_owned()));
        } else if params.codec == CODEC_ID_OPUS {
            let head = opus::Head::parse(params.extra_data.as_deref().unwrap_or_default())?;
            let opus = opus::Opus::new(head)?;
            // An Ogg stream's granule positions count the pre-skip (RFC 7845 §4).
            pre_skip = if container == Some("ogg") { u64::from(head.pre_skip) } else { 0 };
            (Codec::Opus(Box::new(opus)), opus::RATE, u16::from(head.channels))
        } else {
            let decoder = symphonia::default::get_codecs().make_audio_decoder(&params, &decoder_options()).map_err(|e| match e {
                Error::Unsupported(_) => MediaError::UnsupportedCodec(codec_name(params.codec).to_owned()),
                e => MediaError::Corrupt(e.to_string()),
            })?;
            let rate = params.sample_rate.filter(|&r| r > 0).ok_or_else(|| MediaError::Corrupt("the audio track has no sample rate".into()))?;
            (Codec::Symphonia(decoder), rate, channels_of(&params))
        };
        let mut source =
            Self { format, track: track.id, codec, info: MediaInfo::default(), rate, skip: 0, keep: None, expected: None, exact: false, pending: None };
        // An MP4's AAC track leaves its channels to the codec's own configuration: the first
        // packet says, and is decoded again from the start.
        let channels = if channels == 0 { source.first_channels() } else { channels };
        let container = container.unwrap_or("unknown").to_owned();
        let edit = if matches!(container.as_str(), "mp4" | "mov") { mp4::edit(&mut file, position) } else { None };
        let to_frames = |units: u64| frames_at(track.time_base, units, rate);
        let skip = edit.map_or(0, |e| to_frames(e.skip));
        let keep = edit.and_then(|e| e.keep).map(to_frames);
        let expected = keep.or_else(|| length.map(|(base, units)| frames_at(base, units, rate)).map(|f| f.saturating_sub(skip + pre_skip)));
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
                Ok(Some(packet)) if packet.track_id != self.track => continue,
                Ok(Some(packet)) => {
                    let channels = decoder.decode(&packet).map(|d| d.spec().channels().count()).unwrap_or(0);
                    decoder.reset();
                    self.pending = Some(packet);
                    return u16::try_from(channels).unwrap_or(0);
                }
                Ok(None) | Err(_) => return 0,
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
        let mut buffer: Vec<f32> = Vec::new();
        let (mut seen, mut kept, mut reported_ms) = (0u64, 0u64, None::<u64>);
        let (mut packets, mut failures) = (0u64, 0u64);
        loop {
            if cancel.is_cancelled() {
                return Err(MediaError::Cancelled);
            }
            let next = match self.pending.take() {
                Some(packet) => Ok(Some(packet)),
                None => self.format.next_packet(),
            };
            let packet = match next {
                Ok(Some(packet)) => packet,
                // How a reader says the stream ended, and how a cut-off file may end too: the
                // length check below tells them apart.
                Ok(None) => break,
                Err(Error::IoError(e)) if e.kind() == ErrorKind::UnexpectedEof => break,
                Err(Error::ResetRequired) => {
                    self.reset()?;
                    continue;
                }
                Err(e) => return Err(MediaError::Corrupt(e.to_string())),
            };
            if packet.track_id != self.track {
                continue;
            }
            packets += 1;
            let (frames, channels): (&[f32], usize) = match &mut self.codec {
                Codec::Opus(opus) => {
                    let channels = opus.channels();
                    match opus.decode(&packet.data, frames_of(packet.trim_start), frames_of(packet.trim_end)) {
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
                        decoded.copy_to_vec_interleaved(&mut buffer);
                        (buffer.as_slice(), decoded.spec().channels().count().max(1))
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
        let track = self.format.default_track(TrackType::Audio).or_else(|| self.format.tracks().iter().find(|t| audio_params(t).is_some()));
        let Some((id, params)) = track.and_then(|t| audio_params(t).map(|p| (t.id, p.clone()))) else {
            return Err(MediaError::Corrupt("the audio stream ended in a new stream without audio".into()));
        };
        self.track = id;
        if let Codec::Symphonia(decoder) = &mut self.codec {
            *decoder = symphonia::default::get_codecs().make_audio_decoder(&params, &decoder_options()).map_err(|e| MediaError::Corrupt(e.to_string()))?;
        }
        Ok(())
    }
}

/// How long the track is as the reader says, in units of the time base given with it (`None`:
/// frames): the track's duration, else its frame count, else the whole file's duration (all a
/// Matroska file gives).
fn track_length(track: &Track, media: &symphonia::core::formats::MediaInfo) -> Option<(Option<TimeBase>, u64)> {
    match (track.duration, track.num_frames) {
        (Some(duration), _) => Some((track.time_base, duration.get())),
        (None, Some(frames)) => Some((None, frames)),
        (None, None) => media.duration.map(|duration| (media.time_base, duration.get())),
    }
}

/// `units` of `base` as frames at `rate`; without a time base they are frames already.
fn frames_at(base: Option<TimeBase>, units: u64, rate: u32) -> u64 {
    match base {
        Some(base) => (u128::from(units) * u128::from(base.numer.get()) * u128::from(rate) / u128::from(base.denom.get())) as u64,
        None => units,
    }
}

/// A packet's trim, in frames.
fn frames_of(trim: symphonia::core::units::Duration) -> usize {
    usize::try_from(trim.get()).unwrap_or(usize::MAX)
}

fn channels_of(params: &AudioCodecParameters) -> u16 {
    params.channels.as_ref().map(|c| c.count()).and_then(|c| u16::try_from(c).ok()).unwrap_or(0)
}

/// A codec's name: the short one of those the import decodes, the usual one of the others.
fn codec_name(codec: AudioCodecId) -> &'static str {
    use symphonia::core::codecs::audio::well_known::*;
    match codec {
        CODEC_ID_MP3 => "mp3",
        CODEC_ID_MP2 => "mp2",
        CODEC_ID_MP1 => "mp1",
        CODEC_ID_AAC => "aac",
        CODEC_ID_ALAC => "alac",
        CODEC_ID_FLAC => "flac",
        CODEC_ID_VORBIS => "vorbis",
        CODEC_ID_OPUS => "opus",
        CODEC_ID_ADPCM_G722 | CODEC_ID_ADPCM_G726 | CODEC_ID_ADPCM_G726LE | CODEC_ID_ADPCM_MS | CODEC_ID_ADPCM_IMA_WAV | CODEC_ID_ADPCM_IMA_QT => "ADPCM",
        CODEC_ID_EAC3 => "E-AC-3",
        CODEC_ID_AC4 => "AC-4",
        CODEC_ID_DCA => "DTS",
        CODEC_ID_WMA => "WMA",
        CODEC_ID_SPEEX => "Speex",
        CODEC_ID_MUSEPACK => "Musepack",
        CODEC_ID_ATRAC1 | CODEC_ID_ATRAC3 | CODEC_ID_ATRAC3PLUS | CODEC_ID_ATRAC9 => "ATRAC",
        CODEC_ID_WAVPACK => "WavPack",
        CODEC_ID_MONKEYS_AUDIO => "Monkey's Audio",
        CODEC_ID_TTA => "TTA",
        CODEC_ID_AC3 => "AC-3",
        CODEC_ID_TRUEHD => "TrueHD",
        CODEC_ID_RA10 | CODEC_ID_RA20 | CODEC_ID_COOK | CODEC_ID_SIPR | CODEC_ID_RALF => "RealAudio",
        CODEC_ID_SBC => "SBC",
        CODEC_ID_APTX => "aptX",
        CODEC_ID_APTX_HD => "aptX HD",
        CODEC_ID_LDAC => "LDAC",
        CODEC_ID_BINK_AUDIO => "Bink Audio",
        CODEC_ID_SMACKER_AUDIO => "Smacker audio",
        // Every PCM variant, A-law and µ-law included, decodes.
        CODEC_ID_PCM_S32LE
        | CODEC_ID_PCM_S32LE_PLANAR
        | CODEC_ID_PCM_S32BE
        | CODEC_ID_PCM_S32BE_PLANAR
        | CODEC_ID_PCM_S24LE
        | CODEC_ID_PCM_S24LE_PLANAR
        | CODEC_ID_PCM_S24BE
        | CODEC_ID_PCM_S24BE_PLANAR
        | CODEC_ID_PCM_S16LE
        | CODEC_ID_PCM_S16LE_PLANAR
        | CODEC_ID_PCM_S16BE
        | CODEC_ID_PCM_S16BE_PLANAR
        | CODEC_ID_PCM_S8
        | CODEC_ID_PCM_S8_PLANAR
        | CODEC_ID_PCM_U32LE
        | CODEC_ID_PCM_U32LE_PLANAR
        | CODEC_ID_PCM_U32BE
        | CODEC_ID_PCM_U32BE_PLANAR
        | CODEC_ID_PCM_U24LE
        | CODEC_ID_PCM_U24LE_PLANAR
        | CODEC_ID_PCM_U24BE
        | CODEC_ID_PCM_U24BE_PLANAR
        | CODEC_ID_PCM_U16LE
        | CODEC_ID_PCM_U16LE_PLANAR
        | CODEC_ID_PCM_U16BE
        | CODEC_ID_PCM_U16BE_PLANAR
        | CODEC_ID_PCM_U8
        | CODEC_ID_PCM_U8_PLANAR
        | CODEC_ID_PCM_F32LE
        | CODEC_ID_PCM_F32LE_PLANAR
        | CODEC_ID_PCM_F32BE
        | CODEC_ID_PCM_F32BE_PLANAR
        | CODEC_ID_PCM_F64LE
        | CODEC_ID_PCM_F64LE_PLANAR
        | CODEC_ID_PCM_F64BE
        | CODEC_ID_PCM_F64BE_PLANAR
        | CODEC_ID_PCM_ALAW
        | CODEC_ID_PCM_MULAW => "pcm",
        // The null codec, and one a reader names by an ID of its own.
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The codec an error names is the one users know it by (docs/dictation.md §31): what the
    /// import decodes by its short name, the others by their usual one.
    #[test]
    fn codecs_are_named_as_users_know_them() {
        use symphonia::core::codecs::audio::well_known::*;
        let names: Vec<&str> = [
            CODEC_ID_MP3,
            CODEC_ID_MP2,
            CODEC_ID_MP1,
            CODEC_ID_AAC,
            CODEC_ID_ALAC,
            CODEC_ID_FLAC,
            CODEC_ID_VORBIS,
            CODEC_ID_OPUS,
            CODEC_ID_ADPCM_MS,
            CODEC_ID_ADPCM_IMA_WAV,
            CODEC_ID_EAC3,
            CODEC_ID_AC4,
            CODEC_ID_DCA,
            CODEC_ID_WMA,
            CODEC_ID_SPEEX,
            CODEC_ID_MUSEPACK,
            CODEC_ID_ATRAC3,
            CODEC_ID_WAVPACK,
            CODEC_ID_MONKEYS_AUDIO,
            CODEC_ID_TTA,
            CODEC_ID_NULL_AUDIO,
            CODEC_ID_PCM_S16LE,
            CODEC_ID_PCM_ALAW,
            CODEC_ID_PCM_F32BE_PLANAR,
            CODEC_ID_AC3,
            CODEC_ID_TRUEHD,
            CODEC_ID_COOK,
            CODEC_ID_RA10,
            CODEC_ID_APTX,
            AudioCodecId::new(symphonia::core::common::FourCc::new(*b"xyzw")),
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
                "pcm",
                "pcm",
                "AC-3",
                "TrueHD",
                "RealAudio",
                "RealAudio",
                "aptX",
                "unknown"
            ]
        );
    }

    /// A track without a time base counts in frames already.
    #[test]
    fn frames_follow_the_time_base_when_there_is_one() {
        assert_eq!(frames_at(None, 4_800, 48_000), 4_800);
        assert_eq!(frames_at(TimeBase::try_new(1, 1000), 1_500, 16_000), 24_000);
    }
}
