//! A take longer than one request may carry, cut into pieces that fit (docs/dictation.md §3.15):
//! iFlytek's dictation takes 60 seconds, GLM-ASR 30, MiMo's audio part about three minutes. The
//! rule is the long recording's fallback cutter's (`voltip_core::dictation::long::EnergySegmenter`):
//! once a piece reaches the limit it is cut in the middle of the quietest 200 ms of its last
//! 5 seconds, so a cut falls in a pause when the speaker made one.

use std::ops::Range;

use crate::realtime::BYTES_PER_MS;

/// Bytes of one 200 ms block of 16 kHz 16-bit mono PCM.
const BLOCK: usize = 200 * BYTES_PER_MS;
/// The span before the limit searched for the quietest block.
const SEARCH: usize = 5_000 * BYTES_PER_MS;

/// The byte ranges of `pcm` (16 kHz, 16-bit little-endian mono) as pieces of at most `max_ms`
/// each, in order and covering all of it. A take within the limit is one piece; an empty one has
/// none.
pub fn quiet_cuts(pcm: &[u8], max_ms: u64) -> Vec<Range<usize>> {
    let len = pcm.len() - pcm.len() % 2;
    let max = usize::try_from(max_ms).unwrap_or(usize::MAX).saturating_mul(BYTES_PER_MS).max(2 * BLOCK);
    let mut pieces = Vec::new();
    let mut start = 0;
    while len - start > max {
        let limit = start + max;
        let from = limit.saturating_sub(SEARCH).max(start + BLOCK);
        let quietest = (from..=limit - BLOCK).step_by(BLOCK).min_by(|&a, &b| energy(&pcm[a..a + BLOCK]).total_cmp(&energy(&pcm[b..b + BLOCK])));
        let cut = quietest.map_or(limit, |at| at + BLOCK / 2);
        pieces.push(start..cut);
        start = cut;
    }
    if len > start {
        pieces.push(start..len);
    }
    pieces
}

/// The sum of squares of a block's samples.
fn energy(block: &[u8]) -> f64 {
    block.as_chunks::<2>().0.iter().map(|s| f64::from(i16::from_le_bytes(*s))).map(|x| x * x).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `seconds` of speech-like noise with a 400 ms pause every `pause_every` seconds.
    fn speech(seconds: usize, pause_every: usize) -> Vec<u8> {
        let rate = 16_000;
        (0..seconds * rate)
            .flat_map(|i| {
                let in_pause = pause_every > 0 && i % (pause_every * rate) >= pause_every * rate - rate * 2 / 5;
                let x: i16 = if in_pause { 0 } else { ((i * 7919 % 2000) as i16) - 1000 };
                x.to_le_bytes()
            })
            .collect()
    }

    #[test]
    fn a_take_within_the_limit_is_one_piece() {
        let pcm = speech(20, 0);
        assert_eq!(quiet_cuts(&pcm, 28_000), vec![0..pcm.len()]);
        assert!(quiet_cuts(&[], 28_000).is_empty());
    }

    #[test]
    fn a_long_take_is_cut_in_its_pauses_within_the_limit() {
        // A pause every 13 s: the cuts land in them, every piece at most 28 s.
        let pcm = speech(90, 13);
        let pieces = quiet_cuts(&pcm, 28_000);
        assert_eq!(pieces.first().map(|p| p.start), Some(0));
        assert_eq!(pieces.last().map(|p| p.end), Some(pcm.len()));
        for pair in pieces.windows(2) {
            assert_eq!(pair[0].end, pair[1].start, "the pieces cover the take");
        }
        for piece in &pieces {
            assert!(piece.len() <= 28_000 * BYTES_PER_MS, "{piece:?}");
            assert_eq!(piece.start % 2, 0, "{piece:?} starts on a sample");
        }
        for piece in &pieces[..pieces.len() - 1] {
            let at = piece.end / BYTES_PER_MS;
            let second = at % 13_000;
            assert!(second >= 12_600 || second == 0, "cut at {at} ms is not in a pause");
        }
    }

    #[test]
    fn a_take_without_pauses_is_cut_by_the_limit_still() {
        let pcm = speech(70, 0);
        let pieces = quiet_cuts(&pcm, 30_000);
        assert_eq!(pieces.len(), 3);
        assert!(pieces.iter().all(|p| p.len() <= 30_000 * BYTES_PER_MS && p.len() >= 25_000 * BYTES_PER_MS - BLOCK || p.end == pcm.len()));
    }
}
