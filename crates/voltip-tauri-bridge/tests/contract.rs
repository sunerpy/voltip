#![allow(clippy::unwrap_used, clippy::expect_used)]
//! IPC contract fixtures shared with the TypeScript side.
//!
//! The Rust types (`UiState`, `UiEvent`, `UiCommand`) are the source of truth for the wire format.
//! This test serializes representative values with serde and compares them byte-for-byte with the
//! JSON checked in under `packages/shared/src/fixtures/ipc/`; `ipc-contract.test.ts` parses the
//! same files with the zod schemas and replays the command entries through `TauriBackend`.
//!
//! Regenerate after an intentional contract change:
//! `UPDATE_IPC_FIXTURES=1 cargo test -p voltip-tauri-bridge --test contract`.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use voltip_core::audio_cache::{AudioCacheView, KeptFormat, KeptRecording, KeptStatus};
use voltip_core::backup::auto::{AutoBackupFailure, AutoBackupFile, AutoBackupView, AutoKind};
use voltip_core::connectivity::{ConnectivityReport, ConnectivityStatus, PeerCheck, ProbeResult, RelayCheck};
use voltip_core::dictation::{ClipboardCode, FailureCode, InsertCheck, ProcessingStage, Via};
use voltip_core::history::process::ProcessState;
use voltip_core::history::{ImportedFile, ProcessedText};
use voltip_core::import::{ImportFailure, ImportJob, ImportState, ImportsView};
use voltip_core::mcp::{HistoryRange, McpAction, McpActivity, McpClient, McpRead, McpSettings, McpStatus, McpWrite};
use voltip_core::models_location::{ModelsLocationView, ModelsMove, ModelsMoveOutcome};
use voltip_core::paste::{CopyReason, PasteFailure, PasteOutcome};
use voltip_core::phone::{PhoneTakeFailure, PhoneTakeState, PhoneTakeView};
use voltip_core::phone::{PhoneTextSource, SentText, SentTextFailure, SentTextState};
use voltip_core::presets::{BuiltinPreset, CustomPreset, PresetDraft, PresetId, PresetRef, PresetTryOutcome};
use voltip_core::ui::{
    GpuDevice, HardwareStatus, HotkeyCapabilities, HotkeyStatus, NetworkStatus, RefineNotice, ServePhase, ServeStatus, UiEvent, UiState, UpdateStatus,
};
use voltip_core::{
    Activation, AppRef, BuiltIn, BuiltinScene, CAPABILITY_OFFLINE, CAPABILITY_STREAMING, ChineseScript, ClipboardAfter, ContextSharing, DeviceConnection,
    DeviceView, DictationPhase, DictationStatus, DictionaryDraft, DictionaryEntry, EditRecord, EngineSettings, EngineStatus, EntrySource, FallbackModel,
    FallbackSettings, HistoryEntry, HistoryHits, HistoryPage, HistoryQuery, HistoryStats, HistoryStatsBucket, ImportMode, InjectMode, LiveText, LocalDevice,
    Locale, ModelFileView, ModelInstallState, ModelPackageView, ModelState, Outcome, OutputMode, PackageProblem, ProbeFailure, ProbeOutcome, ProbeReport,
    ProviderId, ProviderSettings, RefineFailure, RelaySource, RelayStatus, ReplacementRule, ResolvedEngines, RuleDraft, RuleKind, Scene, SceneDraft,
    SceneMatch, SceneOverrides, SceneRef, Segment, ServiceKind, Settings, SoloKey, SoloLoss, SwitchedWindow, TakeContext, TakeKind, ThemeId, UserSecrets,
    VocabularyHit, VocabularyHits,
};
use voltip_core::{EntryOrigin, FailureKind, OriginKind, RelayFailure};
use voltip_crypto::{PublicKey, SafetyCode};
use voltip_identity::{DeviceIdentityPublic, TrustedDevice};
use voltip_pairing::{FailureReason, PairingState, Snapshot};
use voltip_protocol::relay::RelayErrorCode;
use voltip_protocol::{DeviceId, DeviceInfo, Platform, SessionId};
use voltip_tauri_bridge::UiCommand;
use voltip_transport::ConnectionState;

const UPDATE_ENV: &str = "UPDATE_IPC_FIXTURES";
const STATE_FILE: &str = "state.json";
const EVENTS_FILE: &str = "events.json";
const COMMANDS_FILE: &str = "commands.json";
const BUILTIN_SCENES_FILE: &str = "scenes-builtin.json";
/// The answers of the history queries (docs/dictation.md §4.4).
const HISTORY_QUERIES_FILE: &str = "history-queries.json";
/// The drafts and answers of `vocabulary_backtest` (docs/dictation.md §16.9).
const VOCABULARY_BACKTEST_FILE: &str = "vocabulary-backtest.json";
/// The answers of the dictionary's learning suggestions and the rules page's hints
/// (docs/dictation.md §16.10).
const VOCABULARY_SUGGESTIONS_FILE: &str = "vocabulary-suggestions.json";
/// The commands that connect an AI tool to the MCP service (docs/dictation.md §25), as the core
/// copies them; `packages/shared/src/mcp.ts` shows the same with a placeholder.
const MCP_SETUP_FILE: &str = "mcp-setup.json";
/// The history page's token diff (`refine.ts` `diffSegments`) as the MCP service's correction
/// summary computes it in Rust (docs/dictation.md §25).
const DIFF_FILE: &str = "diff.json";

const DESKTOP_KEY: PublicKey = PublicKey([0x11; 32]);
const PHONE_KEY: PublicKey = PublicKey([0x22; 32]);
const TABLET_KEY: PublicKey = PublicKey([0x33; 32]);
const DESKTOP_DEVICE_ID: &str = "0f3f1a1e-8d4b-4c8e-9f7a-1c2d3e4f5a6b";
const PHONE_DEVICE_ID: &str = "7a6b5c4d-3e2f-4a1b-8c9d-0e1f2a3b4c5d";
const TABLET_DEVICE_ID: &str = "c0ffee00-1234-4abc-9def-0123456789ab";
const SESSION_ID: &str = "5eed5eed-0000-4000-8000-00000000c0de";
const RELAY_URL: &str = "wss://relay.example.test/ws";
const TRUSTED_AT: u64 = 1_758_700_000;
const LAST_SEEN: u64 = 1_758_700_600;
const EXPIRES_AT: u64 = 1_758_700_120;
const REMAINING_SECS: u64 = 87;
const RELAY_ATTEMPTS: u32 = 2;
const HISTORY_ID: &str = "9b1deb4d-3b7d-4bad-9bdd-2b0d7b3dcb6d";
const HISTORY_ID_2: &str = "1c2d3e4f-5a6b-4c8e-9f7a-0f3f1a1e8d4b";
/// Kept recordings (docs/dictation.md §26).
const RECORDING_ID: &str = "4a7c2e10-6b1d-4f3a-9c8e-5d2b7a1f0e93";
const RECORDING_ID_2: &str = "8e3f1b2a-0c4d-4e5f-a6b7-c8d9e0f1a2b3";
const RECORDING_ID_3: &str = "d1e2f3a4-b5c6-4d7e-8f9a-0b1c2d3e4f5a";
const RECORDING_ID_4: &str = "2f4e6d8c-0a1b-4c3d-9e5f-7a8b9c0d1e2f";
const AT_MS: u64 = 1_758_700_600_000;
const RAW_TEXT: &str = "把 fetchUser 改成 async 然后加上错误处理";
const REFINED_TEXT: &str = "把 fetchUser 改成 async，然后加上错误处理。";
const ASR_MODEL: &str = "Qwen/Qwen3-ASR-1.7B";
const REFINE_MODEL: &str = "qwen/qwen3.8-27b";
const QWEN_ID: &str = "qwen3-asr-0.6b";
const SENSE_VOICE_ID: &str = "sense-voice-small";
const PARAFORMER_ID: &str = "paraformer-zh";
const STREAMING_ID: &str = "zipformer-stream-zh-en";
const QWEN_BYTES: u64 = 690_417_824;
const SENSE_VOICE_BYTES: u64 = 239_549_735;
const PARAFORMER_BYTES: u64 = 227_405_559;
const STREAMING_BYTES: u64 = 169_347_218;
/// The sherpa-onnx release package the streaming model comes in (docs/dictation.md §29.12).
const STREAMING_PACKAGE: &str = "sherpa-onnx-x-asr-480ms-streaming-zipformer-transducer-zh-en-punct-int8-2026-06-05";
const LIVE_COMMITTED: &str = "把 fetchUser 改成 async，";
const LIVE_CURRENT: &str = "然后加上错误";
const MODELS_DIR: &str = "C:\\Users\\me\\AppData\\Roaming\\dev.voltip.desktop\\models";
const DICT_ID: &str = "3b241101-e2bb-4255-8caf-4136c566a962";
const DICT_ID_2: &str = "6f1c2d3e-4a5b-4c6d-8e7f-9a0b1c2d3e4f";
const RULE_ID: &str = "8d7e6f5a-4b3c-4d2e-9f1a-0b9c8d7e6f5a";
const RULE_ID_2: &str = "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d";
const HISTORY_ID_3: &str = "2d3e4f5a-6b7c-4d8e-9f0a-1b2c3d4e5f6a";
const HISTORY_ID_4: &str = "6a0c1f5e-2b7d-4c3a-9e8f-0d1c2b3a4f56";
/// An imported file's entry, and the jobs of the import queue (docs/dictation.md §31).
const HISTORY_ID_5: &str = "f1e2d3c4-b5a6-4978-8a6b-5c4d3e2f1a0b";
const IMPORT_IDS: [&str; 13] = [
    "a0000000-0000-4000-8000-000000000001",
    "a0000000-0000-4000-8000-000000000002",
    "a0000000-0000-4000-8000-000000000003",
    "a0000000-0000-4000-8000-000000000004",
    "a0000000-0000-4000-8000-000000000005",
    "a0000000-0000-4000-8000-000000000006",
    "a0000000-0000-4000-8000-000000000007",
    "a0000000-0000-4000-8000-000000000008",
    "a0000000-0000-4000-8000-000000000009",
    "a0000000-0000-4000-8000-00000000000a",
    "a0000000-0000-4000-8000-00000000000b",
    "a0000000-0000-4000-8000-00000000000c",
    "a0000000-0000-4000-8000-00000000000d",
];
/// A voice edit (docs/dictation.md §19): the selection, the spoken instruction, the rewrite.
const EDIT_SELECTION: &str = "大家好，会议改到周四十点哈";
const EDIT_INSTRUCTION: &str = "改得更正式";
const EDIT_REWRITE: &str = "各位同事：会议改至周四上午十点。";
const SCENE_ID: &str = "5c0ffee0-1a2b-4c3d-8e4f-5a6b7c8d9e0f";
const SCENE_ID_2: &str = "e0e1e2e3-e4e5-4e6e-8e7e-8e9eaebecede";
const SCENE_ID_3: &str = "b0117e1e-5ce0-4e5e-8a1e-000000000003";
const PRESET_ID: &str = "7e57ab1e-0b0e-4c0d-9e5e-7e57ab1e0b0e";
/// docs/dictation.md §25: an entry and a rule an AI tool wrote, and two lines of the activity.
const DICT_ID_3: &str = "4d0c7a1e-5b6c-4d7e-8f90-a1b2c3d4e5f6";
const RULE_ID_3: &str = "b2c3d4e5-f6a7-4b8c-9d0e-1f2a3b4c5d6e";
const MCP_ACTIVITY_ID: &str = "c3d4e5f6-a7b8-4c9d-8e0f-2a3b4c5d6e7f";
const MCP_ACTIVITY_ID_2: &str = "d4e5f6a7-b8c9-4d0e-9f1a-3b4c5d6e7f80";
const MCP_ACTIVITY_ID_3: &str = "e5f6a7b8-c9d0-4e1f-8a2b-4c5d6e7f8091";

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/shared/src/fixtures/ipc")
}

fn uuid<T: DeserializeOwned>(text: &str) -> T {
    serde_json::from_value(Value::String(text.to_owned())).unwrap()
}

/// A self-check from the phone: the relay answers, the computer is online with an encrypted round
/// trip, the tablet is offline.
fn connectivity_report() -> ConnectivityReport {
    ConnectivityReport {
        checked_at: AT_MS,
        relay: RelayCheck { configured: true, result: Some(ProbeResult::Ok { ms: 48 }) },
        peers: vec![
            PeerCheck { public_key: DESKTOP_KEY.to_hex(), name: "Surface-Laptop".into(), online: true, rtt_ms: Some(61) },
            PeerCheck { public_key: TABLET_KEY.to_hex(), name: "iPad".into(), online: false, rtt_ms: None },
        ],
    }
}

/// A self-check whose relay probe did not get through, one way after the other.
fn failed_connectivity_reports() -> Vec<ConnectivityReport> {
    [ProbeResult::Timeout, ProbeResult::Refused, ProbeResult::Failed { reason: "tls handshake eof".into(), kind: FailureKind::Tls }]
        .into_iter()
        .map(|result| ConnectivityReport { checked_at: AT_MS, relay: RelayCheck { configured: true, result: Some(result) }, peers: Vec::new() })
        .chain([ConnectivityReport { checked_at: AT_MS, relay: RelayCheck { configured: false, result: None }, peers: Vec::new() }])
        .collect()
}

/// What the hotkey can do in a session: `global` registers, `everywhere` fires over any window.
fn capabilities(global: bool, everywhere: bool) -> HotkeyCapabilities {
    HotkeyCapabilities {
        global,
        everywhere,
        hold: global,
        toggle_command: "voltip-desktop --toggle".into(),
        edit_toggle_command: "voltip-desktop --edit-toggle".into(),
        // A registering session watches lone keys too (docs/dictation.md §13.1); pure Wayland none.
        solo_keys: if global {
            vec![
                SoloKey::RightCtrl,
                SoloKey::RightAlt,
                SoloKey::RightShift,
                SoloKey::RightMeta,
                SoloKey::MouseMiddle,
                SoloKey::MouseBack,
                SoloKey::MouseForward,
            ]
        } else {
            Vec::new()
        },
    }
}

fn desktop_identity() -> DeviceIdentityPublic {
    DeviceIdentityPublic {
        device_id: uuid::<DeviceId>(DESKTOP_DEVICE_ID),
        name: "Surface-Laptop".into(),
        platform: Platform::Windows,
        public_key: DESKTOP_KEY,
        fingerprint: DESKTOP_KEY.fingerprint(),
    }
}

fn phone_device() -> TrustedDevice {
    TrustedDevice {
        device_id: uuid::<DeviceId>(PHONE_DEVICE_ID),
        name: "Pixel 10".into(),
        platform: Platform::Android,
        public_key: PHONE_KEY,
        fingerprint: PHONE_KEY.fingerprint(),
        trusted_at: TRUSTED_AT,
        last_seen: Some(LAST_SEEN),
        sync: true,
        sync_gen: 3,
    }
}

fn tablet_device() -> TrustedDevice {
    TrustedDevice {
        device_id: uuid::<DeviceId>(TABLET_DEVICE_ID),
        name: "iPad".into(),
        platform: Platform::Ios,
        public_key: TABLET_KEY,
        fingerprint: TABLET_KEY.fingerprint(),
        trusted_at: TRUSTED_AT,
        last_seen: None,
        sync: false,
        sync_gen: 1,
    }
}

fn safety_code() -> SafetyCode {
    SafetyCode { words: ["amber".into(), "canyon".into(), "lantern".into(), "orbit".into()], fingerprint: PHONE_KEY.fingerprint() }
}

fn idle_snapshot() -> Snapshot {
    Snapshot {
        state: PairingState::Idle,
        session_id: None,
        code: None,
        ticket_uri: None,
        expires_at: None,
        remaining_secs: None,
        safety_code: None,
        peer: None,
        local_confirmed: false,
        peer_confirmed: false,
        scanned: false,
    }
}

/// The computer after the phone proved it scanned the QR code: it confirmed by itself and waits
/// for the phone's user (docs/pairing.md 「扫码配对只在手机上确认」).
fn verification_snapshot() -> Snapshot {
    Snapshot {
        state: PairingState::AwaitingVerification,
        session_id: Some(uuid::<SessionId>(SESSION_ID)),
        code: Some("483 921".into()),
        ticket_uri: Some("voltip://pair?v=1&t=QUJDREVGR0g".into()),
        expires_at: Some(EXPIRES_AT),
        remaining_secs: Some(REMAINING_SECS),
        safety_code: Some(safety_code()),
        peer: Some(DeviceInfo { device_id: uuid::<DeviceId>(PHONE_DEVICE_ID), name: "Pixel 10".into(), platform: Platform::Android }),
        local_confirmed: true,
        peer_confirmed: false,
        scanned: true,
    }
}

/// The build's own service in these samples. Its host must never appear in any fixture: the
/// status reports no host for it (`regression_no_fixture_names_the_builtin_host`).
const BUILT: BuiltIn = BuiltIn {
    asr_url: Some("https://asr.builtin-host.test"),
    asr_token: Some("builtin-token"),
    asr_model: ASR_MODEL,
    refine_url: Some("https://llm.builtin-host.test/v1"),
    refine_api_key: Some("builtin-key"),
    refine_model: REFINE_MODEL,
    // The status says where the live preview comes from (docs/dictation.md §11.8).
    asr_live_preview: true,
};

/// Recognition on the built-in service, clean-up on Groq with a chosen model, a custom endpoint
/// configured but not in use.
fn engine_settings() -> EngineSettings {
    EngineSettings {
        asr_provider: ProviderId::Builtin,
        llm_provider: ProviderId::Groq,
        refine_enabled: true,
        // docs/dictation.md §21: a custom preset, so the sample shows the UUID form.
        refine_preset: PresetId::Custom(uuid(PRESET_ID)),
        providers: [
            (ProviderId::Groq, ProviderSettings { llm_model: Some("openai/gpt-oss-20b".into()), ..Default::default() }),
            (
                ProviderId::Custom,
                ProviderSettings {
                    asr_url: Some("https://asr.example.test".into()),
                    asr_model: Some(ASR_MODEL.into()),
                    // docs/dictation.md §3.7: the custom clean-up on the Responses interface.
                    llm_url: Some("http://127.0.0.1:8787/v1".into()),
                    llm_model: Some("claude-opus-5-5".into()),
                    llm_api: Some(voltip_core::LlmApi::Responses),
                    llm_reasoning: Some(voltip_core::ReasoningEffort::High),
                },
            ),
        ]
        .into(),
        local_model: None,
        local_device: LocalDevice::Auto,
        local_gpu: None,
        local_threads: None,
        language: Some("zh".into()),
        live_preview: true,
        output_mode: OutputMode::WholeTake,
        vad_trim: false,
        chinese_script: ChineseScript::Simplified,
        inject: InjectMode::Paste,
        // docs/dictation.md §14.3: the text stays on the clipboard, and a switched window still
        // gets the paste (the sample shows both non-default values).
        clipboard_after: ClipboardAfter::Keep,
        switched_window: SwitchedWindow::PasteHere,
        // docs/dictation.md §28: both switches off, away from their defaults, so the sample shows them.
        insert_check: false,
        result_cards_stay: false,
        // docs/dictation.md §3.5: a ready fallback model for the recognition (and the selected one,
        // listed again: skipped), one without its key for the clean-up. §3.14: the recognition also
        // moves on for failures, to a model of this device too (one installed, one downloading).
        asr_fallback: FallbackSettings {
            enabled: true,
            on_outage: true,
            models: vec![
                FallbackModel { provider: ProviderId::Custom, model: "qwen3-asr-flash".into() },
                FallbackModel { provider: ProviderId::Builtin, model: String::new() },
                FallbackModel { provider: ProviderId::Local, model: QWEN_ID.into() },
                FallbackModel { provider: ProviderId::Local, model: PARAFORMER_ID.into() },
            ],
        },
        llm_fallback: FallbackSettings {
            enabled: true,
            on_outage: false,
            models: vec![FallbackModel { provider: ProviderId::Openai, model: "gpt-6-luna".into() }],
        },
    }
}

fn user_keys() -> UserSecrets {
    let mut keys = UserSecrets::default();
    keys.set(ProviderId::Groq, ServiceKind::Llm, Some("gsk_example_not_a_real_key".into()));
    keys
}

fn settings() -> Settings {
    Settings {
        history: voltip_core::HistorySettings { enabled: true, keep: 200 },
        overlay: voltip_core::OverlayPlacement::Top,
        theme: ThemeId::Graphite,
        follow_system_theme: true,
        relay_url: Some(RELAY_URL.into()),
        engines: engine_settings(),
        locale: Locale::ZhCn,
        auto_update: true,
        activation: Activation::HoldOrToggle,
        hold_threshold_ms: 300,
        extra_recording_ms: 150,
        // docs/dictation.md §18.5: both switches away from their defaults, so the sample shows each.
        context_sharing: ContextSharing { app_name: false, window_title: true },
        // docs/dictation.md §13.1: a lone-key trigger next to the chord.
        solo_key: Some(SoloKey::RightCtrl),
        // docs/dictation.md §28: a 复制上一条结果 chord, unassigned by default.
        copy_last_hotkey: Some("Ctrl+Shift+V".into()),
        // docs/pairing.md 「常开配对」: on, away from its default.
        pairing_always_on: true,
        microphone: Some("wasapi:{0.0.1.00000000}.{c2}".into()),
        // docs/dictation.md §22: the microphone and the computer's sound, an hour at most, with the
        // echo cancellation (§22.6) away from its default.
        recording: voltip_core::RecordingSettings {
            source: voltip_core::RecordingSource::Mixed,
            output_device: Some("wasapi:{0.0.0.00000000}.{a1}".into()),
            max_minutes: 60,
            echo_cancel: false,
        },
        // docs/dictation.md §23.6: the local speech service on, with a preset and a scene.
        serve: voltip_core::ServeSettings {
            enabled: true,
            port: 47840,
            preset: Some(voltip_core::PresetId::Builtin(voltip_core::BuiltinPreset::Prompt)),
            scene: Some(uuid(SCENE_ID)),
        },
        // docs/dictation.md §24: a manual proxy that signs in, with hosts that go direct.
        proxy: voltip_core::ProxySettings {
            mode: voltip_core::ProxyMode::Manual,
            server: Some("http://proxy.example.com:3128".into()),
            username: Some("alice".into()),
            no_proxy: "corp.example.com,10.0.0.0/8".into(),
        },
        // docs/dictation.md §25: the MCP service on, sharing the full history of the last seven days.
        mcp: McpSettings { enabled: true, port: 47841, read: McpRead::Full, history_range: HistoryRange::Last7Days, write: McpWrite::Dictionary },
        // docs/dictation.md §20.11: the phone's talk button tapped, away from holding.
        phone_talk: voltip_core::PhoneTalkMode::Tap,
        // docs/dictation.md §26: the recording cache keeps more, in more space, than by default.
        audio_cache: voltip_core::AudioCacheSettings { enabled: true, keep: 50, max_mb: 500 },
        // docs/dictation.md §4.6.1: no weekly backup, and a folder of the user's.
        backup: voltip_core::BackupSettings { weekly: false, folder: Some("D:\\同步盘\\Voltip 备份".into()) },
        ..Settings::default()
    }
}

/// The scenes (docs/dictation.md §18.1): one with every override and a title keyword, one plain
/// and switched off, and a built-in one (§18.10) that lists no application yet.
fn scenes() -> Vec<Scene> {
    let legal = BuiltinScene::Legal.template(Platform::Windows);
    vec![
        Scene {
            id: uuid(SCENE_ID),
            name: "代码评审".into(),
            enabled: true,
            matching: SceneMatch { apps: vec!["chrome".into(), "code".into()], title_contains: vec!["Pull request".into()] },
            overrides: SceneOverrides {
                refine_enabled: Some(true),
                refine_preset: Some(PresetId::Builtin(BuiltinPreset::Formal)),
                output_mode: Some(OutputMode::StreamingFinal),
                language: Some("en".into()),
                chinese_script: Some(ChineseScript::AsIs),
                prompt: Some("这是代码评审意见：保留代码标识符原样。".into()),
            },
            created_at_ms: AT_MS - 86_400_000,
            updated_at_ms: AT_MS - 3_600_000,
            builtin: None,
        },
        Scene {
            id: uuid(SCENE_ID_2),
            name: "聊天".into(),
            enabled: false,
            matching: SceneMatch { apps: vec!["slack".into(), "wechat".into()], title_contains: Vec::new() },
            overrides: SceneOverrides { refine_preset: Some(PresetId::Builtin(BuiltinPreset::Punctuation)), ..SceneOverrides::default() },
            created_at_ms: AT_MS,
            updated_at_ms: AT_MS,
            builtin: None,
        },
        Scene {
            id: uuid(SCENE_ID_3),
            name: legal.name,
            enabled: legal.enabled,
            matching: legal.matching,
            overrides: legal.overrides,
            created_at_ms: AT_MS,
            updated_at_ms: AT_MS,
            builtin: Some(BuiltinScene::Legal),
        },
    ]
}

/// The custom presets (docs/dictation.md §21): one, the one the engine settings name.
fn custom_presets() -> Vec<CustomPreset> {
    vec![CustomPreset {
        id: uuid(PRESET_ID),
        name: "周报".into(),
        prompt: "把正文整理成周报：本周完成、下周计划、风险三部分。".into(),
        created_at_ms: AT_MS - 86_400_000,
        updated_at_ms: AT_MS,
    }]
}

/// The take's context (docs/dictation.md §18.6): the app in front and the scene that matched.
fn take_context() -> TakeContext {
    TakeContext {
        app: AppRef { id: "code".into(), name: "Code".into() }, scene: Some(SceneRef { id: uuid(SCENE_ID), name: "代码评审".into(), builtin: None })
    }
}

fn done_phase() -> DictationPhase {
    DictationPhase::Done {
        text: REFINED_TEXT.into(),
        raw_text: RAW_TEXT.into(),
        chars: REFINED_TEXT.chars().count(),
        via: Via::Paste,
        refined: true,
        duration_ms: 3200,
        asr_ms: 640,
        refine_ms: Some(2400),
        refine_failure: None,
        refine_error: None,
        mode: OutputMode::WholeTake,
        segments: None,
        live_error: None,
        clipboard_code: None,
        // docs/dictation.md §28: the insert check saw the text at the caret.
        check: Some(InsertCheck::Written),
    }
}

/// `Done` in `streaming_final` (docs/dictation.md §12): the flushed stream's sentences are the
/// text, the whole-take transcriber never ran (`asr_ms` is the finalisation time).
fn streamed_done_phase() -> DictationPhase {
    DictationPhase::Done {
        text: REFINED_TEXT.into(),
        raw_text: RAW_TEXT.into(),
        chars: REFINED_TEXT.chars().count(),
        via: Via::Paste,
        refined: true,
        duration_ms: 3200,
        asr_ms: 45,
        refine_ms: Some(2400),
        refine_failure: None,
        refine_error: None,
        mode: OutputMode::StreamingFinal,
        segments: Some(stream_segments()),
        live_error: None,
        clipboard_code: None,
        check: None,
    }
}

/// `Done` of a streaming mode that fell back to the whole take before the first sentence (§12):
/// `mode` reads `whole_take`, `live_error` says why.
fn fallen_back_done_phase() -> DictationPhase {
    match done_phase() {
        DictationPhase::Done { text, raw_text, chars, via, refined, duration_ms, asr_ms, refine_ms, refine_error, refine_failure, .. } => {
            DictationPhase::Done {
                text,
                raw_text,
                chars,
                via,
                refined,
                duration_ms,
                asr_ms,
                refine_ms,
                refine_error,
                refine_failure,
                mode: OutputMode::WholeTake,
                segments: None,
                live_error: Some("open: asr: 实时识别模型未下载：实时预览".into()),
                clipboard_code: None,
                check: None,
            }
        }
        other => other,
    }
}

/// The two sentences of the sample take as the streaming recogniser committed them.
fn stream_segments() -> Vec<Segment> {
    vec![
        Segment { text: LIVE_COMMITTED.into(), refined: None, start_ms: 0, end_ms: 1480 },
        Segment { text: "然后加上错误处理".into(), refined: None, start_ms: 1480, end_ms: 3200 },
    ]
}

/// 用 AI 预设处理's result as a long entry carries it (docs/dictation.md §22).
fn processed_text() -> ProcessedText {
    ProcessedText {
        text: "- 预算已批准\n- 下周三前提交方案".into(),
        preset: PresetRef { id: PresetId::Builtin(BuiltinPreset::Notes), name: "要点纪要".into() },
        at_ms: AT_MS + 60_000,
    }
}

fn history_entries() -> Vec<HistoryEntry> {
    vec![
        HistoryEntry {
            id: uuid(HISTORY_ID),
            at_ms: AT_MS,
            raw_text: RAW_TEXT.into(),
            text: REFINED_TEXT.into(),
            refined: true,
            asr_model: ASR_MODEL.into(),
            // docs/dictation.md §29: the local engine's GPU failed and the CPU did the take.
            asr_note: Some(voltip_core::dictation::AsrNote::GpuFailedRanOnCpu),
            // docs/dictation.md §3.14: the selected providers' models wrote it.
            asr_provider: Some(ProviderId::Builtin),
            asr_fallback: None,
            refine_model: Some(REFINE_MODEL.into()),
            refine_provider: Some(ProviderId::Groq),
            refine_fallback: None,
            duration_ms: 3200,
            asr_ms: 640,
            refine_ms: Some(2400),
            refine_failure: None,
            refine_error: None,
            // docs/dictation.md §28: the insert check could not tell.
            outcome: Outcome::Inserted { via: Via::Paste, check: Some(InsertCheck::Unconfirmed) },
            starred: true,
            mode: OutputMode::WholeTake,
            segments: None,
            live_error: None,
            vocabulary: Some(vocabulary_hits()),
            kind: TakeKind::Dictation,
            edit: None,
            app: Some(AppRef { id: "code".into(), name: "Code".into() }),
            scene: Some(SceneRef { id: uuid(SCENE_ID), name: "代码评审".into(), builtin: None }),
            preset: Some(PresetRef { id: PresetId::Builtin(BuiltinPreset::Formal), name: "书面语".into() }),
            origin: None,
            processed: None,
            file: None,
        },
        HistoryEntry {
            id: uuid(HISTORY_ID_2),
            at_ms: AT_MS - 60_000,
            raw_text: "测试一下".into(),
            text: "测试一下".into(),
            refined: false,
            asr_model: ASR_MODEL.into(),
            asr_note: None,
            // docs/dictation.md §3.14: the selected service failed and the built-in one stood in.
            asr_provider: Some(ProviderId::Builtin),
            asr_fallback: Some(voltip_core::dictation::SwitchReason::Outage),
            refine_model: None,
            refine_provider: None,
            refine_fallback: None,
            duration_ms: 900,
            asr_ms: 410,
            refine_ms: None,
            // docs/dictation.md §3.6: the clean-up was turned down, the raw text went in; the
            // service's words stay for the technical details.
            refine_failure: Some(RefineFailure::RateLimited),
            refine_error: Some("服务繁忙：refine service overloaded (HTTP 503): This model is currently experiencing high demand.".into()),
            // Written before the fallback codes (2026-09-29): no `code`.
            outcome: Outcome::Clipboard { reason: "没有可粘贴的前台窗口".into(), code: None },
            starred: false,
            mode: OutputMode::WholeTake,
            segments: None,
            live_error: None,
            vocabulary: None,
            kind: TakeKind::Dictation,
            edit: None,
            app: None,
            scene: None,
            preset: None,
            origin: None,
            processed: None,
            file: None,
        },
        // docs/dictation.md §19: the instruction is the raw text, the rewrite the text.
        HistoryEntry {
            id: uuid(HISTORY_ID_3),
            at_ms: AT_MS - 120_000,
            raw_text: EDIT_INSTRUCTION.into(),
            text: EDIT_REWRITE.into(),
            refined: true,
            asr_model: ASR_MODEL.into(),
            asr_note: None,
            asr_provider: None,
            asr_fallback: None,
            refine_model: Some(REFINE_MODEL.into()),
            refine_provider: None,
            refine_fallback: None,
            duration_ms: 1400,
            asr_ms: 380,
            refine_ms: Some(900),
            refine_failure: None,
            refine_error: None,
            // docs/dictation.md §28: the rewrite never reached the field (the 没有写进去 card).
            outcome: Outcome::Inserted { via: Via::Paste, check: Some(InsertCheck::NotWritten) },
            starred: false,
            mode: OutputMode::WholeTake,
            segments: None,
            live_error: None,
            vocabulary: None,
            kind: TakeKind::Edit,
            edit: Some(EditRecord { instruction: EDIT_INSTRUCTION.into(), selection: EDIT_SELECTION.into() }),
            // An edit carries the app it ran in, never a scene (docs/dictation.md §19).
            app: Some(AppRef { id: "slack".into(), name: "Slack".into() }),
            scene: None,
            preset: None,
            origin: None,
            processed: None,
            file: None,
        },
        // docs/dictation.md §20.6: text a phone sent, inserted as it was.
        HistoryEntry {
            id: uuid(HISTORY_ID_4),
            at_ms: AT_MS - 180_000,
            raw_text: "会议改到三点".into(),
            text: "会议改到三点".into(),
            refined: false,
            asr_model: String::new(),
            asr_note: None,
            asr_provider: None,
            asr_fallback: None,
            refine_model: None,
            refine_provider: None,
            refine_fallback: None,
            duration_ms: 0,
            asr_ms: 0,
            refine_ms: None,
            refine_failure: None,
            refine_error: None,
            outcome: Outcome::Inserted { via: Via::Paste, check: None },
            starred: false,
            mode: OutputMode::WholeTake,
            segments: None,
            live_error: None,
            vocabulary: None,
            kind: TakeKind::Dictation,
            edit: None,
            app: None,
            scene: None,
            preset: None,
            origin: Some(EntryOrigin { device: "Pixel 8".into(), kind: OriginKind::Typed }),
            processed: None,
            file: None,
        },
    ]
}

/// An imported file's entry (docs/dictation.md §31): its file's name and length, its segments with
/// their times and, where the polish ran, the polished text of each.
fn imported_history_entry() -> HistoryEntry {
    HistoryEntry {
        id: uuid(HISTORY_ID_5),
        at_ms: AT_MS - 240_000,
        raw_text: "嗯今天我们讨论一下进度然后呢就是下周发布".into(),
        text: "今天我们讨论一下进度。下周发布。".into(),
        refined: true,
        asr_model: "whisper-large-v3".into(),
        asr_note: None,
        asr_provider: None,
        asr_fallback: None,
        refine_model: Some("llama-3.3-70b-versatile".into()),
        refine_provider: None,
        refine_fallback: None,
        duration_ms: 9_500,
        asr_ms: 1_800,
        refine_ms: Some(1_200),
        refine_failure: None,
        refine_error: None,
        outcome: Outcome::Inserted { via: Via::Clipboard, check: None },
        starred: false,
        mode: OutputMode::WholeTake,
        segments: Some(vec![
            Segment { text: "嗯今天我们讨论一下进度".into(), refined: Some("今天我们讨论一下进度。".into()), start_ms: 0, end_ms: 4_000 },
            Segment { text: "然后呢就是下周发布".into(), refined: Some("下周发布。".into()), start_ms: 4_000, end_ms: 9_500 },
        ]),
        live_error: None,
        vocabulary: None,
        kind: TakeKind::Dictation,
        edit: None,
        app: None,
        scene: None,
        preset: Some(PresetRef { id: PresetId::Builtin(BuiltinPreset::Proofread), name: "校对".into() }),
        origin: None,
        processed: None,
        file: Some(ImportedFile { name: "季度会议.mp4".into(), duration_ms: 9_500 }),
    }
}

/// The import queue (docs/dictation.md §31): a job in every state, and every reason one fails.
fn imports_view() -> ImportsView {
    let job = |i: usize, name: &str, state: ImportState| ImportJob {
        id: uuid(IMPORT_IDS[i]),
        name: name.into(),
        size: 48_213_504,
        duration_ms: None,
        paused: false,
        state,
    };
    let failed = |i: usize, failure: ImportFailure, resumable: bool| job(i, "失败的文件.mkv", ImportState::Failed { failure, resumable });
    ImportsView {
        available: true,
        extensions: ["mp3", "m4a", "mp4", "mkv"].map(String::from).to_vec(),
        jobs: vec![
            ImportJob { duration_ms: Some(1_800_000), paused: true, ..job(0, "季度会议.mp4", ImportState::Recognizing { done: 12, total: 240 }) },
            job(1, "访谈.m4a", ImportState::Queued),
            job(2, "讲座.mov", ImportState::Decoding { pct: Some(42) }),
            job(3, "播客.mp3", ImportState::Decoding { pct: None }),
            ImportJob { duration_ms: Some(1_800_000), ..job(4, "课堂.webm", ImportState::Polishing { done: 3, total: 238 }) },
            ImportJob { duration_ms: Some(9_500), ..job(5, "季度会议.mp4", ImportState::Done { entry: uuid(HISTORY_ID_5), polish_failed: 2 }) },
            failed(6, ImportFailure::Recognition { detail: "额度已用完：quota exceeded (HTTP 429)".into() }, true),
            failed(7, ImportFailure::UnsupportedCodec { codec: "AC-3".into() }, false),
            failed(8, ImportFailure::Unreadable { detail: "No such file or directory (os error 2)".into() }, false),
            failed(9, ImportFailure::UnsupportedFormat, false),
            failed(10, ImportFailure::NoAudio, false),
            failed(11, ImportFailure::Corrupt { detail: "it ends after 12.0 s of 60.0 s".into() }, false),
            ImportJob {
                state: ImportState::Failed { failure: ImportFailure::Storage { detail: "No space left on device (os error 28)".into() }, resumable: false },
                ..failed(12, ImportFailure::NoSpeech, false)
            },
        ],
    }
}

/// What the dictionary and the rules did to the first history entry (docs/dictation.md §16.3).
fn vocabulary_hits() -> VocabularyHits {
    VocabularyHits { corrections: vec![VocabularyHit { id: uuid(DICT_ID), count: 1 }], rules: vec![VocabularyHit { id: uuid(RULE_ID_2), count: 2 }] }
}

/// The personal dictionary (docs/dictation.md §16.1): a manual entry and one added from history.
fn dictionary() -> Vec<DictionaryEntry> {
    vec![
        DictionaryEntry {
            id: uuid(DICT_ID),
            term: "fetchUser".into(),
            heard_as: vec!["fetch user".into(), "费驰优瑟".into()],
            enabled: true,
            source: EntrySource::Manual,
            created_at_ms: AT_MS - 86_400_000,
            updated_at_ms: AT_MS - 3_600_000,
        },
        DictionaryEntry {
            id: uuid(DICT_ID_2),
            term: "good idea".into(),
            heard_as: vec!["谷歌IDR".into()],
            enabled: false,
            source: EntrySource::History { history_id: uuid(HISTORY_ID) },
            created_at_ms: AT_MS,
            updated_at_ms: AT_MS,
        },
        // docs/dictation.md §25: written by an AI tool over the MCP service.
        DictionaryEntry {
            id: uuid(DICT_ID_3),
            term: "Voltip".into(),
            heard_as: vec!["维他命".into()],
            enabled: true,
            source: EntrySource::Mcp { client: "claude-code".into() },
            created_at_ms: AT_MS,
            updated_at_ms: AT_MS,
        },
    ]
}

/// The replacement rules (docs/dictation.md §16.1): one literal, one regex.
/// The pairs the learning suggestions leave out (docs/dictation.md §16.10), oldest first.
fn ignored_pairs() -> Vec<voltip_core::vocabulary::IgnoredPair> {
    use voltip_core::vocabulary::IgnoredPair;
    vec![
        IgnoredPair { heard: "集成".into(), polished: "继承".into(), at_ms: AT_MS - 86_400_000 },
        IgnoredPair { heard: "提成".into(), polished: "集成".into(), at_ms: AT_MS },
    ]
}

fn rules() -> Vec<ReplacementRule> {
    vec![
        ReplacementRule {
            id: uuid(RULE_ID),
            name: "git push".into(),
            kind: RuleKind::Literal,
            pattern: "给他push".into(),
            replacement: "git push".into(),
            case_sensitive: true,
            enabled: true,
            created_at_ms: AT_MS - 86_400_000,
            updated_at_ms: AT_MS - 86_400_000,
            source: EntrySource::Manual,
            pending: false,
        },
        ReplacementRule {
            id: uuid(RULE_ID_2),
            name: "PR 编号".into(),
            kind: RuleKind::Regex,
            pattern: r"\bpr (\d+)".into(),
            replacement: "PR #$1".into(),
            case_sensitive: false,
            enabled: false,
            created_at_ms: AT_MS,
            updated_at_ms: AT_MS,
            source: EntrySource::Manual,
            pending: false,
        },
        // docs/dictation.md §25: an AI tool's rule, off until the user approves it.
        ReplacementRule {
            id: uuid(RULE_ID_3),
            name: "非书 → 飞书".into(),
            kind: RuleKind::Literal,
            pattern: "非书".into(),
            replacement: "飞书".into(),
            case_sensitive: true,
            enabled: false,
            created_at_ms: AT_MS,
            updated_at_ms: AT_MS,
            source: EntrySource::Mcp { client: "claude-code".into() },
            pending: true,
        },
    ]
}

/// docs/dictation.md §25: the MCP service running, a client heard from, a write that can be undone,
/// a rule waiting for approval and a read of the full history.
fn mcp_status() -> McpStatus {
    McpStatus {
        available: true,
        phase: ServePhase::Running,
        address: Some("http://127.0.0.1:47841/mcp".into()),
        error: None,
        clients: vec![McpClient { name: "claude-code".into(), last_seen_ms: AT_MS }],
        activity: vec![
            McpActivity {
                id: uuid(MCP_ACTIVITY_ID),
                at_ms: AT_MS,
                client: "claude-code".into(),
                action: McpAction::AddEntry,
                subject: Some("Voltip".into()),
                count: None,
                range: None,
                pending: false,
                undoable: true,
                undone: false,
            },
            McpActivity {
                id: uuid(MCP_ACTIVITY_ID_2),
                at_ms: AT_MS - 60_000,
                client: "claude-code".into(),
                action: McpAction::AddRule,
                subject: Some("非书 → 飞书".into()),
                count: None,
                range: None,
                pending: true,
                undoable: true,
                undone: false,
            },
            McpActivity {
                id: uuid(MCP_ACTIVITY_ID_3),
                at_ms: AT_MS - 120_000,
                client: "codex".into(),
                action: McpAction::SearchHistory,
                subject: None,
                count: Some(12),
                range: Some(HistoryRange::Last30Days),
                pending: false,
                undoable: false,
                undone: false,
            },
        ],
    }
}

/// A `live_inject` entry (docs/dictation.md §12): the sentences it pasted, no refinement, and the
/// reason the stream stopped early (the remainder was transcribed and pasted last).
fn live_inject_history_entry() -> HistoryEntry {
    HistoryEntry {
        raw_text: RAW_TEXT.into(),
        text: RAW_TEXT.into(),
        refined: false,
        refine_model: None,
        refine_ms: None,
        asr_ms: 380,
        mode: OutputMode::LiveInject,
        segments: Some(stream_segments()),
        live_error: Some("live tap overrun: the decoder fell behind the microphone".into()),
        // A built-in scene (§18.10) is named by its category; the interface shows its own name.
        app: Some(AppRef { id: "winword".into(), name: "Word".into() }),
        scene: Some(SceneRef { id: uuid(SCENE_ID_3), name: "legal".into(), builtin: Some(BuiltinScene::Legal) }),
        preset: None,
        ..history_entries().remove(0)
    }
}

/// `EngineSettings` selecting a local model on a chosen GPU with a thread count (what
/// `settings_set_engines` sends to activate one), with the §12 fields set so the one command
/// sample carries every key.
fn local_engine_settings() -> EngineSettings {
    EngineSettings {
        asr_provider: ProviderId::Local,
        local_model: Some(PARAFORMER_ID.into()),
        local_device: LocalDevice::Gpu,
        local_gpu: Some("Vulkan0".into()),
        local_threads: Some(4),
        output_mode: OutputMode::StreamingFinal,
        vad_trim: true,
        chinese_script: ChineseScript::Traditional,
        ..engine_settings()
    }
}

/// The resolved status on-device: no host, readiness from the library (the Paraformer entry is
/// installed or still downloading).
fn local_engine_status(ready: bool) -> EngineStatus {
    let library: Vec<ModelState> = models_installed_and_downloading()
        .into_iter()
        .map(|m| {
            if m.id == PARAFORMER_ID && ready {
                ModelState { state: ModelInstallState::Installed { path: format!("{MODELS_DIR}\\{PARAFORMER_ID}"), installed_at: TRUSTED_AT }, ..m }
            } else if m.id == STREAMING_ID && !ready {
                ModelState { state: ModelInstallState::NotInstalled, ..m }
            } else {
                m
            }
        })
        .collect();
    ResolvedEngines::resolve_with_models(&local_engine_settings(), &user_keys(), &BUILT, &library).status()
}

/// The catalogue as the UI sees it, in the states `state.json` shows: the default (balanced) tier
/// installed and active, the light tiers not installed / downloading, the streaming model installed
/// (never active: it only previews).
fn models_installed_and_downloading() -> Vec<ModelState> {
    let offline = vec![CAPABILITY_OFFLINE.to_owned()];
    vec![
        ModelState {
            id: QWEN_ID.into(),
            name: "均衡".into(),
            engine: "transcribe_cpp".into(),
            tier: "balanced".into(),
            capabilities: offline.clone(),
            languages: ["zh", "en", "ja", "ko", "yue", "de", "fr", "es", "ru", "ar"].map(String::from).to_vec(),
            size_bytes: QWEN_BYTES,
            description: "推荐；Qwen3-ASR 0.6B，30 语种自动识别，自带标点；690 MB".into(),
            recommended: true,
            repo: "handy-computer/Qwen3-ASR-0.6B-gguf".into(),
            dir: format!("{MODELS_DIR}\\{QWEN_ID}"),
            files: model_files("handy-computer/Qwen3-ASR-0.6B-gguf", &[("Qwen3-ASR-0.6B-Q6_K.gguf", 690_417_824)]),
            package: None,
            active: true,
            state: ModelInstallState::Installed { path: format!("{MODELS_DIR}\\{QWEN_ID}"), installed_at: TRUSTED_AT },
        },
        ModelState {
            id: SENSE_VOICE_ID.into(),
            name: "轻量".into(),
            engine: "sense_voice".into(),
            tier: "light".into(),
            capabilities: offline.clone(),
            languages: ["zh", "en", "ja", "ko", "yue"].map(String::from).to_vec(),
            size_bytes: SENSE_VOICE_BYTES,
            description: "SenseVoice Small，中英日韩粤，自带标点与数字规整（ITN）；240 MB".into(),
            recommended: false,
            repo: "csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17".into(),
            dir: format!("{MODELS_DIR}\\{SENSE_VOICE_ID}"),
            files: model_files("csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17", &[("model.int8.onnx", 239_233_841), ("tokens.txt", 315_894)]),
            package: None,
            active: false,
            state: ModelInstallState::NotInstalled,
        },
        ModelState {
            id: PARAFORMER_ID.into(),
            name: "轻量 · 中文".into(),
            engine: "paraformer".into(),
            tier: "light".into(),
            capabilities: offline,
            languages: ["zh", "en"].map(String::from).to_vec(),
            size_bytes: PARAFORMER_BYTES,
            description: "Paraformer 中文（含方言）更准，中英混读；无标点，开启 AI 润色可补；227 MB".into(),
            recommended: false,
            repo: "csukuangfj/sherpa-onnx-paraformer-zh-2024-03-09".into(),
            dir: format!("{MODELS_DIR}\\{PARAFORMER_ID}"),
            files: model_files("csukuangfj/sherpa-onnx-paraformer-zh-2024-03-09", &[("model.int8.onnx", 227_330_205), ("tokens.txt", 75_354)]),
            package: None,
            active: false,
            state: ModelInstallState::Downloading { received: 104_857_600, total: 227_330_205, file: "model.int8.onnx".into() },
        },
        ModelState {
            id: STREAMING_ID.into(),
            name: "实时预览".into(),
            engine: "zipformer_streaming".into(),
            tier: "streaming".into(),
            capabilities: vec![CAPABILITY_STREAMING.to_owned()],
            languages: ["zh", "en"].map(String::from).to_vec(),
            size_bytes: STREAMING_BYTES,
            description: "边说边出字的预览模型（Zipformer 流式，中英混读，自带标点）；最终文本仍由所选引擎识别；169 MB".into(),
            recommended: false,
            repo: "csukuangfj/sherpa-onnx-x-asr-480ms-streaming-zipformer-transducer-zh-en-punct-int8-2026-06-05".into(),
            dir: format!("{MODELS_DIR}\\{STREAMING_ID}"),
            // Published as a package (docs/dictation.md §29.12): its files list no address of their own.
            files: [
                ("encoder.int8.onnx", 155_278_641),
                ("decoder.onnx", 11_309_084),
                ("joiner.int8.onnx", 2_581_422),
                ("tokens.txt", 58_806),
                ("bpe.model", 119_265),
            ]
            .map(|(name, size_bytes)| ModelFileView { name: name.into(), size_bytes, urls: Vec::new() })
            .to_vec(),
            package: Some(ModelPackageView {
                name: format!("{STREAMING_PACKAGE}.tar.bz2"),
                urls: vec![format!("https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/{STREAMING_PACKAGE}.tar.bz2")],
            }),
            active: false,
            state: ModelInstallState::Installed { path: format!("{MODELS_DIR}\\{STREAMING_ID}"), installed_at: TRUSTED_AT },
        },
    ]
}

/// A model's files with the two public addresses each, as `ModelStore::view` reports them.
fn model_files(repo: &str, files: &[(&str, u64)]) -> Vec<ModelFileView> {
    files
        .iter()
        .map(|(name, size)| ModelFileView {
            name: (*name).to_owned(),
            size_bytes: *size,
            urls: ["https://huggingface.co", "https://hf-mirror.com"].map(|base| format!("{base}/{repo}/resolve/main/{name}")).to_vec(),
        })
        .collect()
}

/// Every remaining `ModelInstallState` variant: verifying, failed (`.part` kept), an incomplete
/// manual import (docs/dictation.md §10), not installed.
fn models_other_states() -> Vec<ModelState> {
    let base = models_installed_and_downloading();
    vec![
        ModelState { active: false, state: ModelInstallState::Verifying, ..base[0].clone() },
        ModelState {
            state: ModelInstallState::ImportIncomplete { missing: vec!["tokens.txt".into()], mismatched: vec!["model.int8.onnx".into()] },
            ..base[1].clone()
        },
        ModelState { active: true, state: ModelInstallState::Failed { message: "sha256 mismatch: model.int8.onnx".into(), package: None }, ..base[2].clone() },
        // The streaming model downloads as a package (docs/dictation.md §29.12).
        ModelState {
            state: ModelInstallState::Downloading { received: 52_428_800, total: 133_895_136, file: format!("{STREAMING_PACKAGE}.tar.bz2") },
            ..base[3].clone()
        },
    ]
}

/// A package that could not be used (docs/dictation.md §29.12), each problem once.
fn models_package_failed() -> Vec<ModelState> {
    let streaming = models_installed_and_downloading().remove(3);
    [
        PackageProblem::Broken,
        PackageProblem::Missing { files: vec!["tokens.txt".into()] },
        PackageProblem::Mismatched { files: vec!["decoder.onnx".into(), "bpe.model".into()] },
    ]
    .into_iter()
    .map(|problem| ModelState {
        state: ModelInstallState::Failed { message: "模型包无法使用：压缩包损坏或不完整".into(), package: Some(problem) },
        ..streaming.clone()
    })
    .collect()
}

/// The live preview while listening (docs/dictation.md §11): one committed sentence, one in progress.
fn live_text() -> LiveText {
    LiveText {
        committed: vec![Segment { text: LIVE_COMMITTED.into(), refined: None, start_ms: 0, end_ms: 1480 }],
        current: LIVE_CURRENT.into(),
        degraded: None,
        injected: 0,
    }
}

fn models_not_installed() -> Vec<ModelState> {
    models_installed_and_downloading().into_iter().map(|m| ModelState { active: false, state: ModelInstallState::NotInstalled, ..m }).collect()
}

fn engine_status() -> EngineStatus {
    let resolved = ResolvedEngines::resolve_with_models(&engine_settings(), &user_keys(), &BUILT, &models_installed_and_downloading());
    // docs/dictation.md §3.5: the built-in recognition ran out of quota at the sample's time, so it
    // is tried again a day later; §3.14: the first fallback model failed and is skipped for a
    // minute, so the model of this device is in use.
    let ledger = voltip_core::dictation::QuotaLedger::with_clock(|| AT_MS);
    ledger.mark(resolved.asr_fallback.selected.as_ref().expect("the sample's fallback models run"), voltip_core::dictation::SwitchReason::Quota);
    let custom = resolved.asr_fallback.targets().find(|t| t.provider == ProviderId::Custom).expect("the custom fallback model is ready");
    ledger.mark(&custom.key, voltip_core::dictation::SwitchReason::Outage);
    let mut status = resolved.status_with(&ledger);
    // docs/dictation.md §3.3: the models Groq listed for its saved key.
    let groq = status.providers.iter_mut().find(|p| p.id == ProviderId::Groq).and_then(|p| p.llm.as_mut()).expect("Groq offers clean-up");
    groq.listed = vec!["openai/gpt-oss-120b".into(), "qwen/qwen3.8-27b".into()];
    status
}

fn devices() -> Vec<DeviceView> {
    vec![
        DeviceView { device: phone_device(), connection: DeviceConnection::Online },
        DeviceView { device: tablet_device(), connection: DeviceConnection::IdentityChanged { presented_fingerprint: TABLET_KEY.fingerprint() } },
    ]
}

/// The audio cache (docs/dictation.md §26) with a recording of every status, and a long one kept as
/// its compressed parts (§26.2).
fn audio_cache() -> AudioCacheView {
    let app = Some(AppRef { id: "code".into(), name: "Visual Studio Code".into() });
    let recording = |id: &str, ago: u64, status: KeptStatus| KeptRecording {
        id: uuid(id),
        at_ms: AT_MS - ago,
        duration_ms: 6_800,
        bytes: 217_644,
        app: app.clone(),
        status,
        format: KeptFormat::Wav,
    };
    let recordings = vec![
        recording(RECORDING_ID, 0, KeptStatus::Pending),
        recording(RECORDING_ID_2, 60_000, KeptStatus::Failed { code: FailureCode::Asr, message: "asr: 请求超时".into() }),
        recording(RECORDING_ID_3, 120_000, KeptStatus::Recognized { entry: uuid(HISTORY_ID) }),
        recording(RECORDING_ID_4, 180_000, KeptStatus::Cancelled),
        KeptRecording {
            app: None,
            duration_ms: 3_725_000,
            bytes: 10_057_500,
            format: KeptFormat::Opus,
            ..recording(HISTORY_ID_2, 240_000, KeptStatus::Interrupted)
        },
    ];
    let bytes = recordings.iter().map(|r| r.bytes).sum();
    AudioCacheView { available: true, recordings, bytes }
}

/// Where the model library is (docs/dictation.md §10.9): a chosen folder, a move to another one
/// halfway, and how the move before it ended.
fn models_location() -> ModelsLocationView {
    ModelsLocationView {
        available: true,
        dir: "D:\\Voltip\\models".into(),
        custom: true,
        default_dir: "C:\\Users\\me\\AppData\\Roaming\\dev.voltip.desktop\\models".into(),
        moving: Some(ModelsMove { to: "E:\\Models".into(), moved_bytes: 268_435_456, total_bytes: 930_170_389 }),
        last: Some(ModelsMoveOutcome::Moved { dir: "D:\\Voltip\\models".into(), deferred: 1 }),
    }
}

/// The automatic backups (docs/dictation.md §4.6.1): a folder of the user's with this computer's
/// weekly backup in it (its tag in the name, user decision D23), the failure of the one before an
/// update, and its notice on home.
fn auto_backup() -> AutoBackupView {
    let failure = AutoBackupFailure { kind: AutoKind::Update, at_ms: AT_MS - 60_000, detail: "Access is denied. (os error 5)".into() };
    let device = voltip_core::backup::auto::device_tag(&DESKTOP_KEY);
    AutoBackupView {
        available: true,
        last: Some(AutoBackupFile { name: format!("voltip-auto-20261030-152000-weekly-{device}.voltip-backup"), at_ms: AT_MS, kind: AutoKind::Weekly }),
        device,
        folder: "D:\\同步盘\\Voltip 备份".into(),
        default_folder: "C:\\Users\\me\\AppData\\Roaming\\voltip\\Voltip\\data\\backups".into(),
        custom: true,
        failure: Some(failure.clone()),
        notice: Some(failure),
    }
}

/// A fully populated state: every `Option` is `Some`, so the TypeScript schema sees each key.
fn full_state() -> UiState {
    UiState {
        app_version: "0.3.0".into(),
        identity: Some(desktop_identity()),
        settings: settings(),
        secret_backend: "memory".into(),
        relay: RelayStatus {
            endpoint: Some(RELAY_URL.into()),
            source: RelaySource::User,
            state: ConnectionState::Connected,
            attempts: RELAY_ATTEMPTS,
            last_failure: None,
        },
        pairing: verification_snapshot(),
        devices: devices(),
        hotkey: HotkeyStatus {
            registered: Some("Ctrl+Alt+Space".into()),
            error: None,
            pressed: false,
            capturing: false,
            backend: "global-shortcut · Windows · RegisterHotKey".into(),
            edit_registered: Some("Ctrl+Alt+E".into()),
            edit_error: None,
            copy_last_registered: Some("Ctrl+Shift+V".into()),
            copy_last_error: None,
            capabilities: capabilities(true, true),
            solo_registered: Some(SoloKey::RightCtrl),
            solo_error: None,
            solo_loss: None,
            solo_pressed: false,
            solo_recovered_at: None,
        },
        presets: custom_presets(),
        dictation: DictationStatus {
            phase: done_phase(),
            session: 7,
            context: Some(take_context()),
            kind: TakeKind::Dictation,
            remote: Some("Pixel 8".into()),
            preset: None,
            source: None,
            segments: None,
            // docs/dictation.md §26: a take recognising a kept recording again.
            recording: Some(uuid(RECORDING_ID)),
            retake: true,
        },
        history_recent: history_entries(),
        audio_cache: audio_cache(),
        imports: imports_view(),
        auto_backup: auto_backup(),
        // More entries than the recent ones: the rest are read through the queries.
        history_total: 312,
        engines: engine_status(),
        update: UpdateStatus::Available {
            version: "2.1.0".into(),
            current: "0.0.1".into(),
            notes: Some("修复听写热键冲突".into()),
            date: Some("2026-09-25T08:00:00Z".into()),
        },
        models: models_installed_and_downloading(),
        models_location: models_location(),
        dictionary: dictionary(),
        rules: rules(),
        vocabulary_ignored: ignored_pairs(),
        scenes: scenes(),
        phone_take: Some(PhoneTakeView {
            device: DESKTOP_KEY.to_hex(),
            take: 3,
            started_at: TRUSTED_AT * 1000,
            state: PhoneTakeState::Done { text: "把 fetchUser 改成 async".into(), pasted: true },
            // docs/dictation.md §20.1: the take went out as Opus.
            opus: true,
        }),
        sent_texts: sent_texts(),
        hardware: hardware_status(),
        connectivity: ConnectivityStatus { running: false, report: Some(connectivity_report()) },
        mirrors: mirror_views(),
        phone_outbox_too_large: vec![uuid(HISTORY_ID)],
        // docs/dictation.md §23.6: the app's local speech service, running.
        serve: ServeStatus { available: true, phase: ServePhase::Running, address: Some("http://127.0.0.1:47840/v1".into()), error: None },
        // docs/dictation.md §24: the proxy's password saved, and the system's proxy.
        network: NetworkStatus { proxy_password: true, system_proxy: Some("http://127.0.0.1:7890".into()) },
        // docs/dictation.md §25: the MCP service running, with its clients and activity.
        mcp: mcp_status(),
        // docs/dictation.md §3.6: the built-in clean-up service turned a take down.
        refine_notice: Some(RefineNotice { failure: RefineFailure::RateLimited, at_ms: 1_727_400_000_000 }),
        // docs/dictation.md §27: the built-in service's day and this device's record.
        usage: usage_view(),
    }
}

/// The built-in service in its limit phase, this device's last days, and every part of today's
/// record (docs/dictation.md §27).
fn usage_view() -> voltip_core::usage::UsageView {
    use voltip_core::usage::{
        AsrQuota, LedgerView, OriginUse, ProviderUse, PurposeUse, QuotaBasis, QuotaMode, QuotaReport, RefineQuota, UsageDayView, UsageNotice, UsageOrigin,
        UsagePurpose, UsageView,
    };
    UsageView {
        builtin: true,
        quota: Some(QuotaReport {
            mode: QuotaMode::Limit,
            basis: QuotaBasis::Device,
            day: "2026-10-27".into(),
            reset_at: 1_793_116_800,
            asr: AsrQuota { used_seconds: 1_380, limit_seconds: Some(3_600) },
            refine: RefineQuota { used: 72, limit: Some(100) },
        }),
        reported_at: Some(1_793_080_000_000),
        unavailable: false,
        record: LedgerView {
            days: vec![
                UsageDayView { day: "2026-10-26".into(), asr_seconds: 2_400, refine: 88 },
                UsageDayView { day: "2026-10-27".into(), asr_seconds: 1_380, refine: 72 },
            ],
            asr_origins: vec![
                OriginUse { origin: UsageOrigin::ThisDevice, seconds: 1_080 },
                OriginUse { origin: UsageOrigin::Phone { name: "Pixel 8".into() }, seconds: 300 },
                OriginUse { origin: UsageOrigin::Retake, seconds: 0 },
                OriginUse { origin: UsageOrigin::Serve, seconds: 0 },
            ],
            refine_purposes: vec![PurposeUse { purpose: UsagePurpose::Refine, count: 60 }, PurposeUse { purpose: UsagePurpose::Edit, count: 12 }],
            others: vec![
                ProviderUse { provider: ProviderId::Local, service: ServiceKind::Asr, model: Some("qwen3-asr-0.6b".into()), amount: 7_800 },
                ProviderUse { provider: ProviderId::Openai, service: ServiceKind::Llm, model: None, amount: 3 },
            ],
            others_today: vec![ProviderUse { provider: ProviderId::Local, service: ServiceKind::Asr, model: Some("qwen3-asr-0.6b".into()), amount: 420 }],
            dismissed: vec![UsageNotice::AsrNear],
        },
    }
}

/// A machine with a discrete and an integrated GPU (docs/dictation.md §10.6).
fn hardware_status() -> HardwareStatus {
    HardwareStatus {
        cpu_threads: 16,
        gpus: vec![
            GpuDevice { name: "Vulkan0".into(), description: "NVIDIA L40S".into(), kind: "vulkan".into(), memory_mb: 46_068, integrated: false },
            GpuDevice { name: "Vulkan1".into(), description: "Intel(R) UHD Graphics 770".into(), kind: "vulkan".into(), memory_mb: 0, integrated: true },
        ],
    }
}

/// Stable tag for each variant; an exhaustive `match` so a new variant breaks this test at compile time.
fn event_tag(event: &UiEvent) -> &'static str {
    match event {
        UiEvent::State(_) => "state",
        UiEvent::Identity(_) => "identity",
        UiEvent::Settings(_) => "settings",
        UiEvent::Relay(_) => "relay",
        UiEvent::Pairing(_) => "pairing",
        UiEvent::Devices { .. } => "devices",
        UiEvent::Trusted(_) => "trusted",
        UiEvent::Unpaired(_) => "unpaired",
        UiEvent::IdentityChanged { .. } => "identity_changed",
        UiEvent::Message { .. } => "message",
        UiEvent::Error { .. } => "error",
        UiEvent::Hotkey(_) => "hotkey",
        UiEvent::Dictation(_) => "dictation",
        UiEvent::History { .. } => "history",
        UiEvent::Engines(_) => "engines",
        UiEvent::Models { .. } => "models",
        UiEvent::ModelsLocation(_) => "models_location",
        UiEvent::Update(_) => "update",
        UiEvent::Dictionary { .. } => "dictionary",
        UiEvent::Rules { .. } => "rules",
        UiEvent::VocabularyIgnored { .. } => "vocabulary_ignored",
        UiEvent::Scenes { .. } => "scenes",
        UiEvent::Presets { .. } => "presets",
        UiEvent::PresetTry { .. } => "preset_try",
        UiEvent::HistoryProcess { .. } => "history_process",
        UiEvent::AudioCache(_) => "audio_cache",
        UiEvent::Imports(_) => "imports",
        UiEvent::AutoBackup(_) => "auto_backup",
        UiEvent::ProviderProbe(_) => "provider_probe",
        UiEvent::PhoneTake { .. } => "phone_take",
        UiEvent::SentTexts { .. } => "sent_texts",
        UiEvent::Hardware(_) => "hardware",
        UiEvent::Connectivity(_) => "connectivity",
        UiEvent::Serve(_) => "serve",
        UiEvent::Network(_) => "network",
        UiEvent::Mcp(_) => "mcp",
        UiEvent::PasteResult { .. } => "paste_result",
        UiEvent::Mirrors { .. } => "mirrors",
        UiEvent::PhoneOutbox { .. } => "phone_outbox",
        UiEvent::RefineNotice { .. } => "refine_notice",
        UiEvent::Usage(_) => "usage",
    }
}

const ALL_EVENT_TAGS: [&str; 33] = [
    "state",
    "identity",
    "settings",
    "relay",
    "pairing",
    "devices",
    "trusted",
    "identity_changed",
    "message",
    "error",
    "hotkey",
    "dictation",
    "history",
    "engines",
    "models",
    "update",
    "dictionary",
    "rules",
    "scenes",
    "presets",
    "preset_try",
    "provider_probe",
    "phone_take",
    "hardware",
    "paste_result",
    "mirrors",
    "phone_outbox",
    "refine_notice",
    "network",
    "mcp",
    "usage",
    "imports",
    "auto_backup",
];

/// The computer's settings as a phone shows them (docs/dictation.md §20.8): every `Option` set.
fn mirror_profile() -> voltip_core::sync::Profile {
    voltip_core::sync::Profile {
        locale: Locale::System,
        theme: ThemeId::Dark,
        follow_system_theme: true,
        asr_provider: ProviderId::Builtin,
        asr_model: "Qwen/Qwen3-ASR-1.7B".into(),
        local_model: Some(QWEN_ID.into()),
        refine_enabled: true,
        llm_provider: Some(ProviderId::Builtin),
        refine_model: "qwen/qwen3.8-27b".into(),
        preset: PresetId::Custom(uuid(PRESET_ID)),
        presets: custom_presets(),
        dictionary: dictionary(),
        rules: rules(),
        scenes: scenes(),
    }
}

/// A phone's copies (docs/dictation.md §20.8): one computer in every state.
fn mirror_views() -> Vec<voltip_core::sync::MirrorView> {
    use voltip_core::sync::{MirrorSyncState, MirrorView};
    let view = |name: &str, state, entries, synced: bool| MirrorView {
        desktop: DESKTOP_KEY.to_hex(),
        name: name.into(),
        state,
        entries,
        synced_at_ms: synced.then_some(AT_MS),
        profile: synced.then(|| mirror_profile().without_lists()),
    };
    vec![
        view("MacBook Pro", MirrorSyncState::Syncing, 1200, true),
        view("Surface", MirrorSyncState::UpToDate, 312, true),
        view("Studio", MirrorSyncState::Offline, 87, true),
        view("Office", MirrorSyncState::Revoked, 0, false),
        view("Old Desk", MirrorSyncState::NeedsUpgrade, 0, false),
        view("Sixth", MirrorSyncState::Limit, 0, false),
    ]
}

/// The phone's list (docs/dictation.md §20.6): one text in every state.
fn sent_texts() -> Vec<SentText> {
    let text = |id: u32, body: &str, source, state| SentText {
        id,
        device: DESKTOP_KEY.to_hex(),
        device_name: "MacBook Pro".into(),
        body: body.into(),
        source,
        sent_at: TRUSTED_AT * 1000 + u64::from(id),
        state,
    };
    vec![
        text(5, "https://example.test/a", PhoneTextSource::Clipboard, SentTextState::Sending),
        text(4, "会议改到三点", PhoneTextSource::Typed, SentTextState::Queued),
        text(3, "收到", PhoneTextSource::Typed, SentTextState::Delivered { pasted: true }),
        text(2, "地址在群里", PhoneTextSource::Clipboard, SentTextState::Delivered { pasted: false }),
        text(1, "晚点回电", PhoneTextSource::Typed, SentTextState::Failed { code: SentTextFailure::NoAnswer, message: "电脑没有回应".into() }),
    ]
}

/// A phone take to the desktop in `state`, through the fold.
fn phone_take_event(state: PhoneTakeState) -> UiEvent {
    UiState::default().apply(voltip_core::CoreEvent::PhoneTake(Some(PhoneTakeView {
        device: DESKTOP_KEY.to_hex(),
        take: 3,
        started_at: TRUSTED_AT * 1000,
        opus: matches!(state, PhoneTakeState::Listening | PhoneTakeState::Processing),
        state,
    })))
}

/// Built through the fold, like `devices_event`.
fn history_event(entries: Vec<HistoryEntry>) -> UiEvent {
    let total = u32::try_from(entries.len()).unwrap();
    UiState::default().apply(voltip_core::CoreEvent::History { recent: entries, total })
}

/// Built through the fold so this test does not name the variant's shape (`UiEvent::Devices` must
/// be a struct variant for serde's internal tagging to accept the list; see `ui.rs`).
fn devices_event(list: Vec<DeviceView>) -> UiEvent {
    UiState::default().apply(voltip_core::CoreEvent::Devices(list))
}

/// Same for the model library (a list, hence a struct variant).
fn models_event(list: Vec<ModelState>) -> UiEvent {
    UiState::default().apply(voltip_core::CoreEvent::Models(list))
}

/// Same for the dictionary and the rules (docs/dictation.md §16.4).
fn dictionary_event(list: Vec<DictionaryEntry>) -> UiEvent {
    UiState::default().apply(voltip_core::CoreEvent::Dictionary(list))
}

fn rules_event(list: Vec<ReplacementRule>) -> UiEvent {
    UiState::default().apply(voltip_core::CoreEvent::Rules(list))
}

/// Same for the scenes (docs/dictation.md §18.6).
fn scenes_event(list: Vec<Scene>) -> UiEvent {
    UiState::default().apply(voltip_core::CoreEvent::Scenes(list))
}

/// Same for the custom presets (docs/dictation.md §21).
fn presets_event(list: Vec<CustomPreset>) -> UiEvent {
    UiState::default().apply(voltip_core::CoreEvent::Presets(list))
}

/// One value per `UiEvent` variant, plus the shapes the TypeScript union has to discriminate
/// (`devices` with every connection kind, a failed pairing, a minimal default state).
fn all_events() -> Vec<UiEvent> {
    let mut events = vec![
        UiEvent::State(Box::default()),
        UiEvent::Mirrors { mirrors: mirror_views() },
        UiEvent::PhoneOutbox { too_large: vec![uuid(HISTORY_ID)] },
        UiEvent::RefineNotice { notice: Some(RefineNotice { failure: RefineFailure::Quota, at_ms: 1_727_400_000_000 }) },
        UiEvent::RefineNotice { notice: None },
        UiEvent::Usage(usage_view()),
        // A build without the built-in service: nothing reported, nothing recorded.
        UiEvent::Usage(voltip_core::usage::UsageView::default()),
        UiEvent::Identity(desktop_identity()),
        UiEvent::Settings(settings()),
        UiEvent::Relay(RelayStatus {
            endpoint: Some(RELAY_URL.into()),
            source: RelaySource::User,
            state: ConnectionState::Reconnecting,
            attempts: RELAY_ATTEMPTS,
            // A relay the user typed in keeps the transport's words; the build's own only its kind.
            last_failure: Some(RelayFailure { kind: FailureKind::Timeout, detail: Some("connect timed out after 8s".into()), min_version: None, at: AT_MS }),
        }),
        // The build's own relay: connected, and never named.
        UiEvent::Relay(RelayStatus { endpoint: None, source: RelaySource::Builtin, state: ConnectionState::Connected, attempts: 0, last_failure: None }),
        // Below the build's own relay's minimum: the version to update to, still without the host.
        UiEvent::Relay(RelayStatus {
            endpoint: None,
            source: RelaySource::Builtin,
            state: ConnectionState::Reconnecting,
            attempts: 1,
            last_failure: Some(RelayFailure { kind: FailureKind::TooOld, detail: None, min_version: Some("0.2.0".into()), at: AT_MS }),
        }),
        UiEvent::Pairing(verification_snapshot()),
        UiEvent::Pairing(Snapshot { state: PairingState::Failed { reason: FailureReason::Relay { code: RelayErrorCode::SessionExpired } }, ..idle_snapshot() }),
        UiEvent::Pairing(Snapshot { state: PairingState::Failed { reason: FailureReason::Timeout }, ..idle_snapshot() }),
        devices_event(vec![
            DeviceView { device: phone_device(), connection: DeviceConnection::Offline },
            DeviceView { device: tablet_device(), connection: DeviceConnection::Connecting },
            DeviceView { device: phone_device(), connection: DeviceConnection::Online },
            DeviceView { device: tablet_device(), connection: DeviceConnection::IdentityChanged { presented_fingerprint: DESKTOP_KEY.fingerprint() } },
        ]),
        devices_event(Vec::new()),
        UiEvent::Trusted(phone_device()),
        UiEvent::Unpaired(phone_device()),
        UiEvent::IdentityChanged { previous: phone_device(), presented_fingerprint: TABLET_KEY.fingerprint() },
        UiEvent::Message { from: PHONE_KEY.to_hex(), body: "把 fetchUser 改成 async".into() },
        UiEvent::Error { message: "relay refused: session_expired".into() },
        UiEvent::Hotkey(HotkeyStatus {
            registered: Some("Ctrl+Alt+Space".into()),
            error: None,
            pressed: true,
            capturing: false,
            backend: "global-shortcut · Windows · RegisterHotKey".into(),
            edit_registered: Some("Ctrl+Alt+E".into()),
            edit_error: None,
            copy_last_registered: Some("Ctrl+Shift+V".into()),
            copy_last_error: None,
            capabilities: capabilities(true, true),
            solo_registered: Some(SoloKey::MouseBack),
            solo_error: None,
            solo_loss: None,
            solo_pressed: true,
            solo_recovered_at: Some(AT_MS),
        }),
        UiEvent::Hotkey(HotkeyStatus {
            registered: None,
            error: Some("Ctrl+Alt+Space 注册失败：already registered".into()),
            pressed: false,
            capturing: false,
            backend: "global-shortcut · Linux · XWayland".into(),
            edit_registered: None,
            edit_error: Some("Ctrl+Alt+E 注册失败：already registered".into()),
            copy_last_registered: None,
            copy_last_error: Some("Ctrl+Shift+V 注册失败：already registered".into()),
            capabilities: capabilities(true, false),
            solo_registered: Some(SoloKey::RightAlt),
            solo_error: None,
            // The X connection went away; the hook reconnects (docs/dictation.md §29).
            solo_loss: Some(SoloLoss::Display),
            solo_pressed: false,
            solo_recovered_at: None,
        }),
        UiEvent::Hotkey(HotkeyStatus {
            registered: None,
            error: Some("Ctrl+Alt+Space 未能生效：纯 Wayland 会话不允许应用设置全局快捷键".into()),
            pressed: false,
            capturing: false,
            backend: "global-shortcut · Linux · Wayland".into(),
            edit_registered: None,
            edit_error: None,
            copy_last_registered: None,
            copy_last_error: None,
            capabilities: capabilities(false, false),
            solo_registered: None,
            solo_error: Some("右 Ctrl 无法单独触发：纯 Wayland 会话不允许应用监听全局按键".into()),
            solo_loss: None,
            solo_pressed: false,
            solo_recovered_at: None,
        }),
        // Every dictation phase the pill and the home page discriminate on. `listening` in its three
        // shapes (device opening; ready with the live preview; preview degraded), `processing` with
        // and without the carried-over preview (docs/dictation.md §11).
        UiEvent::Dictation(DictationStatus {
            phase: DictationPhase::Idle,
            session: 6,
            context: None,
            kind: TakeKind::Dictation,
            remote: None,
            preset: None,
            source: None,
            segments: None,
            recording: None,
            retake: false,
        }),
        UiEvent::Dictation(DictationStatus {
            phase: DictationPhase::Listening { started_at: AT_MS, ready: false, live: None, locked: false },
            session: 7,
            context: None,
            kind: TakeKind::Dictation,
            remote: None,
            preset: None,
            source: None,
            segments: None,
            recording: None,
            retake: false,
        }),
        // docs/dictation.md §22: a long take from the microphone and the computer's sound, past its
        // first two minutes, with segments recognised as it goes.
        UiEvent::Dictation(DictationStatus {
            phase: DictationPhase::Listening { started_at: AT_MS + 180, ready: true, live: Some(live_text()), locked: false },
            session: 7,
            context: None,
            kind: TakeKind::Dictation,
            remote: None,
            preset: None,
            source: Some(voltip_core::RecordingSource::Mixed),
            segments: Some(voltip_core::dictation::SegmentProgress { done: 5, total: 6 }),
            recording: None,
            retake: false,
        }),
        UiEvent::Dictation(DictationStatus {
            phase: DictationPhase::Listening {
                started_at: AT_MS + 180,
                ready: true,
                live: Some(LiveText { degraded: Some("live tap overrun: the decoder fell behind the microphone".into()), ..live_text() }),
                locked: false,
            },
            session: 7,
            context: None,
            kind: TakeKind::Dictation,
            remote: None,
            preset: None,
            source: None,
            segments: None,
            recording: None,
            retake: false,
        }),
        // `hold_or_toggle` locked by a short press (docs/dictation.md §13): the pill shows a lock.
        UiEvent::Dictation(DictationStatus {
            phase: DictationPhase::Listening { started_at: AT_MS + 180, ready: true, live: None, locked: true },
            session: 7,
            context: None,
            kind: TakeKind::Dictation,
            remote: None,
            preset: None,
            source: None,
            segments: None,
            recording: None,
            retake: false,
        }),
        // `live_inject` (docs/dictation.md §12): the first committed sentence is already pasted.
        UiEvent::Dictation(DictationStatus {
            phase: DictationPhase::Listening { started_at: AT_MS + 180, ready: true, live: Some(LiveText { injected: 1, ..live_text() }), locked: false },
            session: 7,
            context: None,
            kind: TakeKind::Dictation,
            remote: None,
            preset: None,
            source: None,
            segments: None,
            recording: None,
            retake: false,
        }),
        // The streaming modes wait for the flush first (§12 `finalizing`).
        UiEvent::Dictation(DictationStatus {
            phase: DictationPhase::Processing {
                stage: ProcessingStage::Finalizing,
                started_at: AT_MS + 3200,
                stage_started_at: AT_MS + 3200,
                preview: Some(live_text().preview()),
            },
            session: 7,
            context: None,
            kind: TakeKind::Dictation,
            remote: None,
            preset: None,
            source: None,
            segments: None,
            recording: None,
            retake: false,
        }),
        UiEvent::Dictation(DictationStatus {
            phase: DictationPhase::Processing {
                stage: ProcessingStage::Transcribing,
                started_at: AT_MS + 3200,
                stage_started_at: AT_MS + 3200,
                preview: Some(live_text().preview()),
            },
            session: 7,
            context: None,
            kind: TakeKind::Dictation,
            remote: None,
            preset: None,
            source: None,
            segments: None,
            recording: None,
            retake: false,
        }),
        UiEvent::Dictation(DictationStatus {
            phase: DictationPhase::Processing { stage: ProcessingStage::Transcribing, started_at: AT_MS + 3200, stage_started_at: AT_MS + 3200, preview: None },
            session: 7,
            context: None,
            kind: TakeKind::Dictation,
            remote: None,
            preset: None,
            source: None,
            segments: None,
            recording: None,
            retake: false,
        }),
        UiEvent::Dictation(DictationStatus {
            phase: DictationPhase::Processing {
                stage: ProcessingStage::Refining,
                started_at: AT_MS + 3200,
                stage_started_at: AT_MS + 4100,
                preview: Some(live_text().preview()),
            },
            session: 7,
            context: None,
            kind: TakeKind::Dictation,
            remote: None,
            // docs/dictation.md §21: the pill names the preset while refining.
            preset: Some(PresetRef { id: PresetId::Custom(uuid(PRESET_ID)), name: "周报".into() }),
            source: None,
            segments: None,
            recording: None,
            retake: false,
        }),
        UiEvent::Dictation(DictationStatus {
            phase: DictationPhase::Processing { stage: ProcessingStage::Inserting, started_at: AT_MS + 3200, stage_started_at: AT_MS + 3200, preview: None },
            session: 7,
            context: None,
            kind: TakeKind::Dictation,
            remote: None,
            preset: None,
            source: None,
            segments: None,
            recording: None,
            retake: false,
        }),
        UiEvent::Dictation(DictationStatus {
            phase: done_phase(),
            session: 7,
            context: None,
            kind: TakeKind::Dictation,
            remote: None,
            preset: None,
            source: None,
            segments: None,
            recording: None,
            retake: false,
        }),
        UiEvent::Dictation(DictationStatus {
            phase: DictationPhase::Done {
                text: RAW_TEXT.into(),
                raw_text: RAW_TEXT.into(),
                chars: RAW_TEXT.chars().count(),
                via: Via::Clipboard,
                refined: false,
                duration_ms: 3200,
                asr_ms: 640,
                refine_ms: None,
                refine_failure: Some(RefineFailure::RateLimited),
                refine_error: Some("请求过于频繁：refine service rate limited (retry after 355000 ms)".into()),
                mode: OutputMode::WholeTake,
                segments: None,
                live_error: None,
                // docs/dictation.md §14.3: another window came to the front, so the text was copied.
                clipboard_code: Some(ClipboardCode::TargetChanged),
                check: None,
            },
            session: 8,
            context: None,
            kind: TakeKind::Dictation,
            remote: None,
            preset: None,
            source: None,
            segments: None,
            recording: None,
            retake: false,
        }),
        // docs/dictation.md §28: the insert check found the field as it was — the 没有写进去 card.
        UiEvent::Dictation(DictationStatus {
            phase: DictationPhase::Done {
                text: RAW_TEXT.into(),
                raw_text: RAW_TEXT.into(),
                chars: RAW_TEXT.chars().count(),
                via: Via::Paste,
                refined: false,
                duration_ms: 3200,
                asr_ms: 640,
                refine_ms: None,
                refine_failure: None,
                refine_error: None,
                mode: OutputMode::WholeTake,
                segments: None,
                live_error: None,
                clipboard_code: None,
                check: Some(InsertCheck::NotWritten),
            },
            session: 9,
            context: None,
            kind: TakeKind::Dictation,
            remote: None,
            preset: None,
            source: None,
            segments: None,
            recording: None,
            retake: false,
        }),
        // `streaming_final`: the stream's sentences are the text; a streaming mode that fell back to
        // the whole take says why in `live_error` (docs/dictation.md §12).
        UiEvent::Dictation(DictationStatus {
            phase: streamed_done_phase(),
            session: 14,
            context: None,
            kind: TakeKind::Dictation,
            remote: None,
            preset: None,
            source: None,
            segments: None,
            recording: None,
            retake: false,
        }),
        UiEvent::Dictation(DictationStatus {
            phase: fallen_back_done_phase(),
            session: 15,
            context: None,
            kind: TakeKind::Dictation,
            remote: None,
            preset: None,
            source: None,
            segments: None,
            recording: None,
            retake: false,
        }),
        UiEvent::Dictation(DictationStatus {
            phase: DictationPhase::Failed { code: FailureCode::NoSpeech, message: "没有听到声音".into(), text: None },
            session: 9,
            context: None,
            kind: TakeKind::Dictation,
            remote: None,
            preset: None,
            source: None,
            segments: None,
            recording: None,
            retake: false,
        }),
        UiEvent::Dictation(DictationStatus {
            phase: DictationPhase::Failed { code: FailureCode::Inject, message: "inject: 前台窗口拒绝了粘贴".into(), text: Some(REFINED_TEXT.into()) },
            session: 10,
            context: None,
            kind: TakeKind::Dictation,
            remote: None,
            preset: None,
            source: None,
            segments: None,
            recording: None,
            retake: false,
        }),
        UiEvent::Dictation(DictationStatus {
            phase: DictationPhase::Failed { code: FailureCode::Asr, message: "asr: 401 unauthorized".into(), text: None },
            session: 12,
            context: None,
            kind: TakeKind::Dictation,
            remote: None,
            preset: None,
            source: None,
            segments: None,
            recording: None,
            retake: false,
        }),
        UiEvent::Dictation(DictationStatus {
            phase: DictationPhase::Failed { code: FailureCode::Audio, message: "audio: no input device".into(), text: None },
            session: 13,
            context: None,
            kind: TakeKind::Dictation,
            remote: None,
            preset: None,
            source: None,
            segments: None,
            recording: None,
            retake: false,
        }),
        UiEvent::Dictation(DictationStatus {
            phase: DictationPhase::CANCELLED,
            session: 11,
            context: None,
            kind: TakeKind::Dictation,
            remote: None,
            preset: None,
            source: None,
            segments: None,
            recording: None,
            retake: false,
        }),
        // `live_inject` cancelled after one sentence was pasted: it stays pasted (§12).
        UiEvent::Dictation(DictationStatus {
            phase: DictationPhase::Cancelled { injected_chars: LIVE_COMMITTED.chars().count() },
            session: 16,
            context: None,
            kind: TakeKind::Dictation,
            remote: None,
            preset: None,
            source: None,
            segments: None,
            recording: None,
            retake: false,
        }),
        // docs/dictation.md §18.6: the probe named the app and a scene matched (the pill shows it);
        // an app no scene names carries no `scene`.
        UiEvent::Dictation(DictationStatus {
            phase: DictationPhase::Listening { started_at: AT_MS, ready: false, live: None, locked: false },
            session: 17,
            context: Some(take_context()),
            kind: TakeKind::Dictation,
            remote: None,
            preset: None,
            source: None,
            segments: None,
            recording: None,
            retake: false,
        }),
        UiEvent::Dictation(DictationStatus {
            phase: DictationPhase::Processing { stage: ProcessingStage::Transcribing, started_at: AT_MS + 3200, stage_started_at: AT_MS + 3200, preview: None },
            session: 18,
            context: Some(TakeContext { app: AppRef { id: "winword".into(), name: "WINWORD".into() }, scene: None }),
            kind: TakeKind::Dictation,
            remote: None,
            preset: None,
            source: None,
            segments: None,
            recording: None,
            retake: false,
        }),
        history_event(history_entries()),
        history_event(vec![live_inject_history_entry()]),
        history_event(vec![HistoryEntry { outcome: Outcome::Failed { reason: "inject: 前台窗口拒绝了粘贴".into() }, ..history_entries().remove(0) }]),
        // docs/dictation.md §4.2: a clipboard fallback with its code.
        history_event(vec![HistoryEntry {
            outcome: Outcome::Clipboard {
                reason: "enigo: the application does not have the permission to simulate input".into(),
                code: Some(ClipboardCode::NoPermission),
            },
            ..history_entries().remove(1)
        }]),
        // docs/dictation.md §22: a long take processed with a preset keeps the result beside its text.
        history_event(vec![HistoryEntry { processed: Some(Box::new(processed_text())), ..history_entries().remove(0) }]),
        // docs/dictation.md §22: a long take's text past 5000 characters waits on the clipboard.
        history_event(vec![HistoryEntry {
            outcome: Outcome::Clipboard { reason: voltip_core::dictation::long::TOO_LONG_TO_PASTE.into(), code: Some(ClipboardCode::TooLong) },
            ..history_entries().remove(1)
        }]),
        history_event(Vec::new()),
        UiEvent::Engines(engine_status()),
        UiEvent::Engines(EngineStatus { live_preview_ready: false, ..engine_status() }),
        // A streaming output mode in effect (§12): the preview is ready and the setting asks for it.
        UiEvent::Engines(EngineStatus { effective_output_mode: OutputMode::LiveInject, ..engine_status() }),
        UiEvent::Engines(EngineStatus::default()),
        // Local mode: `asr_host` reads `local`, the token is absent, readiness comes from the library.
        UiEvent::Engines(local_engine_status(true)),
        UiEvent::Engines(local_engine_status(false)),
        // The engines pane's 测试连接 (docs/dictation.md §3.3): a model list, and a refusal.
        UiEvent::ProviderProbe(ProbeReport {
            provider: ProviderId::Groq,
            kind: ServiceKind::Llm,
            outcome: ProbeOutcome::Ok { models: vec!["llama-3.3-70b-versatile".into(), "openai/gpt-oss-20b".into()], latency_ms: 184 },
        }),
        UiEvent::ProviderProbe(ProbeReport {
            provider: ProviderId::Builtin,
            kind: ServiceKind::Asr,
            outcome: ProbeOutcome::Failed { reason: ProbeFailure::HttpStatus, status: Some(502), detail: None },
        }),
        // docs/dictation.md §3.12: what the service said goes with the reason.
        UiEvent::ProviderProbe(ProbeReport {
            provider: ProviderId::Openai,
            kind: ServiceKind::Llm,
            outcome: ProbeOutcome::Failed {
                reason: ProbeFailure::Unauthorized,
                status: None,
                detail: Some("Incorrect API key provided: sk-***. (invalid_api_key)".into()),
            },
        }),
        // What the local engines can run on (§10.6): a CPU-only build and one with GPUs.
        UiEvent::Hardware(HardwareStatus { cpu_threads: 8, gpus: Vec::new() }),
        UiEvent::Hardware(hardware_status()),
        // The phone as microphone (docs/dictation.md §20): every take state the phone shows.
        UiEvent::PhoneTake { take: None },
        phone_take_event(PhoneTakeState::Starting),
        phone_take_event(PhoneTakeState::Listening),
        phone_take_event(PhoneTakeState::Processing),
        phone_take_event(PhoneTakeState::Done { text: "把 fetchUser 改成 async".into(), pasted: true }),
        phone_take_event(PhoneTakeState::Failed { code: PhoneTakeFailure::Busy, message: "正在听写".into(), text: None }),
        phone_take_event(PhoneTakeState::Failed { code: PhoneTakeFailure::Microphone, message: "microphone: 权限被拒绝".into(), text: None }),
        // Recognised on the computer, not inserted (2026-10-13): the text comes along.
        phone_take_event(PhoneTakeState::Failed {
            code: PhoneTakeFailure::Failed,
            message: "inject: keystroke: refused".into(),
            text: Some("把 fetchUser 改成 async".into()),
        }),
        phone_take_event(PhoneTakeState::Cancelled),
        // Text the phone sent (§20.6): the list, empty and full.
        UiEvent::SentTexts { texts: Vec::new() },
        UiEvent::SentTexts { texts: sent_texts() },
        // The connectivity self-check (docs/pairing.md): running, then a report with every probe outcome.
        UiEvent::Connectivity(ConnectivityStatus { running: true, report: None }),
        UiEvent::Connectivity(ConnectivityStatus { running: false, report: Some(connectivity_report()) }),
    ];
    events.extend(failed_connectivity_reports().into_iter().map(|report| UiEvent::Connectivity(ConnectivityStatus { running: false, report: Some(report) })));
    events.extend([
        // docs/dictation.md §23.6: the listener being started, then one that could not start (every
        // key of the status present).
        UiEvent::Serve(ServeStatus { available: true, phase: ServePhase::Starting, address: None, error: None }),
        UiEvent::Serve(ServeStatus {
            available: true,
            phase: ServePhase::Failed,
            address: Some("http://127.0.0.1:47840/v1".into()),
            error: Some("无法监听 127.0.0.1:47840：Address already in use".into()),
        }),
        // docs/dictation.md §24: a password saved and the system's proxy; then neither.
        UiEvent::Network(NetworkStatus { proxy_password: true, system_proxy: Some("socks5h://127.0.0.1:1080".into()) }),
        UiEvent::Network(NetworkStatus::default()),
        // docs/dictation.md §25: the MCP service starting, failing (every key of the status), running
        // with its activity, then off; and the phone, where it is not available.
        UiEvent::Mcp(McpStatus { available: true, phase: ServePhase::Starting, ..McpStatus::default() }),
        UiEvent::Mcp(McpStatus {
            available: true,
            phase: ServePhase::Failed,
            address: Some("http://127.0.0.1:47841/mcp".into()),
            error: Some("无法监听 127.0.0.1:47841：Address already in use".into()),
            clients: Vec::new(),
            activity: Vec::new(),
        }),
        UiEvent::Mcp(mcp_status()),
        UiEvent::Mcp(McpStatus { available: true, ..McpStatus::default() }),
        UiEvent::Mcp(McpStatus::default()),
        // The model library: every `ModelInstallState` variant across these three lists.
        models_event(models_installed_and_downloading()),
        models_event(models_other_states()),
        models_event(models_package_failed()),
        models_event(models_not_installed()),
        models_event(Vec::new()),
        // Every updater state the settings page discriminates on, in the order the shell publishes
        // them (check → download → ready → install), plus the two terminal ones.
        UiEvent::Update(UpdateStatus::Idle),
        UiEvent::Update(UpdateStatus::Checking),
        UiEvent::Update(UpdateStatus::UpToDate { version: "0.0.1".into(), checked_at: TRUSTED_AT }),
        UiEvent::Update(UpdateStatus::Available {
            version: "2.1.0".into(),
            current: "0.0.1".into(),
            notes: Some("修复听写热键冲突".into()),
            date: Some("2026-09-25T08:00:00Z".into()),
        }),
        UiEvent::Update(UpdateStatus::Available { version: "2.1.0".into(), current: "0.0.1".into(), notes: None, date: None }),
        UiEvent::Update(UpdateStatus::Downloading { version: "2.1.0".into(), received: 4_194_304, total: Some(15_728_640) }),
        UiEvent::Update(UpdateStatus::Downloading { version: "2.1.0".into(), received: 4_194_304, total: None }),
        UiEvent::Update(UpdateStatus::Ready { version: "2.1.0".into() }),
        UiEvent::Update(UpdateStatus::Installing { version: "2.1.0".into() }),
        UiEvent::Update(UpdateStatus::Failed { message: "updater: 无法连接更新源".into() }),
        UiEvent::Update(UpdateStatus::Store { version: "0.0.1".into() }),
        UiEvent::Update(UpdateStatus::Disabled),
        // The vocabulary lists (docs/dictation.md §16.4), full and empty.
        dictionary_event(dictionary()),
        dictionary_event(Vec::new()),
        rules_event(rules()),
        rules_event(Vec::new()),
        // The pairs the learning suggestions leave out (docs/dictation.md §16.10), and none.
        UiEvent::VocabularyIgnored { pairs: ignored_pairs() },
        UiEvent::VocabularyIgnored { pairs: Vec::new() },
        // Voice edit (docs/dictation.md §19): the edit hotkey switched off, and an edit take in every
        // phase the pill discriminates on, with the four refusals.
        UiEvent::Settings(Settings { edit_hotkey: None, ..settings() }),
        UiEvent::Dictation(DictationStatus {
            phase: DictationPhase::Listening {
                started_at: AT_MS + 180,
                ready: true,
                live: Some(LiveText { committed: Vec::new(), current: EDIT_INSTRUCTION.into(), degraded: None, injected: 0 }),
                locked: false,
            },
            session: 20,
            context: None,
            kind: TakeKind::Edit,
            remote: None,
            preset: None,
            source: None,
            segments: None,
            recording: None,
            retake: false,
        }),
        UiEvent::Dictation(DictationStatus {
            phase: DictationPhase::Processing {
                stage: ProcessingStage::Refining,
                started_at: AT_MS + 1400,
                stage_started_at: AT_MS + 1400,
                preview: Some(EDIT_INSTRUCTION.into()),
            },
            session: 20,
            context: None,
            kind: TakeKind::Edit,
            remote: None,
            preset: None,
            source: None,
            segments: None,
            recording: None,
            retake: false,
        }),
        UiEvent::Dictation(DictationStatus {
            phase: DictationPhase::Done {
                text: EDIT_REWRITE.into(),
                raw_text: EDIT_INSTRUCTION.into(),
                chars: EDIT_REWRITE.chars().count(),
                via: Via::Paste,
                refined: true,
                duration_ms: 1400,
                asr_ms: 380,
                refine_ms: Some(900),
                refine_failure: None,
                refine_error: None,
                mode: OutputMode::WholeTake,
                segments: None,
                live_error: None,
                clipboard_code: None,
                check: None,
            },
            session: 20,
            context: Some(TakeContext { app: AppRef { id: "slack".into(), name: "Slack".into() }, scene: None }),
            kind: TakeKind::Edit,
            remote: None,
            preset: None,
            source: None,
            segments: None,
            recording: None,
            retake: false,
        }),
        UiEvent::Dictation(DictationStatus {
            phase: DictationPhase::Failed { code: FailureCode::NoSelection, message: "没有选中文本".into(), text: None },
            session: 21,
            context: None,
            kind: TakeKind::Edit,
            remote: None,
            preset: None,
            source: None,
            segments: None,
            recording: None,
            retake: false,
        }),
        UiEvent::Dictation(DictationStatus {
            phase: DictationPhase::Failed {
                code: FailureCode::SelectionTooLong, message: "选中文本过长：2400 字（上限 2000 字）".into(), text: None
            },
            session: 22,
            context: None,
            kind: TakeKind::Edit,
            remote: None,
            preset: None,
            source: None,
            segments: None,
            recording: None,
            retake: false,
        }),
        UiEvent::Dictation(DictationStatus {
            phase: DictationPhase::Failed {
                code: FailureCode::Selection, message: "selection: keystroke: no copy tool on Wayland · GNOME".into(), text: None
            },
            session: 23,
            context: None,
            kind: TakeKind::Edit,
            remote: None,
            preset: None,
            source: None,
            segments: None,
            recording: None,
            retake: false,
        }),
        UiEvent::Dictation(DictationStatus {
            phase: DictationPhase::Failed {
                code: FailureCode::EditUnavailable,
                message: "edit: 语音编辑需要 AI 润色服务：请先配置润色的 API 密钥".into(),
                text: None,
            },
            session: 24,
            context: None,
            kind: TakeKind::Edit,
            remote: None,
            preset: None,
            source: None,
            segments: None,
            recording: None,
            retake: false,
        }),
        // §19.2: a terminal in front — refused before the copy chord and the microphone.
        UiEvent::Dictation(DictationStatus {
            phase: DictationPhase::Failed {
                code: FailureCode::EditInTerminal, message: "终端里不支持语音编辑：终端里的选区不能被替换".into(), text: None
            },
            session: 25,
            context: None,
            kind: TakeKind::Edit,
            remote: None,
            preset: None,
            source: None,
            segments: None,
            recording: None,
            retake: false,
        }),
        // The scenes (docs/dictation.md §18.6), full and empty.
        scenes_event(scenes()),
        scenes_event(Vec::new()),
        presets_event(custom_presets()),
        presets_event(Vec::new()),
        // 试一试 (docs/dictation.md §21): a text, and a refusal without an AI service.
        UiEvent::PresetTry {
            id: 3,
            outcome: PresetTryOutcome::Ok { text: "Meeting at 10 a.m. tomorrow.".into(), latency_ms: 820, model: REFINE_MODEL.into() },
        },
        UiEvent::PresetTry { id: 4, outcome: PresetTryOutcome::Failed { reason: voltip_core::PRESET_TRY_UNCONFIGURED.into() } },
        // 用 AI 预设处理 (docs/dictation.md §22): progress, the stored result, a failure, a cancel.
        UiEvent::HistoryProcess { request_id: 5, id: uuid(HISTORY_ID), state: ProcessState::Running { done: 2, total: 5 } },
        UiEvent::HistoryProcess { request_id: 5, id: uuid(HISTORY_ID), state: ProcessState::Done { processed: processed_text() } },
        UiEvent::HistoryProcess {
            request_id: 6,
            id: uuid(HISTORY_ID),
            state: ProcessState::Failed { reason: voltip_core::history::process::PROCESS_UNCONFIGURED.into() },
        },
        UiEvent::HistoryProcess { request_id: 7, id: uuid(HISTORY_ID), state: ProcessState::Cancelled },
        // docs/dictation.md §26: the kept recordings, and the phone's (none kept there).
        UiEvent::AudioCache(audio_cache()),
        UiEvent::AudioCache(AudioCacheView::default()),
        // docs/dictation.md §31: the import queue, a file that nothing was said in, the phone (no
        // imports), and an imported file's entry.
        UiEvent::Imports(imports_view()),
        UiEvent::Imports(ImportsView {
            available: true,
            extensions: vec!["wav".into()],
            jobs: vec![ImportJob {
                id: uuid(IMPORT_IDS[0]),
                name: "静音.wav".into(),
                size: 1_024,
                duration_ms: Some(2_000),
                paused: false,
                state: ImportState::Failed { failure: ImportFailure::NoSpeech, resumable: false },
            }],
        }),
        UiEvent::Imports(ImportsView::default()),
        history_event(vec![imported_history_entry()]),
        // docs/dictation.md §4.6.1: the automatic backups in a folder of the user's, the default
        // folder before it was looked at, and the phone (none made there).
        UiEvent::AutoBackup(auto_backup()),
        UiEvent::AutoBackup(AutoBackupView { folder: auto_backup().default_folder, custom: false, last: None, failure: None, notice: None, ..auto_backup() }),
        UiEvent::AutoBackup(AutoBackupView::default()),
        // docs/dictation.md §10.9: a move halfway, every way one ends, and the phone (no library).
        UiEvent::ModelsLocation(models_location()),
        UiEvent::ModelsLocation(ModelsLocationView { moving: None, last: Some(ModelsMoveOutcome::Cancelled), ..models_location() }),
        UiEvent::ModelsLocation(ModelsLocationView {
            moving: None,
            last: Some(ModelsMoveOutcome::Failed {
                message: "所选位置空间不足：移动模型需要 1031 MB 空闲空间，当前可用 512 MB".into()
            }),
            ..models_location()
        }),
        UiEvent::ModelsLocation(ModelsLocationView::default()),
    ]);
    events.extend(paste_results());
    events
}

/// The history's paste button (`voltip_core::paste`): every outcome with every reason, so the
/// TypeScript enums are checked against all the Rust names.
fn paste_results() -> Vec<UiEvent> {
    let copied = [CopyReason::NoProbe, CopyReason::Timeout, CopyReason::TargetChanged, CopyReason::ClipboardOnly, CopyReason::PasteFailed]
        .map(|reason| PasteOutcome::Copied { reason });
    let failed = [PasteFailure::Busy, PasteFailure::Invalid, PasteFailure::Timeout, PasteFailure::Inject, PasteFailure::Unsupported]
        .map(|reason| PasteOutcome::Failed { reason });
    std::iter::once(PasteOutcome::Pasted)
        .chain(copied)
        .chain(failed)
        .zip(1..)
        .map(|(outcome, request_id)| UiEvent::PasteResult { request_id, outcome })
        .collect()
}

/// Variant name for a parsed command; exhaustive so a new variant must be added to the fixture.
fn command_variant(cmd: &UiCommand) -> &'static str {
    match cmd {
        UiCommand::PairingStart => "PairingStart",
        UiCommand::PairingJoinCode { .. } => "PairingJoinCode",
        UiCommand::PairingJoinTicket { .. } => "PairingJoinTicket",
        UiCommand::PairingConfirm => "PairingConfirm",
        UiCommand::PairingReject => "PairingReject",
        UiCommand::PairingCancel => "PairingCancel",
        UiCommand::PairingReset => "PairingReset",
        UiCommand::DeviceForget { .. } => "DeviceForget",
        UiCommand::DeviceSyncSet { .. } => "DeviceSyncSet",
        UiCommand::DeviceRename { .. } => "DeviceRename",
        UiCommand::SendText { .. } => "SendText",
        UiCommand::SettingsSetRelay { .. } => "SettingsSetRelay",
        UiCommand::SettingsSetTheme { .. } => "SettingsSetTheme",
        UiCommand::SettingsSetHotkey { .. } => "SettingsSetHotkey",
        UiCommand::SettingsSetEditHotkey { .. } => "SettingsSetEditHotkey",
        UiCommand::SettingsSetCopyLastHotkey { .. } => "SettingsSetCopyLastHotkey",
        UiCommand::SettingsSetSoloKey { .. } => "SettingsSetSoloKey",
        UiCommand::SettingsSetMicrophone { .. } => "SettingsSetMicrophone",
        UiCommand::SettingsSetRecording { .. } => "SettingsSetRecording",
        UiCommand::SettingsSetLocale { .. } => "SettingsSetLocale",
        UiCommand::SettingsSetAutoUpdate { .. } => "SettingsSetAutoUpdate",
        UiCommand::SettingsSetHistory { .. } => "SettingsSetHistory",
        UiCommand::SettingsSetAudioCache { .. } => "SettingsSetAudioCache",
        UiCommand::SettingsSetBackup { .. } => "SettingsSetBackup",
        UiCommand::BackupFolderReset => "BackupFolderReset",
        UiCommand::BackupNoticeDismiss => "BackupNoticeDismiss",
        UiCommand::AudioCacheClear => "AudioCacheClear",
        UiCommand::AudioCacheDelete { .. } => "AudioCacheDelete",
        UiCommand::AudioRetake { .. } => "AudioRetake",
        UiCommand::SettingsSetOverlay { .. } => "SettingsSetOverlay",
        UiCommand::PhoneTakeStart { .. } => "PhoneTakeStart",
        UiCommand::PhoneTakeStop => "PhoneTakeStop",
        UiCommand::PhoneTakeCancel => "PhoneTakeCancel",
        UiCommand::PhoneTextSend { .. } => "PhoneTextSend",
        UiCommand::SentTextsClear => "SentTextsClear",
        UiCommand::SettingsSetPairingAlwaysOn { .. } => "SettingsSetPairingAlwaysOn",
        UiCommand::RelayReconnect => "RelayReconnect",
        UiCommand::DevicesRefresh => "DevicesRefresh",
        UiCommand::ConnectivityCheck => "ConnectivityCheck",
        UiCommand::DictationStart => "DictationStart",
        UiCommand::DictationStop => "DictationStop",
        UiCommand::DictationCancel => "DictationCancel",
        UiCommand::DictationDismiss { .. } => "DictationDismiss",
        UiCommand::HotkeyEdge { .. } => "HotkeyEdge",
        UiCommand::SettingsSetActivation { .. } => "SettingsSetActivation",
        UiCommand::SettingsSetEngines { .. } => "SettingsSetEngines",
        UiCommand::ProviderKeySet { .. } => "ProviderKeySet",
        UiCommand::EnginesQuotaReset { .. } => "EnginesQuotaReset",
        UiCommand::ProviderProbe { .. } => "ProviderProbe",
        UiCommand::ProviderModelsRefresh { .. } => "ProviderModelsRefresh",
        UiCommand::HistoryDelete { .. } => "HistoryDelete",
        UiCommand::HistoryClear => "HistoryClear",
        UiCommand::HistoryStar { .. } => "HistoryStar",
        UiCommand::HistoryProcess { .. } => "HistoryProcess",
        UiCommand::HistoryProcessCancel { .. } => "HistoryProcessCancel",
        UiCommand::ModelDownload { .. } => "ModelDownload",
        UiCommand::ModelCancel { .. } => "ModelCancel",
        UiCommand::ModelRemove { .. } => "ModelRemove",
        UiCommand::ModelImport { .. } => "ModelImport",
        UiCommand::ModelsLocationReset => "ModelsLocationReset",
        UiCommand::ModelsLocationCancel => "ModelsLocationCancel",
        UiCommand::DictionaryAdd { .. } => "DictionaryAdd",
        UiCommand::DictionaryUpdate { .. } => "DictionaryUpdate",
        UiCommand::DictionaryRemove { .. } => "DictionaryRemove",
        UiCommand::DictionaryReorder { .. } => "DictionaryReorder",
        UiCommand::VocabularySuggestionIgnore { .. } => "VocabularySuggestionIgnore",
        UiCommand::VocabularySuggestionUnignore { .. } => "VocabularySuggestionUnignore",
        UiCommand::RulesAdd { .. } => "RulesAdd",
        UiCommand::RulesUpdate { .. } => "RulesUpdate",
        UiCommand::RulesRemove { .. } => "RulesRemove",
        UiCommand::RulesReorder { .. } => "RulesReorder",
        UiCommand::RulesImport { .. } => "RulesImport",
        UiCommand::ScenesAdd { .. } => "ScenesAdd",
        UiCommand::ScenesUpdate { .. } => "ScenesUpdate",
        UiCommand::ScenesRemove { .. } => "ScenesRemove",
        UiCommand::ScenesReorder { .. } => "ScenesReorder",
        UiCommand::ScenesRestore { .. } => "ScenesRestore",
        UiCommand::PresetsAdd { .. } => "PresetsAdd",
        UiCommand::PresetsUpdate { .. } => "PresetsUpdate",
        UiCommand::PresetsRemove { .. } => "PresetsRemove",
        UiCommand::PresetsTry { .. } => "PresetsTry",
        UiCommand::SettingsSetContextSharing { .. } => "SettingsSetContextSharing",
        UiCommand::SettingsSetPinnedScene { .. } => "SettingsSetPinnedScene",
        UiCommand::SettingsSetPhoneTalk { .. } => "SettingsSetPhoneTalk",
        UiCommand::SettingsSetServe { .. } => "SettingsSetServe",
        UiCommand::ServeCopyToken => "ServeCopyToken",
        UiCommand::ServeRotateToken => "ServeRotateToken",
        UiCommand::SettingsSetMcp { .. } => "SettingsSetMcp",
        UiCommand::McpCopyToken => "McpCopyToken",
        UiCommand::McpRotateToken => "McpRotateToken",
        UiCommand::McpCopySetup { .. } => "McpCopySetup",
        UiCommand::McpUndo { .. } => "McpUndo",
        UiCommand::SettingsSetProxy { .. } => "SettingsSetProxy",
        UiCommand::NetworkRefresh => "NetworkRefresh",
        UiCommand::RefineNoticeClose => "RefineNoticeClose",
        UiCommand::UsageRefresh => "UsageRefresh",
        UiCommand::UsageNoticeDismiss { .. } => "UsageNoticeDismiss",
        UiCommand::ImportAdd { .. } => "ImportAdd",
        UiCommand::ImportCancel { .. } => "ImportCancel",
        UiCommand::ImportRetry { .. } => "ImportRetry",
        UiCommand::ImportRemove { .. } => "ImportRemove",
    }
}

/// `(tauri command name, args object exactly as `TauriBackend.invoke` sends it, expected variant)`.
fn all_commands() -> Vec<(&'static str, Value, &'static str)> {
    vec![
        ("pairing_start", Value::Null, "PairingStart"),
        ("pairing_join_code", json!({ "code": "483 921" }), "PairingJoinCode"),
        ("pairing_join_ticket", json!({ "uri": "voltip://pair?v=1&t=QUJDREVGR0g" }), "PairingJoinTicket"),
        ("pairing_confirm", Value::Null, "PairingConfirm"),
        ("pairing_reject", Value::Null, "PairingReject"),
        ("pairing_cancel", Value::Null, "PairingCancel"),
        ("pairing_reset", Value::Null, "PairingReset"),
        ("device_forget", json!({ "publicKey": PHONE_KEY.to_hex() }), "DeviceForget"),
        // docs/dictation.md §20.8: the computer's switch per phone.
        ("device_sync_set", json!({ "publicKey": PHONE_KEY.to_hex(), "on": false }), "DeviceSyncSet"),
        ("device_rename", json!({ "name": "Studio" }), "DeviceRename"),
        ("send_text", json!({ "publicKey": PHONE_KEY.to_hex(), "body": "把 fetchUser 改成 async" }), "SendText"),
        ("settings_set_relay", json!({ "url": RELAY_URL, "enabled": true }), "SettingsSetRelay"),
        ("settings_set_theme", json!({ "theme": "graphite", "followSystem": true }), "SettingsSetTheme"),
        ("settings_set_hotkey", json!({ "hotkey": "Ctrl+Alt+Space" }), "SettingsSetHotkey"),
        // docs/dictation.md §19: the voice-edit hotkey (`null` switches it off).
        ("settings_set_edit_hotkey", json!({ "hotkey": "Ctrl+Alt+Shift+E" }), "SettingsSetEditHotkey"),
        // docs/dictation.md §28: the 复制上一条结果 hotkey (`null` assigns none).
        ("settings_set_copy_last_hotkey", json!({ "hotkey": "Ctrl+Shift+V" }), "SettingsSetCopyLastHotkey"),
        // docs/dictation.md §13.1: the lone-key trigger (`null` switches it off).
        ("settings_set_solo_key", json!({ "key": "mouse_back" }), "SettingsSetSoloKey"),
        ("settings_set_microphone", json!({ "device": "wasapi:{0.0.1.00000000}.{c2}" }), "SettingsSetMicrophone"),
        // docs/dictation.md §22: the source, the output device, the longest length of a take and the
        // echo cancellation of a mixed one (§22.6).
        (
            "settings_set_recording",
            json!({ "recording": { "source": "mixed", "output_device": "wasapi:{0.0.0.00000000}.{a1}", "max_minutes": 60, "echo_cancel": false } }),
            "SettingsSetRecording",
        ),
        ("settings_set_locale", json!({ "locale": "en" }), "SettingsSetLocale"),
        ("settings_set_auto_update", json!({ "enabled": true }), "SettingsSetAutoUpdate"),
        ("settings_set_history", json!({ "enabled": false, "keep": 100 }), "SettingsSetHistory"),
        // docs/dictation.md §26: the audio cache.
        ("settings_set_audio_cache", json!({ "enabled": true, "keep": 50, "maxMb": 500 }), "SettingsSetAudioCache"),
        ("audio_cache_clear", Value::Null, "AudioCacheClear"),
        ("audio_cache_delete", json!({ "id": RECORDING_ID_2 }), "AudioCacheDelete"),
        // docs/dictation.md §4.6.1: the weekly backup, the default folder again, home's notice closed.
        ("settings_set_backup", json!({ "weekly": false }), "SettingsSetBackup"),
        ("backup_folder_reset", Value::Null, "BackupFolderReset"),
        ("backup_notice_dismiss", Value::Null, "BackupNoticeDismiss"),
        // §26.4: on other models than the settings', for that take only.
        (
            "audio_retake",
            json!({ "id": RECORDING_ID_2, "asr": { "kind": "remote", "provider": "custom", "model": "whisper-2" }, "refine": { "kind": "off" } }),
            "AudioRetake",
        ),
        ("settings_set_overlay", json!({ "placement": "top" }), "SettingsSetOverlay"),
        ("phone_take_start", json!({ "publicKey": DESKTOP_KEY.to_hex() }), "PhoneTakeStart"),
        ("phone_take_stop", Value::Null, "PhoneTakeStop"),
        ("phone_take_cancel", Value::Null, "PhoneTakeCancel"),
        // docs/dictation.md §20.6: text from the phone, and forgetting the list.
        ("phone_text_send", json!({ "publicKey": DESKTOP_KEY.to_hex(), "body": "会议改到三点", "source": "typed" }), "PhoneTextSend"),
        ("sent_texts_clear", Value::Null, "SentTextsClear"),
        // The network changed: check the relay socket now (docs/pairing.md 「重连」).
        ("relay_reconnect", Value::Null, "RelayReconnect"),
        // Always-on pairing (docs/pairing.md 「常开配对」).
        ("settings_set_pairing_always_on", json!({ "enabled": true }), "SettingsSetPairingAlwaysOn"),
        ("devices_refresh", Value::Null, "DevicesRefresh"),
        ("connectivity_check", Value::Null, "ConnectivityCheck"),
        ("dictation_start", Value::Null, "DictationStart"),
        ("dictation_stop", Value::Null, "DictationStop"),
        ("dictation_cancel", Value::Null, "DictationCancel"),
        // docs/dictation.md §28: the result card's 关闭, for the take whose card it is.
        ("dictation_dismiss", json!({ "session": 3 }), "DictationDismiss"),
        // The activation machine's inputs (docs/dictation.md §13): one CLI press — of the voice-edit
        // key (`voltip --edit-toggle`, §19) — and one mode change.
        ("hotkey_edge", json!({ "pressed": true, "atMs": AT_MS, "source": "cli", "purpose": "edit" }), "HotkeyEdge"),
        ("settings_set_activation", json!({ "activation": "hold_or_toggle", "holdThresholdMs": 300, "extraRecordingMs": 150 }), "SettingsSetActivation"),
        // One sample per command (the TS contract test replays each once). The on-device shape
        // carries the local fields (`local_model`, `local_device`, `local_gpu`, `local_threads`);
        // the provider shape is the `settings` event.
        ("settings_set_engines", json!({ "engines": local_engine_settings() }), "SettingsSetEngines"),
        ("provider_key_set", json!({ "provider": "groq", "kind": "llm", "value": "gsk_example_not_a_real_key" }), "ProviderKeySet"),
        ("provider_probe", json!({ "provider": "custom", "kind": "asr", "baseUrl": "http://192.168.1.20:8000/v1", "key": null }), "ProviderProbe"),
        // A model list opens (docs/dictation.md §3.3).
        ("provider_models_refresh", json!({ "provider": "aliyun", "kind": "asr" }), "ProviderModelsRefresh"),
        // 重新检查 on the fallback models (docs/dictation.md §3.5).
        ("engines_quota_reset", json!({ "kind": "asr" }), "EnginesQuotaReset"),
        ("history_delete", json!({ "id": HISTORY_ID }), "HistoryDelete"),
        ("history_clear", Value::Null, "HistoryClear"),
        ("history_star", json!({ "id": HISTORY_ID, "starred": true }), "HistoryStar"),
        // 用 AI 预设处理 (docs/dictation.md §22).
        ("history_process", json!({ "requestId": 5, "id": HISTORY_ID, "preset": "notes" }), "HistoryProcess"),
        ("history_process_cancel", json!({ "requestId": 5 }), "HistoryProcessCancel"),
        ("model_download", json!({ "id": SENSE_VOICE_ID }), "ModelDownload"),
        ("model_cancel", json!({ "id": SENSE_VOICE_ID }), "ModelCancel"),
        ("model_remove", json!({ "id": PARAFORMER_ID }), "ModelRemove"),
        ("model_import", json!({ "id": SENSE_VOICE_ID }), "ModelImport"),
        // docs/dictation.md §10.9: back to the default folder, and stopping a move.
        ("models_location_reset", Value::Null, "ModelsLocationReset"),
        ("models_location_cancel", Value::Null, "ModelsLocationCancel"),
        // The vocabulary (docs/dictation.md §16.4): the add from a history entry carries every key.
        (
            "dictionary_add",
            json!({ "entry": DictionaryDraft { term: "good idea".into(), heard_as: vec!["谷歌IDR".into()], enabled: true }, "historyId": HISTORY_ID }),
            "DictionaryAdd",
        ),
        (
            "dictionary_update",
            json!({ "id": DICT_ID, "entry": DictionaryDraft { term: "fetchUser".into(), heard_as: vec!["fetch user".into()], enabled: false } }),
            "DictionaryUpdate",
        ),
        ("dictionary_remove", json!({ "id": DICT_ID_2 }), "DictionaryRemove"),
        ("dictionary_reorder", json!({ "ids": [DICT_ID_2, DICT_ID] }), "DictionaryReorder"),
        ("vocabulary_suggestion_ignore", json!({ "heard": "提成", "polished": "集成" }), "VocabularySuggestionIgnore"),
        ("vocabulary_suggestion_unignore", json!({ "heard": "提成", "polished": "集成" }), "VocabularySuggestionUnignore"),
        (
            "rules_add",
            json!({ "rule": RuleDraft {
                name: "PR 编号".into(),
                kind: RuleKind::Regex,
                pattern: r"\bpr (\d+)".into(),
                replacement: "PR #$1".into(),
                case_sensitive: false,
                enabled: true,
            } }),
            "RulesAdd",
        ),
        (
            "rules_update",
            json!({ "id": RULE_ID, "rule": RuleDraft {
                name: "git push".into(),
                kind: RuleKind::Literal,
                pattern: "给他push".into(),
                replacement: "git push".into(),
                case_sensitive: true,
                enabled: false,
            } }),
            "RulesUpdate",
        ),
        ("rules_remove", json!({ "id": RULE_ID_2 }), "RulesRemove"),
        ("rules_reorder", json!({ "ids": [RULE_ID_2, RULE_ID] }), "RulesReorder"),
        (
            "rules_import",
            json!({ "toml": "version = 1\n\n[[rule]]\nname = \"git push\"\npattern = \"给他push\"\nreplacement = \"git push\"\n", "mode": ImportMode::Merge }),
            "RulesImport",
        ),
        // Scenes (docs/dictation.md §18.6): the add carries every key; the update a plain draft.
        ("scenes_add", json!({ "scene": SceneDraft::from(&scenes()[0]) }), "ScenesAdd"),
        ("scenes_update", json!({ "id": SCENE_ID_2, "scene": SceneDraft::from(&scenes()[1]) }), "ScenesUpdate"),
        ("scenes_remove", json!({ "id": SCENE_ID_2 }), "ScenesRemove"),
        ("scenes_reorder", json!({ "ids": [SCENE_ID_2, SCENE_ID, SCENE_ID_3] }), "ScenesReorder"),
        ("scenes_restore", json!({ "id": SCENE_ID_3 }), "ScenesRestore"),
        // Presets (docs/dictation.md §21): 试一试 on a saved preset and on the instruction being edited.
        ("presets_add", json!({ "preset": PresetDraft::from(&custom_presets()[0]) }), "PresetsAdd"),
        ("presets_update", json!({ "id": PRESET_ID, "preset": PresetDraft { name: "周报（短）".into(), prompt: "三句话以内。".into() } }), "PresetsUpdate"),
        ("presets_remove", json!({ "id": PRESET_ID }), "PresetsRemove"),
        ("presets_try", json!({ "id": 3, "preset": "translate", "prompt": null, "text": "明天上午十点开会" }), "PresetsTry"),
        ("settings_set_context_sharing", json!({ "appName": true, "windowTitle": false }), "SettingsSetContextSharing"),
        ("settings_set_pinned_scene", json!({ "id": "0f3f1a1e-8d4b-4c8e-9f7a-1c2d3e4f5a6b" }), "SettingsSetPinnedScene"),
        // The phone's talk button (docs/dictation.md §20.11).
        ("settings_set_phone_talk", json!({ "mode": "tap" }), "SettingsSetPhoneTalk"),
        // The local speech service (docs/dictation.md §23.6).
        ("settings_set_serve", json!({ "enabled": true, "port": 47840, "preset": "prompt", "scene": SCENE_ID }), "SettingsSetServe"),
        ("serve_copy_token", Value::Null, "ServeCopyToken"),
        ("serve_rotate_token", Value::Null, "ServeRotateToken"),
        // The local MCP service (docs/dictation.md §25).
        (
            "settings_set_mcp",
            json!({ "enabled": true, "port": 47841, "read": "full", "historyRange": "last_7_days", "write": "enable_rules" }),
            "SettingsSetMcp",
        ),
        ("mcp_copy_token", Value::Null, "McpCopyToken"),
        ("mcp_rotate_token", Value::Null, "McpRotateToken"),
        ("mcp_copy_setup", json!({ "client": "codex" }), "McpCopySetup"),
        ("mcp_undo", json!({ "id": MCP_ACTIVITY_ID }), "McpUndo"),
        // The proxy (docs/dictation.md §24): a manual one with a new password.
        (
            "settings_set_proxy",
            json!({ "mode": "manual", "server": "http://proxy.example.com:3128", "username": "alice", "noProxy": "corp.example.com,10.0.0.0/8", "password": "example-password" }),
            "SettingsSetProxy",
        ),
        ("network_refresh", Value::Null, "NetworkRefresh"),
        // docs/dictation.md §3.6.
        ("refine_notice_close", Value::Null, "RefineNoticeClose"),
        // docs/dictation.md §27.
        ("usage_refresh", Value::Null, "UsageRefresh"),
        ("usage_notice_dismiss", json!({ "notice": "asr_near" }), "UsageNoticeDismiss"),
        // docs/dictation.md §31: two files on Groq's Whisper, polished on Model Studio, in Chinese;
        // then one job's 取消, 重试 and 移除.
        (
            "import_add",
            json!({
                "paths": ["C:\\Users\\me\\Videos\\季度会议.mp4", "/home/me/播客.mp3"],
                "asr": { "kind": "remote", "provider": "groq", "model": "whisper-large-v3" },
                "refine": { "provider": "aliyun", "model": "qwen-plus" },
                "language": "zh"
            }),
            "ImportAdd",
        ),
        ("import_cancel", json!({ "id": IMPORT_IDS[0] }), "ImportCancel"),
        ("import_retry", json!({ "id": IMPORT_IDS[6] }), "ImportRetry"),
        ("import_remove", json!({ "id": IMPORT_IDS[5] }), "ImportRemove"),
    ]
}

/// Commands a shell answers itself, without a `UiCommand` (the desktop's recorder suspends the OS
/// hotkey registration; its updater checks / downloads / installs; the phone opens its share
/// sheet). They are part of the wire contract the webview sees, so they are in the fixture, but
/// they never reach the bridge.
fn shell_only_commands() -> Vec<(&'static str, Value)> {
    vec![
        ("hotkey_capture", json!({ "active": true })),
        ("update_check", Value::Null),
        ("update_install", Value::Null),
        // docs/dictation.md §20.7: the phone's share sheet.
        ("phone_share_text", json!({ "text": "今天下午三点开会。" })),
    ]
}

/// `printWidth` of the repository's `oxfmt` configuration.
const PRINT_WIDTH: usize = 100;

/// serde's pretty JSON in the layout `oxfmt` (prettier rules) keeps: arrays of scalars that fit on
/// one line are inline, everything else stays one item per line. Keeps the checked-in fixture both
/// byte-identical to this test's output and clean under `pnpm -r run format:check`.
fn oxfmt_style(pretty: &str) -> String {
    let lines: Vec<&str> = pretty.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let indent = line.len() - line.trim_start().len();
        if line.ends_with('[')
            && let Some(end) = (i + 1..lines.len()).find(|&j| lines[j].len() - lines[j].trim_start().len() == indent && lines[j].trim_start().starts_with(']'))
        {
            let items = &lines[i + 1..end];
            let scalar = !items.is_empty() && items.iter().all(|l| !l.trim().starts_with(['{', '[']) && !l.trim_end().ends_with(['{', '[']));
            let joined = format!("{line}{}{}", items.iter().map(|l| l.trim()).collect::<Vec<_>>().join(" "), lines[end].trim_start());
            if scalar && joined.len() <= PRINT_WIDTH {
                out.push(joined);
                i = end + 1;
                continue;
            }
        }
        out.push(line.to_owned());
        i += 1;
    }
    let mut text = out.join("\n");
    text.push('\n');
    text
}

fn pretty<T: Serialize>(value: &T) -> String {
    oxfmt_style(&serde_json::to_string_pretty(value).expect("fixture value serializes"))
}

#[test]
fn oxfmt_style_inlines_only_short_scalar_arrays() {
    let input = "{\n  \"a\": [\n    1,\n    2\n  ],\n  \"b\": [\n    {\n      \"c\": 1\n    }\n  ],\n  \"d\": []\n}\n";
    assert_eq!(oxfmt_style(input), "{\n  \"a\": [1, 2],\n  \"b\": [\n    {\n      \"c\": 1\n    }\n  ],\n  \"d\": []\n}\n");
    let long = format!("[\n  \"{}\",\n  \"{}\"\n]\n", "x".repeat(60), "y".repeat(60));
    assert_eq!(oxfmt_style(&long), long, "over the print width stays one item per line");
}

/// Git for Windows checks text files out with CRLF by default (`core.autocrlf=true`); the fixtures
/// are compared byte for byte, so `.gitattributes` pins LF for every text file. Without it the
/// three fixture tests failed on a native Windows checkout (2026-09-26).
#[test]
fn regression_text_files_check_out_with_lf_on_every_os() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.gitattributes");
    let attributes = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert!(attributes.lines().any(|l| l.split_whitespace().eq(["*", "text=auto", "eol=lf"])), "{attributes}");
    for name in [STATE_FILE, EVENTS_FILE, COMMANDS_FILE, BUILTIN_SCENES_FILE, MCP_SETUP_FILE, DIFF_FILE, VOCABULARY_SUGGESTIONS_FILE] {
        let fixture = std::fs::read_to_string(fixtures_dir().join(name)).unwrap();
        assert!(!fixture.contains('\r'), "{name} has CRLF line endings");
    }
}

/// Write in update mode; otherwise compare byte-for-byte and point at the first differing line.
fn check_fixture(name: &str, expected: &str) {
    let path = fixtures_dir().join(name);
    if std::env::var_os(UPDATE_ENV).is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, expected).unwrap();
        return;
    }
    let actual = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}; run {UPDATE_ENV}=1 cargo test -p voltip-tauri-bridge --test contract", path.display()));
    // A CRLF checkout differs on every line yet compares equal line by line: say so instead of
    // calling the fixture stale (regenerating it would not help).
    assert!(
        !actual.contains('\r') || actual.replace("\r\n", "\n") != expected,
        "{} was checked out with CRLF line endings; .gitattributes pins LF (re-checkout: git rm -r --cached . && git reset --hard)",
        path.display()
    );
    if actual != expected {
        let line = actual.lines().zip(expected.lines()).position(|(a, e)| a != e).map_or(actual.lines().count().min(expected.lines().count()) + 1, |i| i + 1);
        let got = actual.lines().nth(line - 1).unwrap_or("<eof>");
        let want = expected.lines().nth(line - 1).unwrap_or("<eof>");
        panic!(
            "{} is stale (first difference at line {line}):\n  fixture: {got}\n  rust:    {want}\nrun {UPDATE_ENV}=1 cargo test -p voltip-tauri-bridge --test contract and commit the result",
            path.display()
        );
    }
}

#[test]
fn state_fixture_matches_serde_output() {
    let state = full_state();
    let text = pretty(&state);
    check_fixture(STATE_FILE, &text);
    let back: UiState = serde_json::from_str(&text).unwrap();
    assert_eq!(back, state, "UiState must round-trip through its own JSON");
    let json: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(json["identity"]["public_key"], Value::String(DESKTOP_KEY.to_hex()), "keys are lower-case hex on the wire");
    assert_eq!(json["pairing"]["state"]["state"], "awaiting_verification");
    assert_eq!(json["devices"][0]["connection"], json!({ "state": "online" }));
    assert_eq!(json["dictation"]["phase"]["phase"], "done");
    assert_eq!(json["dictation"]["session"], 7);
    assert_eq!(json["history_recent"][1]["outcome"]["kind"], "clipboard");
    // Recognition on the built-in service (no host on the wire), clean-up on Groq with the user's key.
    assert_eq!(json["engines"]["asr_provider"], "builtin");
    assert_eq!(json["engines"]["asr_host"], "", "the built-in host is never shown");
    assert_eq!(json["engines"]["asr_ready"], true);
    assert_eq!((json["engines"]["llm_provider"].as_str(), json["engines"]["refine_host"].as_str()), (Some("groq"), Some("api.groq.com")));
    assert_eq!(json["engines"]["refine_model"], "openai/gpt-oss-20b");
    let providers = json["engines"]["providers"].as_array().unwrap();
    let card = |id: &str| providers.iter().find(|p| p["id"] == id).unwrap_or_else(|| panic!("{id} card"));
    assert_eq!(card("groq")["llm"]["key"], json!({ "set": true, "source": "user" }));
    assert_eq!(card("groq")["asr"]["key"], json!({ "set": true, "source": "user" }), "one key for both of a vendor's services");
    assert_eq!(card("builtin")["asr"]["key"], json!({ "set": true, "source": "builtin" }));
    assert!(card("builtin")["asr"].get("base_url").is_none(), "no endpoint for the built-in card");
    assert_eq!(card("custom")["asr"]["base_url"], "https://asr.example.test");
    assert_eq!(json["settings"]["engines"]["inject"], "paste");
    assert_eq!(json["settings"]["locale"], "zh-cn", "kebab-case on the wire");
    assert_eq!(json["settings"]["auto_update"], true);
    assert_eq!(json["update"]["state"], "available");
    assert_eq!(json["settings"]["engines"]["asr_provider"], "builtin");
    assert_eq!(json["engines"]["local_ready"], false);
    assert_eq!(json["engines"]["live_preview_ready"], true, "remote ASR with the streaming model installed previews");
    assert_eq!(json["settings"]["engines"]["live_preview"], true);
    assert!(json["engines"].get("local_model").is_none(), "None is omitted for remote recognition");
    assert_eq!(json["models"].as_array().map(Vec::len), Some(4), "the library is never empty in the sample state");
    assert_eq!(json["models"][0]["state"]["kind"], "installed");
    assert_eq!(json["models"][0]["active"], true);
    assert_eq!(json["models"][0]["tier"], "balanced");
    assert_eq!(json["models"][0]["capabilities"], json!(["offline"]));
    assert_eq!(json["models"][2]["state"], json!({ "kind": "downloading", "received": 104857600, "total": 227330205, "file": "model.int8.onnx" }));
    assert_eq!(json["models"][3]["capabilities"], json!(["streaming"]));
    assert_eq!(json["models"][3]["tier"], "streaming");
    assert_eq!(json["models"][3]["active"], false, "the streaming model is never the active (final-text) model");
    // docs/dictation.md §16 / §17.
    assert_eq!(json["dictionary"][1]["source"], json!({ "kind": "history", "history_id": HISTORY_ID }));
    assert_eq!(json["dictionary"][0]["heard_as"], json!(["fetch user", "费驰优瑟"]));
    assert_eq!(json["rules"][1]["kind"], "regex");
    assert_eq!(json["rules"][1]["case_sensitive"], false);
    assert_eq!(json["history_recent"][0]["vocabulary"]["rules"][0], json!({ "id": RULE_ID_2, "count": 2 }));
    assert!(json["history_recent"][1].get("vocabulary").is_none(), "None is omitted");
    assert_eq!(json["settings"]["engines"]["chinese_script"], "simplified");
    // docs/dictation.md §19.
    assert_eq!(json["settings"]["edit_hotkey"], "Ctrl+Alt+E");
    assert_eq!(json["dictation"]["kind"], "dictation");
    assert_eq!(json["hotkey"]["edit_registered"], "Ctrl+Alt+E");
    assert_eq!(json["history_recent"][2]["kind"], "edit");
    assert_eq!(json["history_recent"][2]["edit"], json!({ "instruction": EDIT_INSTRUCTION, "selection": EDIT_SELECTION }));
    assert!(json["history_recent"][0].get("edit").is_none(), "None is omitted");
    assert_eq!(json["history_recent"][0]["kind"], "dictation");
    // docs/dictation.md §18.
    assert_eq!(json["settings"]["context_sharing"], json!({ "app_name": false, "window_title": true }));
    assert_eq!(json["scenes"][0]["match"]["title_contains"], json!(["Pull request"]));
    assert_eq!(json["scenes"][0]["overrides"]["refine_preset"], "formal");
    assert_eq!(json["scenes"][1]["overrides"], json!({ "refine_preset": "punctuation" }), "unset overrides are absent");
    assert_eq!(json["dictation"]["context"]["scene"]["name"], "代码评审");
    assert_eq!(json["history_recent"][0]["app"], json!({ "id": "code", "name": "Code" }));
    assert!(json["history_recent"][1].get("app").is_none() && json["history_recent"][1].get("scene").is_none(), "None is omitted");
    // docs/dictation.md §21: presets are strings (a built-in name or a UUID) wherever they appear.
    assert_eq!(json["settings"]["engines"]["refine_preset"], PRESET_ID);
    assert_eq!(json["presets"][0]["id"], PRESET_ID);
    assert_eq!(json["presets"][0]["name"], "周报");
    assert_eq!(json["history_recent"][0]["preset"], json!({ "id": "formal", "name": "书面语" }));
    assert!(json["history_recent"][1].get("preset").is_none() && json["dictation"].get("preset").is_none(), "None is omitted");
}

#[test]
fn events_fixture_covers_every_variant_and_matches_serde_output() {
    let events = all_events();
    let tags: std::collections::BTreeSet<&str> = events.iter().map(event_tag).collect();
    for tag in ALL_EVENT_TAGS {
        assert!(tags.contains(tag), "events fixture lacks a `{tag}` event");
    }
    for event in &events {
        let text = serde_json::to_string(event).unwrap_or_else(|e| panic!("{} does not serialize as a tagged event: {e}", event_tag(event)));
        let json: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json["type"], event_tag(event), "{text}");
    }
    let text = pretty(&events);
    check_fixture(EVENTS_FILE, &text);
    let back: Vec<UiEvent> = serde_json::from_str(&text).unwrap();
    assert_eq!(back, events, "every UiEvent must round-trip through its own JSON");
    // Every `ModelInstallState` variant is represented somewhere in the `models` events.
    let kinds: std::collections::BTreeSet<String> = events
        .iter()
        .filter_map(|e| if let UiEvent::Models { models } = e { Some(models) } else { None })
        .flatten()
        .map(|m| serde_json::to_value(&m.state).unwrap()["kind"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(kinds, ["not_installed", "downloading", "verifying", "installed", "failed", "import_incomplete"].map(String::from).into_iter().collect());
    // On-device recognition shows on the engines events too.
    let local: Vec<&UiEvent> = events.iter().filter(|e| matches!(e, UiEvent::Engines(s) if s.asr_provider == ProviderId::Local)).collect();
    assert_eq!(local.len(), 2);
    for e in local {
        let json = serde_json::to_value(e).unwrap();
        assert_eq!(json["asr_host"], "");
        assert_eq!(json["local_model"], PARAFORMER_ID);
    }
    // The live preview (docs/dictation.md §11) is sampled in every shape the pill discriminates on.
    let listening: Vec<Value> = events
        .iter()
        .filter(|e| matches!(e, UiEvent::Dictation(s) if matches!(s.phase, DictationPhase::Listening { .. })))
        .map(|e| serde_json::to_value(e).unwrap()["phase"].clone())
        .collect();
    assert_eq!(listening.len(), 7);
    assert_eq!(listening[5].get("context"), None, "the context sits next to the phase, not inside it");
    assert_eq!(listening[4]["live"]["injected"], 1, "live_inject counts the pasted sentences");
    assert_eq!(listening[1]["live"]["injected"], 0);
    assert_eq!(listening[0]["ready"], false);
    assert_eq!(listening[0]["locked"], false, "the lock flag is always on the wire");
    assert_eq!(listening[3]["locked"], true);
    assert!(listening[0].get("live").is_none(), "no preview while the device opens");
    assert_eq!(listening[1]["ready"], true);
    assert_eq!(listening[1]["live"]["committed"][0]["text"], LIVE_COMMITTED);
    assert_eq!(listening[1]["live"]["committed"][0]["end_ms"], 1480);
    assert_eq!(listening[1]["live"]["current"], LIVE_CURRENT);
    assert!(listening[1]["live"].get("degraded").is_none(), "None is omitted");
    assert!(listening[2]["live"]["degraded"].as_str().is_some_and(|d| d.contains("overrun")));
    let previews: Vec<Option<&str>> = events
        .iter()
        .filter_map(|e| match e {
            UiEvent::Dictation(DictationStatus { phase: DictationPhase::Processing { preview, .. }, .. }) => Some(preview.as_deref()),
            _ => None,
        })
        .collect();
    assert_eq!(
        previews,
        vec![
            Some("把 fetchUser 改成 async，然后加上错误"),
            Some("把 fetchUser 改成 async，然后加上错误"),
            None,
            Some("把 fetchUser 改成 async，然后加上错误"),
            None,
            None,
            Some(EDIT_INSTRUCTION)
        ]
    );
    // docs/dictation.md §18.6: the context rides next to the phase; a take without a scene omits it.
    let contexts: Vec<Value> = events
        .iter()
        .filter_map(|e| if let UiEvent::Dictation(s) = e { s.context.as_ref().map(|_| serde_json::to_value(e).unwrap()["context"].clone()) } else { None })
        .collect();
    assert_eq!(contexts.len(), 3);
    assert_eq!(contexts[0]["scene"]["name"], "代码评审");
    assert_eq!(contexts[1], json!({ "app": { "id": "winword", "name": "WINWORD" } }));
    assert_eq!(contexts[2], json!({ "app": { "id": "slack", "name": "Slack" } }), "an edit take names its app, never a scene (§19)");
    // §12 on the wire: the `finalizing` stage, `done` with its mode / segments / live_error (absent
    // for a plain whole take), `cancelled` with `injected_chars`, and the effective output mode.
    let phases: Vec<Value> =
        events.iter().filter_map(|e| if let UiEvent::Dictation(s) = e { Some(serde_json::to_value(s).unwrap()["phase"].clone()) } else { None }).collect();
    assert!(phases.iter().any(|p| p["phase"] == "processing" && p["stage"] == "finalizing"), "{phases:?}");
    let dones: Vec<&Value> = phases.iter().filter(|p| p["phase"] == "done").collect();
    assert_eq!(dones.len(), 6);
    assert_eq!(dones[0]["mode"], "whole_take");
    assert!(dones[0].get("segments").is_none() && dones[0].get("live_error").is_none(), "None is omitted: {}", dones[0]);
    // docs/dictation.md §28: the insert check rides on `done`, and an unchecked take omits it.
    assert_eq!(dones[0]["check"], "written");
    assert!(dones[1].get("check").is_none(), "{}", dones[1]);
    assert_eq!(dones[2]["check"], "not_written");
    assert_eq!(dones[3]["mode"], "streaming_final");
    assert_eq!(dones[3]["segments"][1]["text"], "然后加上错误处理");
    assert_eq!(dones[3]["segments"][1]["end_ms"], 3200);
    assert_eq!(dones[4]["mode"], "whole_take");
    assert!(dones[4]["live_error"].as_str().is_some_and(|e| e.starts_with("open:")));
    let cancelled: Vec<&Value> = phases.iter().filter(|p| p["phase"] == "cancelled").collect();
    assert_eq!(cancelled.iter().map(|c| c["injected_chars"].as_u64().unwrap()).collect::<Vec<_>>(), vec![0, LIVE_COMMITTED.chars().count() as u64]);
    let modes: std::collections::BTreeSet<&str> =
        events.iter().filter_map(|e| if let UiEvent::Engines(s) = e { Some(s.effective_output_mode.as_str()) } else { None }).collect();
    assert_eq!(modes, ["whole_take", "streaming_final", "live_inject"].into_iter().collect());
    let history_modes: Vec<&str> =
        events.iter().filter_map(|e| if let UiEvent::History { recent, .. } = e { Some(recent) } else { None }).flatten().map(|h| h.mode.as_str()).collect();
    assert!(history_modes.contains(&"live_inject") && history_modes.contains(&"whole_take"), "{history_modes:?}");
    let processing_json = serde_json::to_value(
        &events[events
            .iter()
            .position(|e| matches!(e, UiEvent::Dictation(DictationStatus { phase: DictationPhase::Processing { preview: None, .. }, .. })))
            .unwrap()],
    )
    .unwrap();
    assert!(processing_json["phase"].get("preview").is_none(), "None is omitted: {processing_json}");
    // Every model capability and tier the settings dialog groups by.
    let (mut tiers, mut capabilities) = (std::collections::BTreeSet::new(), std::collections::BTreeSet::new());
    for m in events.iter().filter_map(|e| if let UiEvent::Models { models } = e { Some(models) } else { None }).flatten() {
        tiers.insert(m.tier.clone());
        capabilities.extend(m.capabilities.iter().cloned());
    }
    assert_eq!(tiers, ["balanced", "light", "streaming"].map(String::from).into_iter().collect());
    assert_eq!(capabilities, ["offline", "streaming"].map(String::from).into_iter().collect());
    // docs/dictation.md §19 on the wire: every status says its kind; the edit samples say `edit`;
    // every failure code is sampled; the switched-off edit hotkey is an explicit `null`.
    let kinds: std::collections::BTreeSet<String> = events
        .iter()
        .filter_map(|e| if let UiEvent::Dictation(_) = e { Some(serde_json::to_value(e).unwrap()["kind"].as_str().unwrap().to_owned()) } else { None })
        .collect();
    assert_eq!(kinds, ["dictation", "edit"].map(String::from).into_iter().collect());
    let codes: std::collections::BTreeSet<String> = phases.iter().filter(|p| p["phase"] == "failed").map(|p| p["code"].as_str().unwrap().to_owned()).collect();
    for code in ["no_selection", "selection_too_long", "selection", "edit_unavailable", "edit_in_terminal"] {
        assert!(codes.contains(code), "events fixture lacks the `{code}` failure");
    }
    let settings_json: Vec<Value> = events.iter().filter(|e| matches!(e, UiEvent::Settings(_))).map(|e| serde_json::to_value(e).unwrap()).collect();
    assert_eq!(settings_json[0]["edit_hotkey"], "Ctrl+Alt+E");
    assert_eq!(settings_json[1]["edit_hotkey"], Value::Null, "switched off is serialised, not omitted");
    let hotkeys: Vec<Value> = events.iter().filter(|e| matches!(e, UiEvent::Hotkey(_))).map(|e| serde_json::to_value(e).unwrap()).collect();
    assert_eq!((hotkeys[0]["edit_registered"].as_str(), hotkeys[1]["edit_error"].as_str().is_some()), (Some("Ctrl+Alt+E"), true));
}

#[test]
fn commands_fixture_is_the_wire_form_and_parses_into_every_variant() {
    let commands = all_commands();
    let mut entries: Vec<Value> = commands.iter().map(|(name, args, _)| json!({ "name": name, "args": args })).collect();
    entries.extend(shell_only_commands().iter().map(|(name, args)| json!({ "name": name, "args": args })));
    check_fixture(COMMANDS_FILE, &pretty(&entries));

    let mut seen = std::collections::BTreeSet::new();
    for (name, args, expected) in &commands {
        // `invoke(name, args)` on the webview becomes the tagged form the bridge understands.
        let mut tagged = match args {
            Value::Null => serde_json::Map::new(),
            Value::Object(map) => map.clone(),
            other => panic!("{name}: args must be an object or null, got {other}"),
        };
        tagged.insert("command".into(), Value::String((*name).to_owned()));
        let cmd: UiCommand = serde_json::from_value(Value::Object(tagged)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(command_variant(&cmd), *expected, "{name}");
        cmd.clone().into_core().unwrap_or_else(|e| panic!("{name}: {e}"));
        seen.insert(*expected);
    }
    // Every variant of `UiCommand` has an entry (the list above is exhaustive by `command_variant`).
    for variant in [
        "PairingStart",
        "PairingJoinCode",
        "PairingJoinTicket",
        "PairingConfirm",
        "PairingReject",
        "PairingCancel",
        "PairingReset",
        "DeviceForget",
        "DeviceRename",
        "SendText",
        "SettingsSetRelay",
        "SettingsSetTheme",
        "SettingsSetHotkey",
        "SettingsSetEditHotkey",
        "SettingsSetCopyLastHotkey",
        "SettingsSetSoloKey",
        "SettingsSetMicrophone",
        "SettingsSetRecording",
        "SettingsSetLocale",
        "SettingsSetAutoUpdate",
        "SettingsSetHistory",
        "SettingsSetOverlay",
        "PhoneTakeStart",
        "PhoneTakeStop",
        "PhoneTakeCancel",
        "PhoneTextSend",
        "SentTextsClear",
        "SettingsSetPairingAlwaysOn",
        "RelayReconnect",
        "DevicesRefresh",
        "ConnectivityCheck",
        "DictationStart",
        "DictationStop",
        "DictationCancel",
        "DictationDismiss",
        "HotkeyEdge",
        "SettingsSetActivation",
        "SettingsSetEngines",
        "ProviderKeySet",
        "ProviderProbe",
        "ProviderModelsRefresh",
        "HistoryDelete",
        "HistoryClear",
        "HistoryStar",
        "ModelDownload",
        "ModelCancel",
        "ModelRemove",
        "DictionaryAdd",
        "DictionaryUpdate",
        "DictionaryRemove",
        "DictionaryReorder",
        "VocabularySuggestionIgnore",
        "VocabularySuggestionUnignore",
        "RulesAdd",
        "RulesUpdate",
        "RulesRemove",
        "RulesReorder",
        "RulesImport",
        "ScenesAdd",
        "ScenesUpdate",
        "ScenesRemove",
        "ScenesReorder",
        "SettingsSetContextSharing",
        "SettingsSetBackup",
        "BackupFolderReset",
        "BackupNoticeDismiss",
        "SettingsSetMcp",
        "McpCopyToken",
        "McpRotateToken",
        "McpCopySetup",
        "McpUndo",
        "ImportAdd",
        "ImportCancel",
        "ImportRetry",
        "ImportRemove",
    ] {
        assert!(seen.contains(variant), "commands fixture lacks {variant}");
    }
    // The secret value travels in a command only; no fixture event or state may carry it.
    let secret = "gsk_example_not_a_real_key";
    assert!(!pretty(&full_state()).contains(secret));
    assert!(!pretty(&all_events()).contains(secret));
}

/// The built-in scenes (§18.10) as the preview's in-memory backend fills them in: each category's
/// defaults on the three desktops and on a phone (no applications, user decision 2026-10-01), and
/// its term pack (`@voltip/shared/mock` reads this file).
#[test]
fn builtin_scenes_fixture_matches_the_core() {
    let rows: Vec<Value> = BuiltinScene::ALL
        .into_iter()
        .map(|scene| {
            json!({
                "id": scene,
                "templates": {
                    "windows": scene.template(Platform::Windows),
                    "macos": scene.template(Platform::Macos),
                    "linux": scene.template(Platform::Linux),
                    "android": scene.template(Platform::Android),
                },
                "terms": voltip_core::vocabulary::packs::terms(scene),
                // The names the history's search finds it by; the dictionaries must say the same.
                "names": { "zh-CN": scene.display_name(), "en": scene.english_name() },
            })
        })
        .collect();
    check_fixture(BUILTIN_SCENES_FILE, &pretty(&rows));
}

/// docs/dictation.md §4.4: the answers of the history queries, and the arguments of
/// `history_query`, in the shapes `packages/shared/src/schema.ts` parses.
#[test]
fn history_queries_fixture_matches_serde_output() {
    let entry = history_entries().remove(0);
    let bucket = |count: u32| HistoryStatsBucket {
        count,
        raw_chars: 58 * u64::from(count),
        corrected_chars: 7 * u64::from(count),
        spoken_ms: 11_000 * u64::from(count),
        latency_ms: 1231 * u64::from(count),
    };
    let answers = json!({
        "query": HistoryQuery { since_ms: Some(AT_MS - 3_600_000), starred: false, failed: true, query: "会议".into(), offset: 100, limit: 100 },
        "page": HistoryPage { entries: vec![entry.clone()], matching: 12, total: 312 },
        "entry": entry,
        "missing": Option::<HistoryEntry>::None,
        "stats": HistoryStats { buckets: vec![bucket(0), bucket(2)], total: bucket(6) },
        "hits": HistoryHits {
            dictionary: [(uuid(DICT_ID), 3)].into_iter().collect(),
            rules: [(uuid(RULE_ID), 1)].into_iter().collect(),
        },
        // docs/dictation.md §20.8: a phone's copy of a computer's history; a record the phone
        // uploaded comes back named after it.
        "mirror_entry": voltip_tauri_bridge::MirrorEntry {
            entry: HistoryEntry { origin: Some(EntryOrigin { device: "Pixel 8".into(), kind: OriginKind::Standalone }), ..entry.clone() },
            shortened: true,
        },
        "mirror_missing": Option::<voltip_tauri_bridge::MirrorEntry>::None,
        "mirror_profile": mirror_profile(),
    });
    check_fixture(HISTORY_QUERIES_FILE, &pretty(&answers));
}

/// docs/dictation.md §16.9: the drafts `vocabulary_backtest` takes (a new dictionary entry, one
/// standing in for an existing entry, a rule) and its answers, in the shapes
/// `packages/shared/src/schema.ts` parses.
#[test]
fn vocabulary_backtest_fixture_matches_serde_output() {
    use voltip_core::vocabulary::{Backtest, BacktestDraft, BacktestExample};
    let entry = DictionaryDraft { term: "Voltip".into(), heard_as: vec!["维他命".into()], enabled: true };
    let rule = RuleDraft {
        name: "非书 → 飞书".into(),
        kind: RuleKind::Literal,
        pattern: "非书".into(),
        replacement: "飞书".into(),
        case_sensitive: false,
        enabled: true,
    };
    let answers = json!({
        "dictionary_draft": BacktestDraft::Dictionary { id: None, entry: entry.clone() },
        "merge_draft": BacktestDraft::Dictionary {
            id: Some(uuid(DICT_ID)),
            entry: DictionaryDraft { heard_as: vec!["沃尔提普".into(), "维他命".into()], ..entry },
        },
        "rule_draft": BacktestDraft::Rule { id: None, rule },
        "answer": Backtest {
            scanned: 499,
            matched: 3,
            kept: 1,
            examples: vec![
                BacktestExample {
                    history_id: uuid(HISTORY_ID),
                    at_ms: AT_MS,
                    before: "维他命的安装包发给我".into(),
                    after: "Voltip的安装包发给我".into(),
                    kept: true,
                },
                BacktestExample {
                    history_id: uuid(HISTORY_ID_2),
                    at_ms: AT_MS - 86_400_000,
                    before: "…今天用维他命跑了一遍回归".into(),
                    after: "…今天用Voltip跑了一遍回归".into(),
                    kept: false,
                },
            ],
        },
        "nothing": Backtest::default(),
    });
    check_fixture(VOCABULARY_BACKTEST_FILE, &pretty(&answers));
}

/// docs/dictation.md §16.10: the learning suggestions' answers, 加入词典's outcomes, 撤销's
/// outcomes and the rules page's hints, as the shells return them.
#[test]
fn vocabulary_suggestions_fixture_matches_serde_output() {
    use voltip_core::mcp::candidates::PairExample;
    use voltip_core::vocabulary::{AcceptOutcome, EntryClash, Refusal, RuleWarning, Suggestion, SuggestionUndo, Suggestions, SuggestionsAnswer, UndoOutcome};
    let ready = SuggestionsAnswer::Ready(Suggestions {
        scanned: 2000,
        refined: 1480,
        skipped: 3,
        total: 41,
        pairs: vec![
            Suggestion {
                heard: "维他命".into(),
                polished: "Voltip".into(),
                count: 12,
                last_at_ms: AT_MS,
                examples: vec![
                    PairExample { history_id: uuid(HISTORY_ID), snippet: "维他命的安装包发给我".into() },
                    PairExample { history_id: uuid(HISTORY_ID_2), snippet: "…今天用维他命跑了一遍回归".into() },
                ],
            },
            Suggestion {
                heard: "提成".into(),
                polished: "集成".into(),
                count: 3,
                last_at_ms: AT_MS - 86_400_000,
                examples: vec![PairExample { history_id: uuid(HISTORY_ID_3), snippet: "提成在 Teams 里面".into() }],
            },
        ],
    });
    let undo =
        SuggestionUndo { id: uuid(HISTORY_ID_4), entry_id: uuid(DICT_ID), term: "Voltip".into(), created: false, previous_heard_as: vec!["沃提普".into()] };
    let answers = json!({
        "ready": ready,
        "history_off": SuggestionsAnswer::HistoryOff,
        "nothing": SuggestionsAnswer::Ready(Suggestions::default()),
        "accepted": AcceptOutcome::Accepted { undo },
        "refused": [
            AcceptOutcome::Refused { refusal: Refusal::Rule { rule_id: uuid(RULE_ID), name: "改回去".into(), becomes: "维他命".into() } },
            AcceptOutcome::Refused { refusal: Refusal::Entry { entry_id: uuid(DICT_ID_2), term: "Vitamin".into(), clash: EntryClash::HeardIsTerm } },
            AcceptOutcome::Refused { refusal: Refusal::Entry { entry_id: uuid(DICT_ID_2), term: "Vitamin".into(), clash: EntryClash::HeardTaken } },
            AcceptOutcome::Refused { refusal: Refusal::Entry { entry_id: uuid(DICT_ID_2), term: "Vitamin".into(), clash: EntryClash::TermTaken } },
            AcceptOutcome::Refused { refusal: Refusal::Entry { entry_id: uuid(DICT_ID), term: "Voltip".into(), clash: EntryClash::Disabled } },
            AcceptOutcome::Refused { refusal: Refusal::Entry { entry_id: uuid(DICT_ID), term: "Voltip".into(), clash: EntryClash::TooMany } },
            AcceptOutcome::Refused { refusal: Refusal::Full { max: 500 } },
            AcceptOutcome::Refused { refusal: Refusal::Failed { message: "vocabulary: dictionary.json：磁盘已满".into() } },
        ],
        "undo": [
            UndoOutcome::Undone,
            UndoOutcome::Expired,
            UndoOutcome::Gone,
            UndoOutcome::Changed,
            UndoOutcome::Failed { message: "vocabulary: dictionary.json：磁盘已满".into() },
        ],
        "rule_warnings": [
            RuleWarning::Duplicate { rule_id: uuid(RULE_ID), other_id: uuid(RULE_ID_2), other_name: "非书 → 飞书 (2)".into() },
            RuleWarning::Inverse { rule_id: uuid(RULE_ID_2), other_id: uuid(RULE_ID), other_name: "飞书 → 非书".into() },
            RuleWarning::UndoesEntry { rule_id: uuid(RULE_ID), entry_id: uuid(DICT_ID), term: "Voltip".into(), heard: "维他命".into() },
        ],
    });
    check_fixture(VOCABULARY_SUGGESTIONS_FILE, &pretty(&answers));
}

/// docs/dictation.md §25: the setup commands the core copies, for every tool and both shells'
/// syntax, with a stand-in token; the settings build the same text around a placeholder.
#[test]
fn mcp_setup_fixture_matches_the_core() {
    use voltip_core::mcp::{SetupClient, setup_command};
    let address = "http://127.0.0.1:47841/mcp";
    let token = "TOKEN";
    let rows: Vec<Value> = [SetupClient::ClaudeCode, SetupClient::Codex]
        .into_iter()
        .flat_map(|client| {
            [false, true].map(|windows| json!({ "client": client, "windows": windows, "address": address, "token": token, "command": setup_command(client, address, token, windows) }))
        })
        .collect();
    check_fixture(MCP_SETUP_FILE, &pretty(&rows));
}

/// docs/dictation.md §25: the correction summary's diff in Rust is the history page's in
/// TypeScript (`refine.ts`); `diff-parity.test.ts` runs `diffSegments` on the same cases.
#[test]
fn diff_fixture_matches_the_core() {
    use voltip_core::mcp::candidates::{diff_segments, replacements};
    let cases = [
        ("今天用维他命测了一下", "今天用 Voltip 测了一下。"),
        ("发到非书群，然后提成在teams里面", "发到飞书群。然后集成在 Teams 里面。"),
        ("把 fetch user 改成 async 然后加上错误处理", "把 fetchUser 改成 async，然后加上错误处理。"),
        ("嗯 那个 我们 v1.2-rc 的 PR 3 合了吗", "我们 v1.2-rc 的 PR #3 合了吗？"),
        ("a  b\u{3000}c  d", "a b c d"),
        ("", "全新的一句"),
        ("全部删掉", ""),
        ("Voltip", "Voltip"),
    ];
    let rows: Vec<Value> = cases
        .iter()
        .map(|(raw, text)| {
            let pairs: Vec<Value> =
                replacements(raw, text).unwrap().into_iter().map(|r| json!({ "heard": r.heard, "polished": r.polished, "at": r.at })).collect();
            json!({ "raw": raw, "text": text, "segments": diff_segments(raw, text).unwrap(), "pairs": pairs })
        })
        .collect();
    check_fixture(DIFF_FILE, &pretty(&rows));
}

/// docs/dictation.md §4.6: the answers of `backup_export`, `backup_open` and `backup_import` with
/// every outcome and every code, and the parts a page sends, in the shapes
/// `packages/shared/src/schema.ts` parses.
#[test]
fn backup_fixture_matches_serde_output() {
    use voltip_core::backup::{
        BACKUP_FORMAT, BACKUP_VERSION, BackupCounts, BackupError, BackupManifest, BackupPart, BackupParts, ExportOutcome, ImportFailure, ImportOutcome,
        ImportReport, OpenOutcome, PartFailure, PartReport, Skipped,
    };
    const BACKUP_FILE: &str = "backup.json";
    let counts = BackupCounts { history: 20_000, dictionary: 12, rules: 3, scenes: 9, presets: 2 };
    let manifest = BackupManifest {
        format: BACKUP_FORMAT.into(),
        version: BACKUP_VERSION,
        app_version: "0.1.4".into(),
        created_at_ms: AT_MS,
        device: "Studio".into(),
        platform: Platform::Windows,
        counts,
    };
    let part = |part: BackupPart, in_file: u32, restored: u32, skipped: Skipped| PartReport { part, in_file, restored, skipped, removed: 0, error: None };
    let report = ImportReport {
        snapshot: None,
        mode: ImportMode::Merge,
        parts: vec![
            part(BackupPart::Presets, 2, 1, Skipped { duplicate: 1, ..Skipped::default() }),
            part(BackupPart::Scenes, 9, 2, Skipped { duplicate: 7, ..Skipped::default() }),
            part(BackupPart::Dictionary, 12, 9, Skipped { duplicate: 1, conflict: 1, invalid: 1, ..Skipped::default() }),
            part(BackupPart::Rules, 3, 0, Skipped::default()).failed(PartFailure::Write),
            PartReport { removed: 5, ..part(BackupPart::History, 20_000, 19_000, Skipped { duplicate: 10, older: 990, ..Skipped::default() }) },
        ],
    };
    let unavailable = PartReport::new(BackupPart::History, 3).failed(PartFailure::Unavailable);
    let answers = json!({
        "parts": BackupParts { history: true, dictionary: true, rules: false, scenes: true, presets: false },
        "export": [
            ExportOutcome::Saved { path: "C:\\Users\\me\\Documents\\Voltip 备份 2026-10-13.voltip-backup".into(), counts, bytes: 10_951_433 },
            ExportOutcome::Cancelled,
            ExportOutcome::failed(&BackupError::Write("No space left on device (os error 28)".into())),
            ExportOutcome::failed(&BackupError::History("database is locked".into())),
            ExportOutcome::unsupported(),
        ],
        "open": [
            OpenOutcome::Preview { token: uuid(HISTORY_ID), manifest },
            OpenOutcome::Cancelled,
            OpenOutcome::failed(&BackupError::Unreadable("Permission denied (os error 13)".into())),
            OpenOutcome::failed(&BackupError::NotBackup),
            OpenOutcome::failed(&BackupError::Damaged("EOF while parsing a list at line 8 column 0".into())),
            OpenOutcome::failed(&BackupError::TooNew(2)),
            OpenOutcome::failed(&BackupError::TooLarge),
            OpenOutcome::unsupported(),
        ],
        "import": [
            ImportOutcome::Done { report },
            // docs/dictation.md §4.6.1: a replacing import names the automatic backup it made first.
            ImportOutcome::Done {
                report: ImportReport {
                    mode: ImportMode::Replace,
                    parts: vec![unavailable],
                    snapshot: Some(format!(
                        "C:\\Users\\me\\AppData\\Roaming\\voltip\\Voltip\\data\\backups\\voltip-auto-20261030-152000-import-{}.voltip-backup",
                        voltip_core::backup::auto::device_tag(&DESKTOP_KEY)
                    )),
                },
            },
            ImportOutcome::failed(ImportFailure::Expired),
            ImportOutcome::failed(ImportFailure::Unavailable),
            ImportOutcome::failed(ImportFailure::Unsupported),
            ImportOutcome::failed(ImportFailure::Busy),
            ImportOutcome::Failed { code: ImportFailure::SnapshotFailed, detail: "No space left on device (os error 28)".into() },
        ],
    });
    check_fixture(BACKUP_FILE, &pretty(&answers));
}

/// Regression (public release, 2026-09-27): the build's own service and relay never reach the
/// UI. The samples compile a built-in service in; no fixture may name its hosts or keys.
#[test]
fn regression_no_fixture_names_the_builtin_host() {
    let texts = [pretty(&full_state()), pretty(&all_events()), pretty(&all_commands())];
    for text in &texts {
        for needle in ["builtin-host", "builtin-token", "builtin-key"] {
            assert!(!text.contains(needle), "{needle} reached the wire:\n{text}");
        }
    }
    let builtin_relay: Value =
        serde_json::to_value(RelayStatus { endpoint: None, source: RelaySource::Builtin, state: ConnectionState::Connected, attempts: 0, last_failure: None })
            .unwrap();
    assert!(builtin_relay.get("endpoint").is_none());
}
