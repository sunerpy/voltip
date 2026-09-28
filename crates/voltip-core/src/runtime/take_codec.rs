//! Opus for a phone take (docs/dictation.md §20.1): the phone encodes its 16 kHz tap into 20 ms
//! packets once the desktop said it decodes them, the desktop decodes them back into the PCM16 its
//! [`crate::dictation::remote::RemoteFeed`] takes. `opus-rs` (a pure-Rust port of libopus 1.6)
//! on both ends, so the phone needs no C toolchain.
//!
//! VoIP application at 24 kbit/s VBR: about a tenth of the PCM stream (256 kbit/s), which
//! recognition does not notice on speech.

use opus_rs::{Application, OpusDecoder, OpusEncoder};
use serde_bytes::ByteBuf;
use voltip_protocol::app::{MAX_OPUS_PACKET_BYTES, TAKE_OPUS_FRAME_SAMPLES, TAKE_SAMPLE_RATE_HZ};

/// Target bitrate of a take's Opus stream.
pub(super) const TAKE_OPUS_BITRATE_BPS: i32 = 24_000;

/// Phone: PCM16 LE in, one Opus packet per complete 20 ms frame out.
pub(super) struct TakeEncoder {
    encoder: OpusEncoder,
    /// Samples of a frame not yet complete.
    pending: Vec<i16>,
    packet: Vec<u8>,
}

impl TakeEncoder {
    pub(super) fn new() -> Result<Self, String> {
        let mut encoder = OpusEncoder::new(TAKE_SAMPLE_RATE_HZ as i32, 1, Application::Voip).map_err(str::to_owned)?;
        encoder.bitrate_bps = TAKE_OPUS_BITRATE_BPS;
        encoder.use_cbr = false;
        Ok(Self { encoder, pending: Vec::with_capacity(TAKE_OPUS_FRAME_SAMPLES), packet: vec![0; MAX_OPUS_PACKET_BYTES] })
    }

    /// Append PCM16 little-endian mono samples; the packets of every frame they complete.
    pub(super) fn push(&mut self, pcm: &[u8]) -> Result<Vec<ByteBuf>, String> {
        let mut packets = Vec::new();
        for sample in pcm.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)) {
            self.pending.push(sample);
            if self.pending.len() == TAKE_OPUS_FRAME_SAMPLES {
                packets.push(self.encode_pending()?);
            }
        }
        Ok(packets)
    }

    /// The last, partial frame padded with silence (nothing when no sample is pending).
    pub(super) fn finish(&mut self) -> Result<Vec<ByteBuf>, String> {
        if self.pending.is_empty() {
            return Ok(Vec::new());
        }
        self.pending.resize(TAKE_OPUS_FRAME_SAMPLES, 0);
        Ok(vec![self.encode_pending()?])
    }

    fn encode_pending(&mut self) -> Result<ByteBuf, String> {
        let n = self.encoder.encode_i16(&self.pending, TAKE_OPUS_FRAME_SAMPLES, &mut self.packet).map_err(str::to_owned)?;
        self.pending.clear();
        Ok(ByteBuf::from(self.packet[..n].to_vec()))
    }
}

/// Desktop: Opus packets in, PCM16 LE out.
pub(super) struct TakeDecoder {
    decoder: OpusDecoder,
    frame: Vec<f32>,
}

impl TakeDecoder {
    pub(super) fn new() -> Result<Self, String> {
        let decoder = OpusDecoder::new(TAKE_SAMPLE_RATE_HZ as i32, 1).map_err(str::to_owned)?;
        Ok(Self { decoder, frame: vec![0.0; TAKE_OPUS_FRAME_SAMPLES] })
    }

    /// Decode `packets` in order into PCM16 little-endian. A packet that does not decode is left
    /// out (and logged); the take goes on with the next one, as with a lost chunk.
    pub(super) fn decode(&mut self, packets: &[ByteBuf]) -> Vec<u8> {
        let mut pcm = Vec::with_capacity(packets.len() * TAKE_OPUS_FRAME_SAMPLES * 2);
        for packet in packets {
            match self.decoder.decode(packet, TAKE_OPUS_FRAME_SAMPLES, &mut self.frame) {
                Ok(n) => {
                    for &s in &self.frame[..n.min(self.frame.len())] {
                        // Full scale is ±1.0; the cast saturates.
                        let v = (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
                        pcm.extend_from_slice(&v.to_le_bytes());
                    }
                }
                Err(e) => tracing::debug!(error = e, bytes = packet.len(), "phone take: an Opus packet did not decode"),
            }
        }
        pcm
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One second of a 440 Hz tone at half scale, PCM16 LE.
    fn tone(samples: usize) -> Vec<u8> {
        (0..samples)
            .flat_map(|i| {
                let t = i as f32 / TAKE_SAMPLE_RATE_HZ as f32;
                (((t * 440.0 * std::f32::consts::TAU).sin() * 0.5 * f32::from(i16::MAX)) as i16).to_le_bytes()
            })
            .collect()
    }

    fn energy(pcm: &[u8]) -> f64 {
        pcm.as_chunks::<2>().0.iter().map(|b| f64::from(i16::from_le_bytes(*b)).powi(2)).sum::<f64>() / (pcm.len() / 2) as f64
    }

    #[test]
    fn a_second_of_audio_is_fifty_small_packets_that_decode_back_to_the_same_length() {
        let pcm = tone(16_000);
        let mut encoder = TakeEncoder::new().unwrap();
        // Uneven pieces, as the pump reads them: frames straddle the pieces.
        let mut packets = Vec::new();
        for piece in pcm.chunks(2 * 1_234) {
            packets.extend(encoder.push(piece).unwrap());
        }
        assert_eq!(packets.len(), 50);
        assert!(encoder.finish().unwrap().is_empty(), "16 000 samples are exactly 50 frames");
        let bytes: usize = packets.iter().map(|p| p.len()).sum();
        assert!(bytes < pcm.len() / 5, "Opus at 24 kbit/s is far smaller than PCM: {bytes} bytes");
        assert!(packets.iter().all(|p| !p.is_empty() && p.len() <= MAX_OPUS_PACKET_BYTES));
        let mut decoder = TakeDecoder::new().unwrap();
        let decoded = decoder.decode(&packets);
        assert_eq!(decoded.len(), pcm.len());
        // Lossy, but the tone survives: the decoded second (past the codec's start-up) carries
        // most of the original energy.
        let (orig, back) = (energy(&pcm[3_200..]), energy(&decoded[3_200..]));
        assert!(back > orig * 0.5 && back < orig * 1.5, "energy {back} vs {orig}");
    }

    #[test]
    fn the_last_partial_frame_is_padded_and_a_bad_packet_is_skipped() {
        let mut encoder = TakeEncoder::new().unwrap();
        assert!(encoder.push(&tone(100)).unwrap().is_empty(), "no frame complete yet");
        let tail = encoder.finish().unwrap();
        assert_eq!(tail.len(), 1);
        assert!(encoder.finish().unwrap().is_empty(), "nothing pending after the flush");
        let mut decoder = TakeDecoder::new().unwrap();
        assert_eq!(decoder.decode(&tail).len(), TAKE_OPUS_FRAME_SAMPLES * 2);
        // A packet whose table of contents asks for two channels does not decode on a mono decoder.
        let stereo = ByteBuf::from(vec![0x04, 0, 0]);
        assert_eq!(decoder.decode(&[stereo]).len(), 0);
    }
}
