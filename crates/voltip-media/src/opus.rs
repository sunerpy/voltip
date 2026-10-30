//! Opus packets (an `.opus` or `.ogg` file's, a WebM or Matroska `A_OPUS` track's) decoded with
//! `opus-rs`: Symphonia 0.5 reads these containers but has no Opus decoder. The stream's
//! `OpusHead` (the codec's extra data) gives the channels and the pre-skip, the encoder's start-up
//! samples that are not part of the sound (RFC 7845 §4.2).

use opus_rs::OpusDecoder;
use voltip_core::import::MediaError;

/// The rate Opus always decodes at here: every Opus stream is 48 kHz inside.
pub const RATE: u32 = 48_000;
/// The most samples per channel one packet can carry (120 ms at 48 kHz).
const MAX_FRAME: usize = 5_760;

/// What a stream's `OpusHead` says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Head {
    /// Output channels.
    pub channels: u8,
    /// Samples at 48 kHz to drop at the start.
    pub pre_skip: u16,
}

impl Head {
    /// Read `OpusHead` (RFC 7845 §5.1). Channel mapping family 0 is one or two channels in one
    /// stream; the others (surround, ambisonics) pack several streams into a packet, which the
    /// decoder does not read.
    pub fn parse(extra: &[u8]) -> Result<Self, MediaError> {
        if extra.len() < 19 || &extra[..8] != b"OpusHead" {
            return Err(MediaError::Corrupt("the Opus stream has no OpusHead".into()));
        }
        let channels = extra[9];
        let pre_skip = u16::from_le_bytes([extra[10], extra[11]]);
        let family = extra[18];
        if channels == 0 {
            return Err(MediaError::Corrupt("the Opus stream has no channels".into()));
        }
        if family != 0 || channels > 2 {
            return Err(MediaError::UnsupportedCodec(format!("Opus ({channels} channels)")));
        }
        Ok(Self { channels, pre_skip })
    }
}

/// One stream's decoder: interleaved `f32` frames at [`RATE`], the pre-skip dropped.
pub struct Opus {
    decoder: OpusDecoder,
    channels: usize,
    /// Pre-skip samples (per channel) still to drop.
    skip: usize,
    frame: Vec<f32>,
}

impl Opus {
    /// A decoder for `head`'s stream; it drops the pre-skip itself (Symphonia 0.5's readers leave
    /// it in, the Ogg one too), unless the container trims the first packet ([`Opus::decode`]).
    pub fn new(head: Head) -> Result<Self, MediaError> {
        let channels = usize::from(head.channels);
        let decoder = OpusDecoder::new(RATE as i32, channels).map_err(|e| MediaError::Corrupt(format!("Opus: {e}")))?;
        Ok(Self { decoder, channels, skip: usize::from(head.pre_skip), frame: vec![0.0; MAX_FRAME * channels] })
    }

    /// The channels of what [`Opus::decode`] returns.
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Decode `packet`, then drop `trim_start` frames from its start and `trim_end` from its end
    /// (the container's own trims) and what is left of the pre-skip; a container that trims the
    /// start has taken the pre-skip off itself. Returns the interleaved frames left; a packet that
    /// does not decode is an error for the caller to pass over.
    pub fn decode(&mut self, packet: &[u8], trim_start: usize, trim_end: usize) -> Result<&[f32], String> {
        let frames = self.decoder.decode(packet, MAX_FRAME, &mut self.frame).map_err(str::to_owned)?;
        let frames = frames.min(MAX_FRAME);
        if trim_start > 0 {
            self.skip = 0;
        }
        let start = trim_start.min(frames);
        let end = frames.saturating_sub(trim_end).max(start);
        let skip = self.skip.min(end - start);
        self.skip -= skip;
        Ok(&self.frame[(start + skip) * self.channels..end * self.channels])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head(channels: u8, pre_skip: u16, family: u8) -> Vec<u8> {
        let mut h = b"OpusHead".to_vec();
        h.push(1);
        h.push(channels);
        h.extend_from_slice(&pre_skip.to_le_bytes());
        h.extend_from_slice(&48_000u32.to_le_bytes());
        h.extend_from_slice(&0i16.to_le_bytes());
        h.push(family);
        h
    }

    #[test]
    fn the_head_gives_the_channels_and_the_pre_skip() {
        assert_eq!(Head::parse(&head(1, 312, 0)).unwrap(), Head { channels: 1, pre_skip: 312 });
        assert_eq!(Head::parse(&head(2, 3840, 0)).unwrap(), Head { channels: 2, pre_skip: 3840 });
        assert!(matches!(Head::parse(&head(6, 312, 1)), Err(MediaError::UnsupportedCodec(name)) if name == "Opus (6 channels)"));
        assert!(matches!(Head::parse(&head(0, 312, 0)), Err(MediaError::Corrupt(_))));
        assert!(matches!(Head::parse(b"OpusTags"), Err(MediaError::Corrupt(_))));
    }

    /// Packets of 20 ms: the first ones lose the pre-skip, and a container's trims come first.
    #[test]
    fn the_pre_skip_is_dropped_once_across_packets() {
        let mut encoder = opus_rs::OpusEncoder::new(RATE as i32, 1, opus_rs::Application::Audio).unwrap();
        let tone: Vec<f32> = (0..960).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
        let mut packet = vec![0u8; 1500];
        let n = encoder.encode(&tone, 960, &mut packet).unwrap();
        let packet = &packet[..n];

        let mut opus = Opus::new(Head { channels: 1, pre_skip: 1_000 }).unwrap();
        assert_eq!(opus.decode(packet, 0, 0).unwrap().len(), 0, "960 of the 1000 dropped");
        assert_eq!(opus.decode(packet, 0, 0).unwrap().len(), 920, "the last 40");
        assert_eq!(opus.decode(packet, 0, 0).unwrap().len(), 960, "then whole packets");
        assert_eq!(opus.decode(packet, 0, 60).unwrap().len(), 900, "the container's end trim");

        let mut trimmed = Opus::new(Head { channels: 1, pre_skip: 312 }).unwrap();
        assert_eq!(trimmed.decode(packet, 312, 0).unwrap().len(), 648, "a container that trims the start took the pre-skip off");
        assert_eq!(trimmed.decode(packet, 0, 0).unwrap().len(), 960, "and nothing more is dropped");
        assert!(trimmed.decode(&[], 0, 0).is_err(), "an empty packet does not decode");
    }
}
