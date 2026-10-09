//! What the parts of a body carry once they are put back together (docs/dictation.md §20.8):
//! the computer's settings and history for a phone, and a phone's own records for the computer.
//! The types live in the core (the protocol crate only moves bytes), so their encoding and limits
//! are checked here.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{MAX_BATCH_DELETES, MAX_BATCH_UPSERTS, MAX_BULK_BYTES, MAX_UPLOAD_RECORDS};
use crate::history::{ChangeBatch, HistoryEntry, SHORTENED_FIELD_CHARS};
use crate::presets::{CustomPreset, PresetId};
use crate::providers::ProviderId;
use crate::scenes::Scene;
use crate::settings::{Locale, Settings, ThemeId};
use crate::vocabulary::{DictionaryEntry, ReplacementRule};
use crate::{CoreError, EngineStatus};

/// The computer's settings as its phones show them, read-only (docs/dictation.md §20.8). Only
/// lists and structures, no maps, so the same settings always encode to the same bytes and their
/// SHA-256 names them ([`Profile::tag`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    /// The computer's interface language setting (shown; the phone keeps its own).
    pub locale: Locale,
    /// The computer's theme (shown; the phone keeps its own).
    pub theme: ThemeId,
    /// The computer follows the system's light or dark mode.
    pub follow_system_theme: bool,
    /// The recognition provider in effect.
    pub asr_provider: ProviderId,
    /// The recognition model, as the computer's title bar names it (at most 256 characters).
    pub asr_model: String,
    /// The local model's catalogue id, on-device recognition only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_model: Option<String>,
    /// Takes are cleaned up.
    pub refine_enabled: bool,
    /// The clean-up provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_provider: Option<ProviderId>,
    /// The clean-up model (at most 256 characters).
    pub refine_model: String,
    /// The preset the clean-up runs with.
    pub preset: PresetId,
    /// Custom presets.
    pub presets: Vec<CustomPreset>,
    /// The personal dictionary.
    pub dictionary: Vec<DictionaryEntry>,
    /// Replacement rules in execution order.
    pub rules: Vec<ReplacementRule>,
    /// Scenes in matching order.
    pub scenes: Vec<Scene>,
}

impl Profile {
    /// The computer's settings now.
    pub fn new(
        settings: &Settings,
        engines: &EngineStatus,
        presets: &[CustomPreset],
        dictionary: &[DictionaryEntry],
        rules: &[ReplacementRule],
        scenes: &[Scene],
    ) -> Self {
        let name = |s: &str| s.chars().take(SHORTENED_FIELD_CHARS).collect::<String>();
        Self {
            locale: settings.locale,
            theme: settings.theme,
            follow_system_theme: settings.follow_system_theme,
            asr_provider: engines.asr_provider,
            asr_model: name(&engines.asr_model),
            local_model: engines.local_model.as_deref().map(name),
            refine_enabled: engines.refine_enabled,
            llm_provider: engines.llm_provider,
            refine_model: name(&engines.refine_model),
            preset: settings.engines.refine_preset,
            presets: presets.to_vec(),
            dictionary: dictionary.to_vec(),
            rules: rules.to_vec(),
            scenes: scenes.to_vec(),
        }
    }

    /// The SHA-256 of the encoding: two profiles with the same tag are the same.
    pub fn tag(&self) -> [u8; 32] {
        let mut bytes = Vec::new();
        if ciborium::into_writer(self, &mut bytes).is_err() {
            return [0; 32];
        }
        Sha256::digest(&bytes).into()
    }

    /// Without the four lists: what is sent when the whole would not fit in a body (it cannot
    /// with the lists' limits, see the test).
    pub fn without_lists(&self) -> Self {
        Self { presets: Vec::new(), dictionary: Vec::new(), rules: Vec::new(), scenes: Vec::new(), ..self.clone() }
    }
}

/// A body put back together from parts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BulkBody {
    /// Computer → phone: its settings, for request `req`.
    MirrorProfile {
        /// The phone's request.
        req: u32,
        /// The phone's sync switch generation on the computer.
        generation: u32,
        /// [`Profile::tag`].
        tag: serde_bytes::ByteBuf,
        /// The settings.
        profile: Box<Profile>,
    },
    /// Computer → phone: a batch of history changes, for request `req`.
    MirrorBatch {
        /// The phone's request.
        req: u32,
        /// The phone's sync switch generation on the computer.
        generation: u32,
        /// The computer's history.
        epoch: Uuid,
        /// Replace the phone's copy.
        reset: bool,
        /// The newest change.
        head: u64,
        /// The change this batch reaches.
        to: u64,
        /// Entries written or changed.
        upserts: Vec<HistoryEntry>,
        /// Entries gone.
        deletes: Vec<Uuid>,
        /// Entries of `upserts` sent shortened.
        shortened: Vec<Uuid>,
        /// More follow.
        more: bool,
    },
    /// Phone → computer: records the phone recognised on its own.
    PhoneRecords {
        /// The records, `segments` left out.
        records: Vec<HistoryEntry>,
    },
}

impl BulkBody {
    /// A batch of history changes answering request `req`.
    pub fn batch(req: u32, generation: u32, batch: ChangeBatch) -> Self {
        Self::MirrorBatch {
            req,
            generation,
            epoch: batch.epoch,
            reset: batch.reset,
            head: batch.head,
            to: batch.to,
            upserts: batch.upserts,
            deletes: batch.deletes,
            shortened: batch.shortened,
            more: batch.more,
        }
    }

    /// CBOR, at most [`MAX_BULK_BYTES`].
    pub fn encode(&self) -> Result<Vec<u8>, CoreError> {
        let mut out = Vec::new();
        ciborium::into_writer(self, &mut out).map_err(|e| CoreError::Invalid(format!("sync body: {e}")))?;
        if out.len() > MAX_BULK_BYTES {
            return Err(CoreError::Invalid(format!("sync body of {} bytes exceeds {MAX_BULK_BYTES}", out.len())));
        }
        Ok(out)
    }

    /// Read a body and check its limits: the size, at most [`MAX_BATCH_UPSERTS`] entries,
    /// [`MAX_BATCH_DELETES`] deletions and [`MAX_UPLOAD_RECORDS`] records, a 32-byte tag, and
    /// `shortened` naming entries of `upserts` only.
    pub fn decode(bytes: &[u8]) -> Result<Self, CoreError> {
        if bytes.len() > MAX_BULK_BYTES {
            return Err(CoreError::Invalid(format!("sync body of {} bytes exceeds {MAX_BULK_BYTES}", bytes.len())));
        }
        let body: Self = ciborium::from_reader(bytes).map_err(|e| CoreError::Invalid(format!("sync body: {e}")))?;
        match &body {
            Self::MirrorProfile { tag, .. } if tag.len() != 32 => return Err(CoreError::Invalid("sync body: settings tag".into())),
            Self::MirrorBatch { upserts, deletes, shortened, .. } => {
                if upserts.len() > MAX_BATCH_UPSERTS || deletes.len() > MAX_BATCH_DELETES {
                    return Err(CoreError::Invalid(format!("sync body: {} entries and {} deletions", upserts.len(), deletes.len())));
                }
                if !shortened.iter().all(|id| upserts.iter().any(|e| e.id == *id)) {
                    return Err(CoreError::Invalid("sync body: a shortened entry is not in the batch".into()));
                }
            }
            Self::PhoneRecords { records } if records.len() > MAX_UPLOAD_RECORDS => {
                return Err(CoreError::Invalid(format!("sync body: {} records", records.len())));
            }
            _ => {}
        }
        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::{MAX_PRESET_NAME_CHARS, MAX_PRESET_PROMPT_CHARS, MAX_PRESETS};
    use crate::scenes::{
        MAX_APP_ID_CHARS, MAX_SCENE_APPS, MAX_SCENE_NAME_CHARS, MAX_SCENE_PROMPT_CHARS, MAX_SCENES, MAX_TITLE_KEYWORD_CHARS, MAX_TITLE_KEYWORDS, SceneMatch,
        SceneOverrides,
    };
    use crate::vocabulary::{
        EntrySource, MAX_DICTIONARY_ENTRIES, MAX_HEARD_AS, MAX_PATTERN_CHARS, MAX_REPLACEMENT_CHARS, MAX_RULE_NAME_CHARS, MAX_RULES, MAX_TERM_CHARS, RuleKind,
    };

    /// A character that takes four bytes in UTF-8.
    const WIDE: char = '𠀀';

    fn wide(n: usize) -> String {
        std::iter::repeat_n(WIDE, n).collect()
    }

    fn entry(text: &str) -> HistoryEntry {
        serde_json::from_value(serde_json::json!({
            "id": Uuid::new_v4(), "at_ms": 1, "raw_text": text, "text": text, "refined": false, "asr_model": "m",
            "duration_ms": 1, "asr_ms": 1, "outcome": { "kind": "inserted", "via": "paste" }
        }))
        .unwrap()
    }

    /// Every list at its limit, every string at its longest, every character four bytes.
    fn largest_profile() -> Profile {
        Profile {
            locale: Locale::System,
            theme: ThemeId::Graphite,
            follow_system_theme: true,
            asr_provider: ProviderId::Builtin,
            asr_model: wide(SHORTENED_FIELD_CHARS),
            local_model: Some(wide(SHORTENED_FIELD_CHARS)),
            refine_enabled: true,
            llm_provider: Some(ProviderId::Builtin),
            refine_model: wide(SHORTENED_FIELD_CHARS),
            preset: PresetId::default(),
            presets: (0..MAX_PRESETS)
                .map(|_| CustomPreset {
                    id: Uuid::new_v4(),
                    name: wide(MAX_PRESET_NAME_CHARS),
                    prompt: wide(MAX_PRESET_PROMPT_CHARS),
                    created_at_ms: u64::MAX,
                    updated_at_ms: u64::MAX,
                })
                .collect(),
            dictionary: (0..MAX_DICTIONARY_ENTRIES)
                .map(|_| DictionaryEntry {
                    id: Uuid::new_v4(),
                    term: wide(MAX_TERM_CHARS),
                    heard_as: vec![wide(MAX_TERM_CHARS); MAX_HEARD_AS],
                    enabled: true,
                    source: EntrySource::Manual,
                    created_at_ms: u64::MAX,
                    updated_at_ms: u64::MAX,
                })
                .collect(),
            rules: (0..MAX_RULES)
                .map(|_| ReplacementRule {
                    id: Uuid::new_v4(),
                    name: wide(MAX_RULE_NAME_CHARS),
                    kind: RuleKind::Regex,
                    pattern: wide(MAX_PATTERN_CHARS),
                    replacement: wide(MAX_REPLACEMENT_CHARS),
                    case_sensitive: true,
                    enabled: true,
                    created_at_ms: u64::MAX,
                    updated_at_ms: u64::MAX,
                })
                .collect(),
            scenes: (0..MAX_SCENES)
                .map(|_| Scene {
                    id: Uuid::new_v4(),
                    name: wide(MAX_SCENE_NAME_CHARS),
                    enabled: true,
                    matching: SceneMatch {
                        apps: vec![wide(MAX_APP_ID_CHARS); MAX_SCENE_APPS],
                        title_contains: vec![wide(MAX_TITLE_KEYWORD_CHARS); MAX_TITLE_KEYWORDS],
                    },
                    overrides: SceneOverrides { prompt: Some(wide(MAX_SCENE_PROMPT_CHARS)), language: Some("zh".into()), ..SceneOverrides::default() },
                    created_at_ms: u64::MAX,
                    updated_at_ms: u64::MAX,
                    builtin: None,
                })
                .collect(),
        }
    }

    #[test]
    fn the_largest_settings_fit_a_body_with_room_to_spare() {
        let profile = largest_profile();
        let body = BulkBody::MirrorProfile { req: 1, generation: 0, tag: serde_bytes::ByteBuf::from(profile.tag().to_vec()), profile: Box::new(profile) };
        let bytes = body.encode().unwrap();
        assert!(bytes.len() < 4 * 1024 * 1024, "{} bytes", bytes.len());
        assert_eq!(BulkBody::decode(&bytes).unwrap(), body);
    }

    #[test]
    fn the_tag_names_the_content() {
        let a = largest_profile();
        let mut b = a.clone();
        assert_eq!(a.tag(), b.tag());
        b.refine_enabled = false;
        assert_ne!(a.tag(), b.tag());
        let bare = a.without_lists();
        assert!(bare.presets.is_empty() && bare.dictionary.is_empty() && bare.rules.is_empty() && bare.scenes.is_empty());
        assert_eq!(bare.asr_model, a.asr_model);
    }

    #[test]
    fn bodies_round_trip_and_their_limits_are_checked() {
        let e = entry("说的话");
        for body in [
            BulkBody::MirrorBatch {
                req: 3,
                generation: 1,
                epoch: Uuid::new_v4(),
                reset: true,
                head: 9,
                to: 9,
                upserts: vec![e.clone()],
                deletes: vec![Uuid::new_v4()],
                shortened: vec![e.id],
                more: false,
            },
            BulkBody::PhoneRecords { records: vec![e.clone(); 3] },
        ] {
            assert_eq!(BulkBody::decode(&body.encode().unwrap()).unwrap(), body);
        }
        let stray = BulkBody::MirrorBatch {
            req: 1,
            generation: 0,
            epoch: Uuid::nil(),
            reset: false,
            head: 1,
            to: 1,
            upserts: vec![e.clone()],
            deletes: Vec::new(),
            shortened: vec![Uuid::new_v4()],
            more: false,
        };
        assert!(BulkBody::decode(&stray.encode().unwrap()).is_err(), "shortened must name entries of the batch");
        let many = BulkBody::PhoneRecords { records: vec![e.clone(); MAX_UPLOAD_RECORDS + 1] };
        assert!(BulkBody::decode(&many.encode().unwrap()).is_err());
        let too_many = BulkBody::MirrorBatch {
            req: 1,
            generation: 0,
            epoch: Uuid::nil(),
            reset: false,
            head: 1,
            to: 1,
            upserts: vec![e; MAX_BATCH_UPSERTS + 1],
            deletes: Vec::new(),
            shortened: Vec::new(),
            more: false,
        };
        assert!(BulkBody::decode(&too_many.encode().unwrap()).is_err());
        let bad_tag = BulkBody::MirrorProfile {
            req: 1,
            generation: 0,
            tag: serde_bytes::ByteBuf::from(vec![0; 31]),
            profile: Box::new(largest_profile().without_lists()),
        };
        assert!(BulkBody::decode(&bad_tag.encode().unwrap()).is_err());
        assert!(BulkBody::decode(&vec![0; MAX_BULK_BYTES + 1]).is_err());
        assert!(BulkBody::decode(&[0xff]).is_err());
        let huge = BulkBody::PhoneRecords { records: vec![entry(&"x".repeat(MAX_BULK_BYTES))] };
        assert!(huge.encode().is_err(), "a body larger than the limit is never sent");
    }
}
