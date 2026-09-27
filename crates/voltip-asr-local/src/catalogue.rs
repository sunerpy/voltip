//! The model catalogue (docs/dictation.md §10 / §11 / §12): a static table compiled into the
//! binary. Six entries: the two Qwen3-ASR GGUF tiers for transcribe.cpp (`handy-computer` on
//! Hugging Face), the two int8 ONNX offline models for sherpa-onnx (`csukuangfj`), the streaming
//! Zipformer transducer for the live preview, and the Silero VAD — an auxiliary entry the UI never
//! shows as a card, installed alongside the first local model when `vad_trim` is on. The UI shows
//! tiers, never file formats.

use serde::{Deserialize, Serialize};
use voltip_core::{ModelInstallState, ModelState};

/// The entry selected when `EngineSettings.local_model` is `None` (`qwen3-asr-0.6b`, tier 均衡).
pub const DEFAULT_MODEL_ID: &str = voltip_core::DEFAULT_LOCAL_MODEL_ID;
/// The streaming model of docs/dictation.md §11 (the only entry with [`Capability::Streaming`]).
pub const STREAMING_MODEL_ID: &str = "zipformer-stream-zh-en";
/// The Silero voice activity detector of docs/dictation.md §12 (the only entry with
/// [`Capability::Vad`]; [`Tier::Auxiliary`], never a card).
pub const VAD_MODEL_ID: &str = "silero-vad";

/// Recogniser family a model runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Engine {
    /// transcribe.cpp (ggml) over a Qwen3-ASR GGUF: whole-utterance, 30 languages auto-detected,
    /// punctuation built in. No streaming and no language hint (upstream, verified 2026-09-25).
    TranscribeCpp,
    /// FunASR SenseVoice through sherpa-onnx (multilingual, punctuation and ITN built in).
    SenseVoice,
    /// FunASR Paraformer through sherpa-onnx (Chinese, no punctuation).
    Paraformer,
    /// Streaming Zipformer transducer through sherpa-onnx's `OnlineRecognizer` (preview only).
    ZipformerStreaming,
    /// Silero voice activity detection through sherpa-onnx's `VoiceActivityDetector`: no text,
    /// only where the speech is (docs/dictation.md §12 `vad_trim`).
    SileroVad,
}

impl Engine {
    /// Wire name (`ModelState.engine`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TranscribeCpp => "transcribe_cpp",
            Self::SenseVoice => "sense_voice",
            Self::Paraformer => "paraformer",
            Self::ZipformerStreaming => "zipformer_streaming",
            Self::SileroVad => "silero_vad",
        }
    }

    /// Whether this family produces a whole-take transcript (as opposed to partial results only,
    /// or no text at all).
    pub fn is_offline(self) -> bool {
        !matches!(self, Self::ZipformerStreaming | Self::SileroVad)
    }
}

/// Product tier the settings dialog groups by (docs/dictation.md §10「产品档位」).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// 均衡（推荐）.
    Balanced,
    /// 高精度.
    Accurate,
    /// 轻量.
    Light,
    /// 实时预览.
    Streaming,
    /// A dependency of the other tiers (the VAD): never shown as a card, installed with the first
    /// local model when the feature that needs it is on (docs/dictation.md §12).
    Auxiliary,
}

impl Tier {
    /// Wire name (`ModelState.tier`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Balanced => "balanced",
            Self::Accurate => "accurate",
            Self::Light => "light",
            Self::Streaming => "streaming",
            Self::Auxiliary => "auxiliary",
        }
    }

    /// Whether the settings dialog shows entries of this tier as model cards.
    pub fn is_visible(self) -> bool {
        !matches!(self, Self::Auxiliary)
    }
}

/// What a model can do (`ModelState.capabilities`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// Whole-take transcription (the final text).
    Offline,
    /// Partial results while recording (the live preview).
    Streaming,
    /// Voice activity detection: where the speech is, no text (docs/dictation.md §12).
    Vad,
}

impl Capability {
    /// Wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Offline => voltip_core::CAPABILITY_OFFLINE,
            Self::Streaming => voltip_core::CAPABILITY_STREAMING,
            Self::Vad => voltip_core::CAPABILITY_VAD,
        }
    }
}

/// One file of a model directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelFile {
    /// File name inside the model directory and inside the Hugging Face repo.
    pub name: &'static str,
    /// Exact size in bytes.
    pub size: u64,
    /// Lower-case hex sha256 of the whole file.
    pub sha256: &'static str,
}

/// One downloadable model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelEntry {
    /// Catalogue id (also the directory name under the models root).
    pub id: &'static str,
    /// Display name.
    pub name: &'static str,
    /// Recogniser family.
    pub engine: Engine,
    /// Product tier.
    pub tier: Tier,
    /// What it can do.
    pub capabilities: &'static [Capability],
    /// ISO language codes.
    pub languages: &'static [&'static str],
    /// One line for the model card.
    pub description: &'static str,
    /// The catalogue's default choice.
    pub recommended: bool,
    /// Hugging Face repo (`owner/name`).
    pub repo: &'static str,
    /// Every file of the model, primary first (one GGUF; ONNX model + tokens; the streaming
    /// transducer's five files).
    pub files: &'static [ModelFile],
}

impl ModelEntry {
    /// Every file, primary first.
    pub fn files(&self) -> &'static [ModelFile] {
        self.files
    }

    /// The file called `name`.
    pub fn file(&self, name: &str) -> Option<&'static ModelFile> {
        self.files.iter().find(|f| f.name == name)
    }

    /// Bytes on disk once installed.
    pub fn size_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    /// Whether the model previews (`capabilities` names [`Capability::Streaming`]).
    pub fn is_streaming(&self) -> bool {
        self.capabilities.contains(&Capability::Streaming)
    }

    /// Whether the entry is the voice activity detector (`capabilities` names [`Capability::Vad`]).
    pub fn is_vad(&self) -> bool {
        self.capabilities.contains(&Capability::Vad)
    }

    /// The UI view of this entry in the given install state (`active` is the core's business).
    pub fn state(&self, state: ModelInstallState) -> ModelState {
        ModelState {
            id: self.id.to_owned(),
            name: self.name.to_owned(),
            engine: self.engine.as_str().to_owned(),
            tier: self.tier.as_str().to_owned(),
            capabilities: self.capabilities.iter().map(|c| c.as_str().to_owned()).collect(),
            languages: self.languages.iter().map(|l| (*l).to_owned()).collect(),
            size_bytes: self.size_bytes(),
            description: self.description.to_owned(),
            recommended: self.recommended,
            repo: self.repo.to_owned(),
            active: false,
            state,
        }
    }
}

const OFFLINE: &[Capability] = &[Capability::Offline];
const STREAMING: &[Capability] = &[Capability::Streaming];
const VAD: &[Capability] = &[Capability::Vad];

/// The catalogue, recommended entry first, then by tier (docs/dictation.md §10 table).
pub const CATALOGUE: &[ModelEntry] = &[
    ModelEntry {
        id: "qwen3-asr-0.6b",
        name: "均衡",
        engine: Engine::TranscribeCpp,
        tier: Tier::Balanced,
        capabilities: OFFLINE,
        languages: &["zh", "en", "ja", "ko", "yue", "de", "fr", "es", "ru", "ar"],
        description: "推荐；Qwen3-ASR 0.6B，30 语种自动识别，自带标点；690 MB",
        recommended: true,
        repo: "handy-computer/Qwen3-ASR-0.6B-gguf",
        files: &[ModelFile { name: "Qwen3-ASR-0.6B-Q6_K.gguf", size: 690_417_824, sha256: "3b051f108f03c0c91bbe1a3b2c1ee15e3ed51e4caec2a48751b01f2a21441cc3" }],
    },
    ModelEntry {
        id: "qwen3-asr-1.7b",
        name: "高精度",
        engine: Engine::TranscribeCpp,
        tier: Tier::Accurate,
        capabilities: OFFLINE,
        languages: &["zh", "en", "ja", "ko", "yue", "de", "fr", "es", "ru", "ar"],
        description: "Qwen3-ASR 1.7B，与云端同款，识别更准但更慢；1.7 GB",
        recommended: false,
        repo: "handy-computer/Qwen3-ASR-1.7B-gguf",
        files: &[ModelFile {
            name: "Qwen3-ASR-1.7B-Q6_K.gguf",
            size: 1_692_554_208,
            sha256: "c75a961b7134a6c952d89797865cb0d0376876185aee04ef6d12c31c2952e4e1",
        }],
    },
    ModelEntry {
        id: "sense-voice-small",
        name: "轻量",
        engine: Engine::SenseVoice,
        tier: Tier::Light,
        capabilities: OFFLINE,
        languages: &["zh", "en", "ja", "ko", "yue"],
        description: "SenseVoice Small，中英日韩粤，自带标点与数字规整（ITN）；240 MB",
        recommended: false,
        repo: "csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17",
        files: &[
            ModelFile { name: "model.int8.onnx", size: 239_233_841, sha256: "c71f0ce00bec95b07744e116345e33d8cbbe08cef896382cf907bf4b51a2cd51" },
            ModelFile { name: "tokens.txt", size: 315_894, sha256: "f449eb28dc567533d7fa59be34e2abca8784f771850c78a47fb731a31429a1dc" },
        ],
    },
    ModelEntry {
        id: "paraformer-zh",
        name: "轻量 · 中文",
        engine: Engine::Paraformer,
        tier: Tier::Light,
        capabilities: OFFLINE,
        languages: &["zh", "en"],
        description: "Paraformer 中文（含方言）更准，中英混读；无标点，开启 AI 润色可补；227 MB",
        recommended: false,
        repo: "csukuangfj/sherpa-onnx-paraformer-zh-2024-03-09",
        files: &[
            ModelFile { name: "model.int8.onnx", size: 227_330_205, sha256: "90bc03034ae1bef9575f8cc798cd1519c8be8aa9e8b458a033e32017ff4d584c" },
            ModelFile { name: "tokens.txt", size: 75_354, sha256: "6c0e3b35cece259829e6cb5b8d90d13db88f61ea3a2953d11898e4b2bfd7a2e2" },
        ],
    },
    ModelEntry {
        id: STREAMING_MODEL_ID,
        name: "实时预览",
        engine: Engine::ZipformerStreaming,
        tier: Tier::Streaming,
        capabilities: STREAMING,
        languages: &["zh", "en"],
        description: "边说边出字的预览模型（Zipformer 流式，中英混读，自带标点）；最终文本仍由所选引擎识别；169 MB",
        recommended: false,
        repo: "csukuangfj/sherpa-onnx-x-asr-480ms-streaming-zipformer-transducer-zh-en-punct-int8-2026-06-05",
        files: &[
            ModelFile { name: "encoder.int8.onnx", size: 155_278_641, sha256: "908596dcc137a73b95be908ca55e88caa1b3dbbe8027c171615f4b0609c5eb1e" },
            ModelFile { name: "decoder.onnx", size: 11_309_084, sha256: "a1cbc9eac2d5e3fb6617a218c67ad6daaa7f4e0fd225f08b2c22ab0413c8c257" },
            ModelFile { name: "joiner.int8.onnx", size: 2_581_422, sha256: "aedb7fa697b2ab43f20499826fff7c997eea7d67db77be97769aeeeb726e63b3" },
            ModelFile { name: "tokens.txt", size: 58_806, sha256: "b818a60878b9aae978cbb8ad594acbd403d76d1af2e31ef4197c84e2dbdba27c" },
            ModelFile { name: "bpe.model", size: 119_265, sha256: "f87a38025a5fdd1e4e9591f6a44bb81295097ce0b80df6f4ab9f44e52c64ca5f" },
        ],
    },
    ModelEntry {
        id: VAD_MODEL_ID,
        name: "语音活动检测",
        engine: Engine::SileroVad,
        tier: Tier::Auxiliary,
        capabilities: VAD,
        languages: &["zh", "en", "ja", "ko", "yue", "de", "fr", "es", "ru", "ar"],
        description: "Silero VAD，本地整段识别前裁掉首尾静音的依赖；随首个本地模型下载，不单独显示；1.8 MB",
        recommended: false,
        // Size and sha256 taken from the Hugging Face API (`?blobs=true`) on 2026-09-25 and
        // verified against the downloaded file; sherpa-onnx's own release asset (643 854 B) is a
        // different export of the same v4 model and is not what this entry pins.
        repo: "csukuangfj/vad",
        files: &[ModelFile { name: "silero_vad.onnx", size: 1_807_522, sha256: "a35ebf52fd3ce5f1469b2a36158dba761bc47b973ea3382b3186ca15b1f5af28" }],
    },
];

/// Look an id up in the catalogue.
pub fn entry(id: &str) -> Option<&'static ModelEntry> {
    CATALOGUE.iter().find(|e| e.id == id)
}

/// The default entry.
pub fn default_entry() -> &'static ModelEntry {
    entry(DEFAULT_MODEL_ID).unwrap_or(&CATALOGUE[0])
}

/// The streaming (preview) entry.
pub fn streaming_entry() -> &'static ModelEntry {
    CATALOGUE.iter().find(|e| e.is_streaming()).unwrap_or(&CATALOGUE[CATALOGUE.len() - 1])
}

/// The voice activity detector entry (docs/dictation.md §12).
pub fn vad_entry() -> &'static ModelEntry {
    CATALOGUE.iter().find(|e| e.is_vad()).unwrap_or(&CATALOGUE[CATALOGUE.len() - 1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_invariants() {
        assert_eq!(CATALOGUE.len(), 6);
        assert_eq!(default_entry().id, DEFAULT_MODEL_ID);
        assert_eq!(DEFAULT_MODEL_ID, "qwen3-asr-0.6b", "the core's default id names the catalogue default");
        assert_eq!(CATALOGUE.iter().filter(|e| e.recommended).count(), 1, "exactly one recommended entry");
        assert!(CATALOGUE[0].recommended, "recommended first");
        assert_eq!(
            CATALOGUE.iter().map(|e| e.id).collect::<Vec<_>>(),
            ["qwen3-asr-0.6b", "qwen3-asr-1.7b", "sense-voice-small", "paraformer-zh", "zipformer-stream-zh-en", "silero-vad"]
        );
        let mut ids: Vec<&str> = CATALOGUE.iter().map(|e| e.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), CATALOGUE.len(), "ids are unique");
        assert_eq!(CATALOGUE.iter().filter(|e| e.is_streaming()).count(), 1, "exactly one streaming entry");
        assert_eq!(streaming_entry().id, STREAMING_MODEL_ID);
        assert_eq!(CATALOGUE.iter().filter(|e| e.is_vad()).count(), 1, "exactly one VAD entry");
        assert_eq!(vad_entry().id, VAD_MODEL_ID);
        assert_eq!(CATALOGUE.iter().filter(|e| !e.tier.is_visible()).map(|e| e.id).collect::<Vec<_>>(), [VAD_MODEL_ID], "only the VAD is hidden");
        for e in CATALOGUE {
            assert!(e.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.'), "{}: id is a safe directory name", e.id);
            assert!(!e.name.is_empty() && !e.description.is_empty() && !e.languages.is_empty());
            let (owner, name) = e.repo.split_once('/').unwrap_or_else(|| panic!("{}", e.repo));
            assert!(!owner.is_empty() && !name.is_empty() && !name.contains('/'), "{}", e.repo);
            assert!((1..=5).contains(&e.files().len()), "{}: {} files", e.id, e.files().len());
            let mut names: Vec<&str> = e.files().iter().map(|f| f.name).collect();
            names.sort_unstable();
            names.dedup();
            assert_eq!(names.len(), e.files().len(), "{}: file names are unique", e.id);
            for f in e.files() {
                assert_eq!(f.sha256.len(), 64, "{}/{}", e.id, f.name);
                assert!(f.sha256.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()), "{}/{}", e.id, f.name);
                assert!(f.size > 0 && !f.name.contains('/') && !f.name.contains(".."));
                assert_eq!(e.file(f.name), Some(f));
            }
            assert_eq!(e.size_bytes(), e.files().iter().map(|f| f.size).sum::<u64>());
            assert_eq!(entry(e.id), Some(e));
            // Streaming entries carry only `streaming`, the VAD only `vad`; every other one only `offline`.
            if e.is_streaming() {
                assert_eq!(e.capabilities, STREAMING);
                assert_eq!((e.engine, e.tier), (Engine::ZipformerStreaming, Tier::Streaming));
                assert!(!e.engine.is_offline());
                assert!(!e.is_vad() && e.tier.is_visible());
            } else if e.is_vad() {
                assert_eq!(e.capabilities, VAD);
                assert_eq!((e.engine, e.tier), (Engine::SileroVad, Tier::Auxiliary));
                assert!(!e.engine.is_offline() && !e.tier.is_visible() && !e.recommended);
            } else {
                assert_eq!(e.capabilities, OFFLINE);
                assert!(!matches!(e.tier, Tier::Streaming | Tier::Auxiliary));
                assert!(e.engine.is_offline() && e.tier.is_visible());
            }
            let state = e.state(ModelInstallState::Verifying);
            assert_eq!(state.id, e.id);
            assert_eq!(state.engine, e.engine.as_str());
            assert_eq!(state.tier, e.tier.as_str());
            assert_eq!(state.is_streaming(), e.is_streaming());
            assert_eq!(state.is_vad(), e.is_vad());
            assert_eq!(state.capabilities, e.capabilities.iter().map(|c| c.as_str()).collect::<Vec<_>>());
            assert!(!state.active);
            assert_eq!(state.state, ModelInstallState::Verifying);
            assert_eq!(state.languages.len(), e.languages.len());
        }
        // The two engine families' file layouts.
        for id in ["qwen3-asr-0.6b", "qwen3-asr-1.7b"] {
            let e = entry(id).unwrap();
            assert_eq!(e.engine, Engine::TranscribeCpp);
            assert_eq!(e.files().len(), 1);
            assert!(e.files()[0].name.ends_with("-Q6_K.gguf"), "{}", e.files()[0].name);
            assert!(e.repo.starts_with("handy-computer/"));
        }
        for id in ["sense-voice-small", "paraformer-zh"] {
            let e = entry(id).unwrap();
            assert_eq!(e.files().iter().map(|f| f.name).collect::<Vec<_>>(), ["model.int8.onnx", "tokens.txt"]);
            assert!(e.repo.starts_with("csukuangfj/"));
            assert_eq!(e.tier, Tier::Light);
        }
        let q = entry("qwen3-asr-0.6b").unwrap();
        assert_eq!((q.tier, q.size_bytes()), (Tier::Balanced, 690_417_824));
        assert_eq!(entry("qwen3-asr-1.7b").unwrap().size_bytes(), 1_692_554_208);
        let sv = entry("sense-voice-small").unwrap();
        assert_eq!(sv.engine, Engine::SenseVoice);
        assert_eq!(sv.size_bytes(), 239_549_735);
        assert_eq!(sv.languages, &["zh", "en", "ja", "ko", "yue"]);
        let pf = entry("paraformer-zh").unwrap();
        assert_eq!(pf.engine, Engine::Paraformer);
        assert_eq!(pf.size_bytes(), 227_405_559);
        let zf = streaming_entry();
        assert_eq!(zf.files().iter().map(|f| f.name).collect::<Vec<_>>(), ["encoder.int8.onnx", "decoder.onnx", "joiner.int8.onnx", "tokens.txt", "bpe.model"]);
        assert_eq!(zf.size_bytes(), 169_347_218);
        let vad = vad_entry();
        assert_eq!(vad.files().iter().map(|f| f.name).collect::<Vec<_>>(), ["silero_vad.onnx"]);
        assert_eq!((vad.repo, vad.size_bytes()), ("csukuangfj/vad", 1_807_522));
        assert_eq!(vad.file("silero_vad.onnx").unwrap().sha256, "a35ebf52fd3ce5f1469b2a36158dba761bc47b973ea3382b3186ca15b1f5af28");
        assert_eq!(serde_json::to_string(&Engine::SileroVad).unwrap(), r#""silero_vad""#);
        assert_eq!(serde_json::to_string(&Tier::Auxiliary).unwrap(), r#""auxiliary""#);
        assert_eq!(Capability::Vad.as_str(), "vad");
        assert_eq!(entry("ghost"), None);
        assert_eq!(serde_json::to_string(&Engine::SenseVoice).unwrap(), r#""sense_voice""#);
        assert_eq!(serde_json::to_string(&Engine::TranscribeCpp).unwrap(), r#""transcribe_cpp""#);
        assert_eq!(serde_json::to_string(&Engine::ZipformerStreaming).unwrap(), r#""zipformer_streaming""#);
        assert_eq!(serde_json::from_str::<Engine>(r#""paraformer""#).unwrap(), Engine::Paraformer);
        assert_eq!(Engine::Paraformer.as_str(), "paraformer");
        assert_eq!(serde_json::to_string(&Tier::Accurate).unwrap(), r#""accurate""#);
        assert_eq!(serde_json::from_str::<Tier>(r#""light""#).unwrap(), Tier::Light);
        assert_eq!(
            [Tier::Balanced, Tier::Accurate, Tier::Light, Tier::Streaming, Tier::Auxiliary].map(Tier::as_str),
            ["balanced", "accurate", "light", "streaming", "auxiliary"]
        );
        assert_eq!(serde_json::to_string(&Capability::Streaming).unwrap(), r#""streaming""#);
        assert_eq!(Capability::Offline.as_str(), "offline");
    }
}
