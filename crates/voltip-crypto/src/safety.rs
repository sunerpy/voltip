//! Safety codes: the human-comparable rendering of a Noise handshake hash.
//!
//! Four BIP-39 English words (11 bits each, 44 bits) plus a 64-bit hex fingerprint. Both sides
//! of a handshake compute the same value; a man in the middle cannot make them agree without
//! breaking the handshake (`docs/threat-model.md`).

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::PublicKey;

/// Number of words in a safety code.
pub const WORD_COUNT: usize = 4;
/// Bits consumed per word (`2^11 = 2048` words).
const BITS_PER_WORD: usize = 11;
/// Bytes of the hash rendered as the hex fingerprint.
const FINGERPRINT_BYTES: usize = 8;

/// The BIP-39 English wordlist (2048 words, public domain / BSD-2 from the BIP repository).
static WORDLIST: &str = include_str!("bip39-english.txt");

fn words() -> Vec<&'static str> {
    WORDLIST.lines().collect()
}

/// A safety code as shown on both devices.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct SafetyCode {
    /// Four lower-case words.
    pub words: [String; WORD_COUNT],
    /// `A7:C4:19:8E · 3D:F2:61:09`.
    pub fingerprint: String,
}

impl SafetyCode {
    /// Derive from a 32-byte handshake hash.
    pub fn from_handshake_hash(hash: &[u8; 32]) -> Self {
        let list = words();
        let mut words: Vec<String> = Vec::with_capacity(WORD_COUNT);
        // Walk the hash as a bit stream, 11 bits per word.
        let mut bit_cursor = 0usize;
        for _ in 0..WORD_COUNT {
            let mut idx: usize = 0;
            for _ in 0..BITS_PER_WORD {
                let byte = hash[bit_cursor / 8];
                let bit = (byte >> (7 - (bit_cursor % 8))) & 1;
                idx = (idx << 1) | usize::from(bit);
                bit_cursor += 1;
            }
            words.push(list[idx].to_owned());
        }
        let words: [String; WORD_COUNT] = words.try_into().unwrap_or_else(|_| unreachable!("exactly WORD_COUNT words were pushed"));
        Self { words, fingerprint: fingerprint_hex(&hash[..FINGERPRINT_BYTES]) }
    }

    /// `Tiger · Apple · River · Moon` — capitalised, dot-separated, for the verify screen.
    pub fn display_words(&self) -> String {
        self.words.iter().map(|w| capitalise(w)).collect::<Vec<_>>().join(" · ")
    }
}

impl std::fmt::Display for SafetyCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.display_words(), self.fingerprint)
    }
}

fn capitalise(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Render bytes as upper-case hex pairs, colon-joined, with a dot every four pairs:
/// `A7:C4:19:8E · 3D:F2:61:09`.
pub fn fingerprint_hex(bytes: &[u8]) -> String {
    bytes.chunks(4).map(|group| group.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":")).collect::<Vec<_>>().join(" · ")
}

impl PublicKey {
    /// Long-lived fingerprint of a device identity: `SHA-256(public key)`, first 8 bytes,
    /// rendered with [`fingerprint_hex`]. Shown in the device list.
    pub fn fingerprint(&self) -> String {
        let digest = Sha256::digest(self.as_bytes());
        fingerprint_hex(&digest[..FINGERPRINT_BYTES])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn wordlist_is_the_bip39_english_list() {
        let list = words();
        assert_eq!(list.len(), 2048);
        assert_eq!(list[0], "abandon");
        assert_eq!(list[2047], "zoo");
        let mut sorted = list.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, list, "list must be sorted so indices are canonical");
    }

    #[test]
    fn zero_hash_maps_to_first_word_and_ones_to_last() {
        let code = SafetyCode::from_handshake_hash(&[0u8; 32]);
        assert_eq!(code.words, ["abandon", "abandon", "abandon", "abandon"].map(String::from));
        assert_eq!(code.fingerprint, "00:00:00:00 · 00:00:00:00");
        let code = SafetyCode::from_handshake_hash(&[0xff; 32]);
        assert_eq!(code.words, ["zoo", "zoo", "zoo", "zoo"].map(String::from));
        assert_eq!(code.fingerprint, "FF:FF:FF:FF · FF:FF:FF:FF");
    }

    #[test]
    fn known_vector() {
        // 0xA7C4198E3DF26109… : first 11 bits of 0xA7C4 = 1010_0111_110 = 1342 -> word[1342].
        let mut h = [0u8; 32];
        h[..8].copy_from_slice(&[0xA7, 0xC4, 0x19, 0x8E, 0x3D, 0xF2, 0x61, 0x09]);
        let code = SafetyCode::from_handshake_hash(&h);
        assert_eq!(code.fingerprint, "A7:C4:19:8E · 3D:F2:61:09");
        assert_eq!(code.words[0], words()[1342]);
        assert_eq!(code.display_words().split(" · ").count(), 4);
        assert!(code.display_words().chars().next().unwrap().is_uppercase());
        assert!(code.to_string().ends_with("(A7:C4:19:8E · 3D:F2:61:09)"));
    }

    #[test]
    fn public_key_fingerprint_is_stable_and_formatted() {
        let pk = PublicKey([3u8; 32]);
        let fp = pk.fingerprint();
        assert_eq!(fp, pk.fingerprint());
        assert_eq!(fp.len(), "A7:C4:19:8E · 3D:F2:61:09".len());
        assert_ne!(fp, PublicKey([4u8; 32]).fingerprint());
    }

    #[test]
    fn capitalise_handles_empty() {
        assert_eq!(capitalise(""), "");
        assert_eq!(capitalise("moon"), "Moon");
        assert_eq!(fingerprint_hex(&[]), "");
        assert_eq!(fingerprint_hex(&[1, 2]), "01:02");
    }

    proptest! {
        #[test]
        fn every_hash_yields_four_valid_words(hash in proptest::array::uniform32(any::<u8>())) {
            let code = SafetyCode::from_handshake_hash(&hash);
            let list = words();
            for w in &code.words {
                prop_assert!(list.binary_search(&w.as_str()).is_ok());
            }
            prop_assert_eq!(code.fingerprint.chars().count(), 25);
            // Serialization roundtrip.
            let json = serde_json::to_string(&code).unwrap();
            let back: SafetyCode = serde_json::from_str(&json).unwrap();
            prop_assert_eq!(back, code);
        }
    }
}
