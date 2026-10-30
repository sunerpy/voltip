// In-memory backend: walks the whole pairing state machine (session → code/QR → countdown → key
// exchange → safety code → dual confirmation → trusted → device online), simulates relay link
// changes, device presence, identity mismatch and message echo, and runs a deterministic dictation
// pipeline (listening → transcribing → refining → done, with a history row; a voice edit of the
// selection set by `setSelection`, docs/dictation.md §19, the same way), and keeps the personal
// dictionary and the replacement rules the way the core does (`./vocabulary`: the same validation,
// refusals, matching, TOML format; the dictionary corrects the transcript and the rules the final
// text of every mock take), and the scenes (`./scenes`: the same validation and first-match rule; a
// fake foreground probe, `setForegroundApp`, picks the scene of every take, which switches its
// output mode and refine switch and lands in the status and the history), and the custom presets
// (`./presets`: the same validation; every take refines with the preset of its start, which the
// status and the history name). Used by `pnpm dev` without Tauri and by every functional test.
// Timers are plain `setTimeout`/`setInterval` so fake timers drive it.
import { z } from "zod";
import type { Backend, DropListener, EventListener, FrameListener, Unsubscribe } from "./backend";
import { type SampleHistoryRow, historyEntries } from "./fixtures/history";
import builtinPresetTexts from "./fixtures/ipc/presets-builtin.json";
import builtinSceneRows from "./fixtures/ipc/scenes-builtin.json";
import { joinLiveText, livePreviewText, relayTooOld } from "./labels";
import {
  type BuiltInService,
  type ListedModels,
  keyEntry,
  providerSpec,
  resolveEngineStatus,
} from "./providers";
import { checkProxySettings, proxyPasswordProblem, proxyProblemCoreText } from "./proxy";
import { isMcpWrite, mcpAddress } from "./mcp";
import { type RetakeModels, applyRetakeModels } from "./audio-cache";
import { baseName } from "./imports";
import {
  type AppRef,
  type ArgsOf,
  AUDIO_CACHE_MAX_KEEP,
  AUDIO_CACHE_MAX_MB,
  AUDIO_CACHE_MB,
  AUDIO_CACHE_MIN_KEEP,
  AUDIO_CACHE_MIN_MB,
  type KeptRecording,
  type KeptStatus,
  type CommandArgs,
  type ConsolePage,
  type SiteId,
  type DeviceConnection,
  type DeviceIdentityPublic,
  type DeviceInfo,
  type DeviceView,
  type DictationPhase,
  type DictationFailureCode,
  type DictionaryEntry,
  type EntrySource,
  type ImportFailure,
  type ImportJob,
  type ImportMode,
  type ImportsView,
  type InsertCheck,
  type PolishModel,
  type RecognitionChoice,
  type SegmentText,
  type VocabularyHits,
  DEFAULT_PRESET,
  type PreviewDraft,
  type RefineFailure,
  type ReplacementRule,
  type RuleDraft,
  type VocabularyBacktest,
  type VocabularyBacktestArgs,
  type VocabularyPreview,
  type AcceptOutcome,
  type RuleWarning,
  type SuggestionAcceptArgs,
  type SuggestionsAnswer,
  type UndoOutcome,
  BACKTEST_LIMIT,
  MAX_BACKTEST_LIMIT,
  MAX_IGNORED,
  MAX_SUGGESTIONS,
  type EngineSettings,
  type EngineStatus,
  type HistoryEntry,
  type IgnoredPair,
  type HotkeyEdgeArgs,
  type InjectPreflight,
  type PasteFailure,
  type PasteOutcome,
  type LiveSegment,
  type LiveText,
  type ModelInstallState,
  type PackageProblem,
  type ModelState,
  type ModelsLocationView,
  type AutoBackupFile,
  type AutoBackupKind,
  type AutoBackupView,
  AUTO_BACKUP_KEEP,
  type BackupFolderOutcome,
  emptyAutoBackup,
  type MutationCommand,
  type OutputMode,
  type Permission,
  type PermissionReport,
  type RelayStatus,
  type SafetyCode,
  type Scene,
  type NetworkStatus,
  type ServeSettings,
  type DictionaryDraft,
  type McpActivity,
  type McpAction,
  type McpSettings,
  type McpSetupClient,
  type McpStatus,
  type ServeStatus,
  type SceneDraft,
  type SceneRef,
  type TakeContext,
  type Settings,
  type Snapshot,
  type TakeKind,
  type TrustedDevice,
  type UiEvent,
  type UiState,
  type UpdateStatus,
  type UsageView,
  type ConnectivityReport,
  type ProbeResult,
  type HotkeyCapabilities,
  type HotkeyStatus,
  SOLO_KEYS,
  type SoloKey,
  type SoloLoss,
  MAX_PHONE_TEXT_CHARS,
  MAX_PASTE_TEXT_CHARS,
  type SentText,
  applyEvent,
  DEFAULT_HOTKEY,
  defaultSettings,
  emptyEngineStatus,
  emptyHotkeyStatus,
  emptyModelsLocation,
  emptyUsageView,
  HISTORY_LIMIT,
  HISTORY_MIN_KEEP,
  MIN_SERVE_PORT,
  MAX_MINUTES_CHOICES,
  HISTORY_RECENT,
  type HistoryHits,
  type HistoryPage,
  type MirrorEntry,
  type MirrorProfile,
  type MirrorView,
  MAX_SYNC_PEERS,
  type HistoryQueryArgs,
  type HistoryStats,
  idleDictation,
  idleSnapshot,
  idleUpdate,
  MAX_ACTIVATION_MS,
  MAX_EDIT_SELECTION_CHARS,
  notApplicablePermissions,
  uncheckedPreflight,
  type AudioDevice,
  type AudioOutputs,
  type ExportFormat,
  type ExportOutcome,
  BACKUP_EXTENSION,
  type BackupCounts,
  type BackupExportOutcome,
  type BackupImportOutcome,
  type BackupOpenOutcome,
  type BackupPartReport,
  type BackupParts,
  dictionaryEntrySchema,
  replacementRuleSchema,
  type HostOs,
  type LevelFrame,
  type ProbeFailure,
  type ServiceKind,
  type GuidePage,
  type ProjectLink,
  FEEDBACK_ATTACHMENT_TYPES,
  FEEDBACK_CONTACT_MAX,
  FEEDBACK_MAX_ATTACHMENTS,
  FEEDBACK_MAX_ATTACHMENT_TOTAL_BYTES,
  FEEDBACK_MAX_IMAGE_BYTES,
  FEEDBACK_MAX_VIDEO_BYTES,
  FEEDBACK_MESSAGE_MAX,
  type AttachmentFile,
  type FeedbackAttachmentError,
  type StagedAttachment,
  type FeedbackDiagnostics,
  type FeedbackDraft,
  type FeedbackError,
  type FeedbackInfo,
  type FeedbackReceipt,
  type HardwareStatus,
  type PhoneTakeState,
  phoneTakeFinal,
  type ProviderId,
  type BuiltinPreset,
  type BuiltinPresetText,
  type CustomPreset,
  type PresetDraft,
  type PresetId,
  type PresetRef,
  type PresetTryOutcome,
  type ProcessState,
  type RecordingSource,
  type SegmentProgress,
  builtinPresetTextSchema,
  isBuiltinPreset,
  type BuiltinScene,
  type BuiltinSceneTerms,
  builtinSceneSchema,
  isPhone,
  sceneDraftSchema,
} from "./schema";
import {
  type PresetTrial,
  checkPresets,
  presetTrial,
  presetTryText,
  resolvePreset,
  validatePresetDraft,
} from "./presets";
import {
  type ForegroundApp,
  checkScenes,
  isTerminalApp,
  matchScene,
  sanitizeForegroundApp,
  recentApps as recentAppsOf,
  validateSceneDraft,
} from "./scenes";
import { historyHitsOf, historyPageOf, historyStatsOf, recentTextsOf } from "./history-queries";
export type { MockBackupFile } from "./backup-merge";
import {
  type MockBackupFile,
  mergeDictionary,
  mergeHistory,
  mergePresets,
  mergeRules,
  mergeScenes,
} from "./backup-merge";
import {
  Vocabulary,
  backtestVocabulary,
  checkDictionary,
  checkRules,
  exportRulesToml,
  parseRulesToml,
  previewVocabulary,
  validateDictionaryDraft,
  validateRuleDraft,
  validateSuggestionPair,
} from "./vocabulary";
import { acceptSuggestion, ruleWarnings, suggestVocabulary } from "./suggestions";

export interface MockPeer {
  /** Milliseconds after `waiting_for_peer` until the simulated phone joins. */
  joinAfterMs: number;
  /** Milliseconds after joining until the simulated phone confirms the safety code. */
  confirmAfterMs: number;
  info?: DeviceInfo;
  /** The simulated phone scanned the QR code: the computer confirms by itself
   *  (docs/pairing.md 「扫码配对只在手机上确认」). */
  scanned?: boolean;
}

export interface MockBackendOptions {
  /** Session lifetime in seconds (R-PR-1 default 120). */
  ttlSecs?: number;
  /** Auto-drive the remote side; `false` (default) leaves it to `simulate*` calls. */
  autoPeer?: MockPeer | false;
  /** Which side this mock plays; only changes default identity/platform. */
  role?: "desktop" | "phone";
  identity?: DeviceIdentityPublic;
  devices?: DeviceView[];
  settings?: Partial<Settings>;
  relay?: RelayStatus;
  /** Initial history rows; defaults to the sample rows placed relative to `now`. */
  history?: HistoryEntry[];
  /** The kept recordings at start (docs/dictation.md §26), newest first; defaults to none. */
  audioCache?: KeptRecording[];
  /** Where the model library is at start (docs/dictation.md §10.9); defaults to
   *  {@link MOCK_MODELS_ROOT}. */
  modelsLocation?: Partial<ModelsLocationView>;
  /** The automatic backups at start (docs/dictation.md §4.6.1): the newest, a failure, home's
   *  notice; the folder follows `settings.backup.folder` otherwise. */
  autoBackup?: Partial<AutoBackupView>;
  /** Overrides for the resolved engine status (tests that need a status the settings cannot
   *  produce). */
  engines?: Partial<EngineStatus>;
  /** The local service's status at start, instead of the one the settings give (tests that need
   *  a status the preview passes through only for a moment, such as `starting`). */
  serve?: ServeStatus;
  /** The MCP service's status at start (docs/dictation.md §25), instead of the one the settings
   *  give: tests that start with clients or activity. */
  mcp?: McpStatus;
  /** What the preview's system proxy is (`network_refresh` reads it again; default: none), and
   *  whether a proxy password starts saved. */
  network?: NetworkStatus;
  /** Which built-in services the preview pretends were compiled in (default: both, with keys);
   *  `{}` is a build without them. */
  builtIn?: { asr?: BuiltInService; llm?: BuiltInService };
  /** Providers whose key is already stored (`provider_key_set` adds / removes more). */
  providerKeys?: readonly { provider: ProviderId; kind: ServiceKind }[];
  /** Model ids `provider_probe` answers with; `false` makes every probe fail as `unreachable`. */
  probeModels?: Partial<Record<ProviderId, readonly string[]>> | false;
  /** Initial updater status (`disabled` for a build without an update source, `failed`, …). */
  update?: UpdateStatus;
  /** The built-in service's usage and this device's record (docs/dictation.md §27); nothing
   *  reported yet unless given. */
  usage?: Partial<UsageView>;
  /** Install state per catalogue id (`{ "sense-voice-small": { kind: "installed", … } }`); every
   *  model starts `not_installed` otherwise. */
  models?: Partial<Record<string, ModelInstallState>>;
  /** Initial personal dictionary and replacement rules (docs/dictation.md §16); both start empty,
   *  like a first run of the core. Ignored by the phone role (the phone has neither). */
  dictionary?: DictionaryEntry[];
  rules?: ReplacementRule[];
  /** Initial pairs the learning suggestions leave out (忽略, docs/dictation.md §16.10), oldest
   *  first as the core keeps them; empty like a first run. Ignored by the phone role. */
  vocabularyIgnored?: IgnoredPair[];
  /** Initial scenes (docs/dictation.md §18); empty like a first run. Ignored by the phone role. */
  scenes?: Scene[];
  /** Initial custom presets (docs/dictation.md §21); empty like a first run. Ignored by the phone
   *  role. */
  presets?: CustomPreset[];
  /** What the fake foreground probe answers when a take starts (`setForegroundApp` changes it);
   *  `null` (default) = no answer, so no take has a context or a scene. */
  foregroundApp?: ForegroundApp | null;
  /** What `permissionsStatus` answers (docs/dictation.md §15.1). A value is returned as is (and
   *  a `permissionsRequest` flips that permission to `granted`, playing the user who accepts); a
   *  function is called on every poll and may throw to simulate a failing query. Defaults to
   *  `mockPermissions(host)`: everything already settled for the identity's platform. */
  permissions?: PermissionReport | (() => PermissionReport);
  /** What `injectPreflight` answers (§15.3); defaults to the unchecked `proceed` of the host. */
  injectPreflight?: InjectPreflight;
  /** What a valid `pasteText` ends with while no take runs (`setPasteOutcome` changes it);
   *  defaults to `pasted`. */
  pasteOutcome?: PasteOutcome;
  /** What a paste take's insert check finds (docs/dictation.md §28; `setInsertCheck` changes it);
   *  absent = no check runs, as on a host without a focus reader. */
  insertCheck?: InsertCheck;
  /** Only this code joins successfully (phone role); any well-formed code otherwise. */
  expectedCode?: string;
  /** What the desktop shell reports about the machine (docs/dictation.md §10.6); defaults to
   *  `MOCK_HARDWARE`, a CPU-only build. */
  hardware?: HardwareStatus;
  /** How the preview's feedback endpoint answers (docs/feedback.md): `configured` (default) takes
   *  every report after `MOCK_FEEDBACK_MS`, `not_configured` is a build without one, a
   *  `FeedbackError` fails every submission with it (`storage_full` and `attachments` only a
   *  report that carries files, as the endpoint does; `attachments` still takes the report). */
  feedback?: "configured" | FeedbackError;
  /** The phone's clipboard (docs/dictation.md §20.6); `null` = empty. Defaults to
   *  `MOCK_PHONE_CLIPBOARD`. */
  phoneClipboard?: string | null;
  /** What `audioOutputs` answers (docs/dictation.md §22); defaults to the computer's sound
   *  available on `MOCK_AUDIO_OUTPUTS` (the phone role: unsupported, no devices). */
  audioOutputs?: AudioOutputs;
  /** Phone (docs/dictation.md §20.8): its copies of computers, each with the entries and the
   *  settings the copy holds; none by default. */
  mirrors?: MockMirror[];
  /** The import queue it starts with (docs/dictation.md §31), empty by default. These jobs only
   *  show: none runs until `import_retry` queues a failed one again. */
  imports?: ImportJob[];
  /** Clock in milliseconds; injectable for deterministic tests. */
  now?: () => number;
  /** Deterministic randomness source in [0, 1). */
  random?: () => number;
}

/** One computer's copy on the mock phone (`mirror_history_query`, `mirror_profile`). */
export interface MockMirror {
  view: MirrorView;
  history: HistoryEntry[];
  profile: MirrorProfile | null;
  /** Entries that arrived shortened. */
  shortened?: readonly string[];
}

/** A computer's settings as a phone shows them, from settings and lists (the copy's `profile`). */
export function mockMirrorProfile(
  settings: Settings = defaultSettings(),
  lists: Partial<Pick<MirrorProfile, "presets" | "dictionary" | "rules" | "scenes">> = {},
): MirrorProfile {
  return {
    locale: settings.locale,
    theme: settings.theme,
    follow_system_theme: settings.follow_system_theme,
    asr_provider: settings.engines.asr_provider,
    asr_model: MOCK_ENGINE_BUILTIN.asr_model,
    refine_enabled: settings.engines.refine_enabled,
    llm_provider: settings.engines.llm_provider,
    refine_model: MOCK_ENGINE_BUILTIN.refine_model,
    preset: settings.engines.refine_preset,
    presets: [...(lists.presets ?? [])],
    dictionary: [...(lists.dictionary ?? [])],
    rules: [...(lists.rules ?? [])],
    scenes: [...(lists.scenes ?? [])],
  };
}

export const MOCK_PUBLIC_KEYS = {
  desktop: "9f0c2b1e6a4d3c5f7e8a9b0c1d2e3f405162738495a6b7c8d9e0f1a2b3c4d5e6",
  phone: "1a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e7f809",
  laptop: "5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d",
} as const;

const SAFETY_WORDS = [
  "amber",
  "boat",
  "cedar",
  "delta",
  "ember",
  "falcon",
  "glacier",
  "harbor",
  "ivory",
  "juniper",
  "kestrel",
  "lantern",
  "meadow",
  "nickel",
  "orchid",
  "pebble",
];

const CREATE_SESSION_MS = 300;
const KEY_EXCHANGE_MS = 400;
const RELAY_CONNECT_MS = 500;
const MESSAGE_ECHO_MS = 150;

/** Simulated pipeline timings (docs/dictation.md §2): ASR, then the optional LLM pass, then the
 *  dwell before the phase returns to idle. */
export const MOCK_ASR_MS = 400;
export const MOCK_REFINE_MS = 300;
/** The detail a turned-down clean-up carries (`simulateRefineFailure`). */
export const MOCK_REFINE_FAILED = "请求过于频繁：mock: the clean-up was turned down";
/** `voltip_core::history::process::PART_MAX_CHARS`: the preview's 用 AI 预设处理 counts its parts
 *  the same way (docs/dictation.md §22). */
export const MOCK_PART_MAX_CHARS = 1_500;
/** Where the preview says an export was saved. */
export const MOCK_EXPORT_DIR = "/home/you/Documents";

/** What a file holds for the preview's import (`simulateImportFile`, docs/dictation.md §31). */
export interface MockImportFile {
  /** Its size in bytes. */
  size: number;
  /** The length of its sound. */
  duration_ms: number;
  /** Why it stops: a decoding failure every time it is read; `recognition` once, at segment
   *  `failAt` (from 0), so that 重试 goes on from there; `no_speech` once it is recognised. */
  failure?: ImportFailure;
  failAt?: number;
}

/** Where the preview's open dialog finds the files to import. */
export const MOCK_IMPORT_DIR = "/home/you/Videos";
/** What 导入音频或视频's open dialog picks in the preview (`simulateImportPick` changes it). */
export const MOCK_IMPORT_PICK: readonly string[] = [
  `${MOCK_IMPORT_DIR}/team-sync.mp4`,
  `${MOCK_IMPORT_DIR}/interview.m4a`,
];
/** The files the preview's import takes: `voltip_media::EXTENSIONS`. */
export const MOCK_IMPORT_EXTENSIONS: readonly string[] = [
  "wav",
  "mp3",
  "m4a",
  "m4b",
  "aac",
  "flac",
  "ogg",
  "oga",
  "opus",
  "mp4",
  "m4v",
  "mov",
  "mkv",
  "mka",
  "webm",
];
/** A step of the preview's import: a quarter of the decoding, or a segment recognised or
 *  polished. */
export const MOCK_IMPORT_STEP_MS = 400;
/** A segment of the preview's import (the core cuts 3–15 s, docs/dictation.md §31). */
export const MOCK_IMPORT_SEGMENT_MS = 12_000;
/** What a path holds when `simulateImportFile` said nothing of it: 1 min 40 s of speech. */
export const MOCK_IMPORT_FILE: MockImportFile = { size: 18_400_000, duration_ms: 100_000 };
/** `ImportError` (docs/dictation.md §31) as the core words it. */
export const MOCK_IMPORT_BUILTIN = "导入只能使用本机模型或你自己设置的服务，不能使用内置服务";
export const MOCK_IMPORT_NO_KEY = "这个云端服务需要填写你自己的密钥，才能用于导入";
export const MOCK_IMPORT_UNAVAILABLE = "这台设备不能导入音视频文件";
export const MOCK_IMPORT_HISTORY_OFF =
  "历史记录已关闭：导入的文字保存在历史记录里，请先打开历史记录";
/** What the preview's import recognises, segment after segment, and how it polishes each. */
export const MOCK_IMPORT_SEGMENTS: readonly { raw: string; refined: string }[] = [
  {
    raw: "嗯那我们开始吧今天主要过一下下周发布的安排",
    refined: "我们开始吧，今天主要过一下下周发布的安排。",
  },
  { raw: "安装包周三出然后周四开始灰度", refined: "安装包周三出，周四开始灰度。" },
  { raw: "文档那边呢截图要重新截一遍", refined: "文档的截图要重新截一遍。" },
  { raw: "那个反馈里提到的导入问题已经修好了", refined: "反馈里提到的导入问题已经修好了。" },
  {
    raw: "大家还有别的问题吗没有的话就先到这里",
    refined: "大家还有别的问题吗？没有的话就先到这里。",
  },
];
export const MOCK_DICTATION_DWELL_MS = 2500;
export const MOCK_DICTATION_FAILED_DWELL_MS = 6000;
/** `voltip_core::dictation::engine::WINDOW_CHANGED`: why a take whose window changed was copied
 *  (docs/dictation.md §14.3). */
export const MOCK_WINDOW_CHANGED = "开始听写时的窗口已不在前台，文字只复制到剪贴板";
/** The streaming modes (docs/dictation.md §12) wait this long for the recogniser's final text
 *  (`finalizing`) instead of the whole-take ASR pass, and `live_inject` pastes its tail in one more
 *  beat of the same length (`inserting`). */
export const MOCK_FINALIZE_MS = 120;
/** `live_error` the mock records when a streaming take ends without any text (§12). */
export const MOCK_EMPTY_STREAM_ERROR = "实时识别未得到文本";

/** The built-in service the browser preview pretends was compiled in: its models only (its host
 *  never reaches the UI, like in the real app). */
export const MOCK_ENGINE_BUILTIN = {
  asr_model: "Qwen/Qwen3-ASR-1.7B",
  refine_model: "qwen/qwen3.8-27b",
} as const;
/** How long a simulated `provider_probe` takes. */
export const MOCK_PROBE_MS = 150;

/** Simulated updater (docs/frontend.md §7): the check answers after `MOCK_UPDATE_CHECK_MS`
 *  with a deterministic newer version; the install streams three download ticks, then `ready`,
 *  then `installing` (the real updater relaunches the app at that point). */
export const MOCK_UPDATE_CHECK_MS = 300;
export const MOCK_UPDATE_TICK_MS = 200;
export const MOCK_UPDATE_TICKS = 3;
export const MOCK_UPDATE_TOTAL_BYTES = 48_000_000;
export const MOCK_CURRENT_VERSION = "0.0.1";
/** The phone's `update_install` before a check found a newer release (`update.rs` `NOTHING_TO_INSTALL`). */
export const PHONE_NOTHING_TO_INSTALL = "updater: 没有可下载的新版本，请先检查更新";
export const MOCK_AVAILABLE_VERSION = "0.0.2";
/** Release notes as release-please writes them into `latest.json`. */
export const MOCK_UPDATE_NOTES = [
  "## 0.0.2 (2026-09-25)",
  "",
  "",
  "### Features",
  "",
  "* **engines:** a faster recognition path ([a1b2c3d](https://example.test/commit/a1b2c3d))",
  "",
  "### Bug Fixes",
  "",
  "* the pill keeps its place on a second screen",
].join("\n");

/** The streaming model the live preview needs (docs/dictation.md §11). */
export const MOCK_STREAMING_MODEL_ID = "zipformer-stream-zh-en";

/** Where the preview pretends the model files land (`<app data dir>/models/<id>`). */
export const MOCK_MODELS_ROOT = "~/.local/share/voltip/models";
/** The folder the preview's folder dialog answers with (docs/dictation.md §10.9). */
export const MOCK_CHOSEN_MODELS_DIR = "/data/Voltip/models";
/** Where the preview pretends the automatic backups go (`<app data dir>/backups`,
 *  docs/dictation.md §4.6.1). */
export const MOCK_BACKUPS_DIR = "~/.local/share/voltip/backups";
/** The folder the preview's 自动备份位置 dialog answers with. */
export const MOCK_CHOSEN_BACKUPS_DIR = "/data/Sync/Voltip";

/** The real catalogue's files, name and size (`voltip_asr_local::catalogue`). */
const MOCK_MODEL_FILES: Record<string, readonly (readonly [string, number])[]> = {
  "qwen3-asr-0.6b": [["Qwen3-ASR-0.6B-Q6_K.gguf", 690_417_824]],
  "qwen3-asr-1.7b": [["Qwen3-ASR-1.7B-Q6_K.gguf", 1_692_554_208]],
  "sense-voice-small": [
    ["model.int8.onnx", 239_233_841],
    ["tokens.txt", 315_894],
  ],
  "paraformer-zh": [
    ["model.int8.onnx", 227_330_205],
    ["tokens.txt", 75_354],
  ],
  "zipformer-stream-zh-en": [
    ["encoder.int8.onnx", 155_278_641],
    ["decoder.onnx", 11_309_084],
    ["joiner.int8.onnx", 2_581_422],
    ["tokens.txt", 58_806],
    ["bpe.model", 119_265],
  ],
};

/** The models that come as one release package of their files (docs/dictation.md §29.12): the
 *  package's path under github.com. */
const MOCK_MODEL_PACKAGES: Record<string, string> = {
  [MOCK_STREAMING_MODEL_ID]:
    "k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-x-asr-480ms-streaming-zipformer-transducer-zh-en-punct-int8-2026-06-05.tar.bz2",
};

/** A catalogue row with its directory and its files' public addresses (or its package's), as the
 *  core reports them. */
function withFiles(
  row: Omit<ModelState, "active" | "state" | "dir" | "files">,
): Omit<ModelState, "active" | "state"> {
  const pkg = MOCK_MODEL_PACKAGES[row.id];
  return {
    ...row,
    dir: `${MOCK_MODELS_ROOT}/${row.id}`,
    files: (MOCK_MODEL_FILES[row.id] ?? []).map(([name, size]) => ({
      name,
      size_bytes: size,
      urls:
        pkg === undefined
          ? ["https://huggingface.co", "https://hf-mirror.com"].map(
              (base) => `${base}/${row.repo}/resolve/main/${name}`,
            )
          : [],
    })),
    ...(pkg === undefined
      ? {}
      : { package: { name: pkg.split("/").at(-1) ?? pkg, urls: [`https://github.com/${pkg}`] } }),
  };
}

/** The local model catalogue the preview pretends was compiled in (docs/dictation.md §10: the
 *  five product tiers, sizes as the real files, names and descriptions in the core's own words —
 *  the UI localises both by id). `MOCK_MODEL_CATALOGUE[0]` is the default. */
const MOCK_MODEL_ROWS: readonly Omit<ModelState, "active" | "state" | "dir" | "files">[] = [
  {
    id: "qwen3-asr-0.6b",
    name: "均衡",
    engine: "transcribe_cpp",
    tier: "balanced",
    capabilities: ["offline"],
    languages: ["zh", "en", "ja", "ko", "yue", "de", "fr", "es", "ru", "ar"],
    size_bytes: 690_417_824,
    description: "推荐；Qwen3-ASR 0.6B，30 语种自动识别，自带标点；690 MB",
    recommended: true,
    repo: "handy-computer/Qwen3-ASR-0.6B-gguf",
  },
  {
    id: "qwen3-asr-1.7b",
    name: "高精度",
    engine: "transcribe_cpp",
    tier: "accurate",
    capabilities: ["offline"],
    languages: ["zh", "en", "ja", "ko", "yue", "de", "fr", "es", "ru", "ar"],
    size_bytes: 1_692_554_208,
    description: "Qwen3-ASR 1.7B，精度最高的档位，30 语种自动识别，自带标点；1.7 GB",
    recommended: false,
    repo: "handy-computer/Qwen3-ASR-1.7B-gguf",
  },
  {
    id: "sense-voice-small",
    name: "轻量",
    engine: "sense_voice",
    tier: "light",
    capabilities: ["offline"],
    languages: ["zh", "en", "ja", "ko", "yue"],
    size_bytes: 239_549_735,
    description: "SenseVoice Small，中英日韩粤，自带标点与数字规整（ITN）；240 MB",
    recommended: false,
    repo: "csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17",
  },
  {
    id: "paraformer-zh",
    name: "轻量 · 中文",
    engine: "paraformer",
    tier: "light",
    capabilities: ["offline"],
    languages: ["zh", "en"],
    size_bytes: 227_405_559,
    description: "Paraformer 中文（含方言）更准，中英混读；无标点，开启 AI 润色可补；227 MB",
    recommended: false,
    repo: "csukuangfj/sherpa-onnx-paraformer-zh-2024-03-09",
  },
  {
    id: MOCK_STREAMING_MODEL_ID,
    name: "实时预览",
    engine: "zipformer_streaming",
    tier: "streaming",
    capabilities: ["streaming"],
    languages: ["zh", "en"],
    size_bytes: 169_347_218,
    description:
      "边说边出字的预览模型（Zipformer 流式，中英混读，自带标点）；最终文本仍由所选引擎识别；169 MB",
    recommended: false,
    repo: "csukuangfj/sherpa-onnx-x-asr-480ms-streaming-zipformer-transducer-zh-en-punct-int8-2026-06-05",
  },
];
export const MOCK_MODEL_CATALOGUE: readonly Omit<ModelState, "active" | "state">[] =
  MOCK_MODEL_ROWS.map(withFiles);
/** Simulated download: `MOCK_MODEL_TICKS` progress events `MOCK_MODEL_TICK_MS` apart, then a
 *  `verifying` beat of the same length, then `installed`. */
export const MOCK_MODEL_TICK_MS = 200;
export const MOCK_MODEL_TICKS = 4;
export const MOCK_MODEL_FILE = "model.int8.onnx";

/** `voltip_core::PRESET_TRY_UNCONFIGURED`: a 试一试 while no clean-up is configured. */
export const PRESET_TRY_UNCONFIGURED = "尚未配置 AI 润色服务，无法试运行预设";
/** The built-in presets' texts (`presets_builtin`), the bytes the desktop shell answers with (the
 *  Rust side keeps the fixture equal to them). */
export const MOCK_BUILTIN_PRESET_TEXTS: readonly BuiltinPresetText[] = builtinPresetTextSchema
  .array()
  .parse(builtinPresetTexts);
/** The built-in scenes (docs/dictation.md §18.10) as the core fills them in: each category's defaults
 *  on the three desktops and on a phone (no applications), and its term pack (the Rust side keeps
 *  the fixture equal to the core). */
export const MOCK_BUILTIN_SCENES = z
  .array(
    z.object({
      id: builtinSceneSchema,
      templates: z.object({
        windows: sceneDraftSchema,
        macos: sceneDraftSchema,
        linux: sceneDraftSchema,
        android: sceneDraftSchema,
      }),
      terms: z.array(z.string()),
    }),
  )
  .parse(builtinSceneRows);

/** What the preview's clean-up answers 试一试 with for the examples of docs/dictation.md §21. */
export const MOCK_PRESET_SAMPLES: Readonly<
  Partial<Record<BuiltinPreset, { input: string; output: string }>>
> = {
  proofread: {
    input: "嗯那个明天上午十点我们开个会吧然后把上周的数据带过来啊不对是上上周的",
    output: "明天上午十点我们开个会吧，然后把上上周的数据带过来。",
  },
  prompt: {
    input: "帮我写个脚本就是把那个日志目录里面超过七天的文件删掉然后每天跑一次对了要能在linux上跑",
    output:
      "请写一个在 Linux 上运行的脚本：\n1. 删除日志目录中超过 7 天的文件；\n2. 每天自动运行一次。",
  },
};
/** A CPU-only build on an 8-thread machine: what the desktop shell reports by default. */
export const MOCK_HARDWARE: HardwareStatus = { cpu_threads: 8, gpus: [] };
/** A build with a GPU backend on a machine with a discrete and an integrated GPU. */
export const MOCK_GPU_HARDWARE: HardwareStatus = {
  cpu_threads: 16,
  gpus: [
    {
      name: "Vulkan0",
      description: "NVIDIA L40S",
      kind: "vulkan",
      memory_mb: 46_068,
      integrated: false,
    },
    {
      name: "Vulkan1",
      description: "Intel UHD Graphics 770",
      kind: "vulkan",
      memory_mb: 0,
      integrated: true,
    },
  ],
};

/** `voltip_desktop_lib::PHONE_TAKE_UNAVAILABLE`: the desktop records phone takes, it sends none. */
export const PHONE_TAKE_UNAVAILABLE = "phone_take: 电脑接收手机的录音，不向其他设备推送";
/** `voltip_desktop_lib::SHARE_UNAVAILABLE`: 「分享」 is the phone's system share sheet. */
export const SHARE_UNAVAILABLE = "share: 电脑端没有系统分享";
/** `feedback::clean_name` in the desktop shell: the last path component, trimmed, control
 *  characters and quotes replaced, at most 120 characters (a longer one keeps its extension);
 *  `undefined` when nothing is left. */
export function cleanAttachmentName(name: string): string | undefined {
  const base = (name.split(/[/\\]/u).pop() ?? "").trim();
  // Characters as Rust counts them (`chars()`): code points.
  // oxlint-disable-next-line no-control-regex -- control characters are what is replaced
  const cleaned = Array.from(base, (c) => (/[\u0000-\u001f\u007f-\u009f"]/u.test(c) ? "_" : c));
  if (cleaned.length === 0) return undefined;
  if (cleaned.length <= 120) return cleaned.join("");
  const text = cleaned.join("");
  const dot = text.lastIndexOf(".");
  const ext = dot >= 0 && Array.from(text.slice(dot)).length <= 10 ? text.slice(dot) : "";
  let stem = cleaned.slice(0, 120 - Array.from(ext).length).join("");
  while (ext.length > 0 && stem.endsWith(ext)) stem = stem.slice(0, -ext.length);
  return `${stem}${ext}`;
}

/** A refused `feedback_attachment_add`, as the shell rejects it. */
function refuseAttachment(reason: FeedbackAttachmentError): Promise<never> {
  return Promise.reject(new Error(reason));
}

/** How long the preview's feedback endpoint takes to answer. */
export const MOCK_FEEDBACK_MS = 300;
/** Always-on pairing in the preview (docs/pairing.md 「常开配对」, the core's `RENEW_BEFORE_SECS`
 *  and `FINISHED_PAUSE`): a waiting session this close to its end is renewed, and a finished one
 *  is followed by the next after this pause. */
export const MOCK_ALWAYS_ON_RENEW_SECS = 10;
export const MOCK_ALWAYS_ON_PAUSE_MS = 4000;
/** The core's refusal on a phone. */
export const ALWAYS_ON_DESKTOP_ONLY = "pairing: 常开配对只在电脑上可用";
/** The core's refusal below the relay's minimum version (`runtime.rs` `require_relay`). */
export const PAIRING_TOO_OLD = "pairing: 此版本过低，请更新 Voltip 后再配对";

/** How long the preview's desktop takes to insert a phone's text (docs/dictation.md §20.6). */
export const MOCK_TEXT_MS = 200;
/** Ports the preview's local speech service cannot bind (another program has them). */
export const MOCK_TAKEN_PORTS: readonly number[] = [47999];
/** `voltip_core::mcp::MCP_UNAVAILABLE`: the phone has no MCP service (docs/dictation.md §25). */
export const MCP_UNAVAILABLE = "mcp: AI 工具接入仅在电脑上提供";

/** What the preview phone's clipboard holds. */
export const MOCK_PHONE_CLIPBOARD = "https://example.test/voltip";
/** `voltip_desktop_lib::PHONE_TEXT_UNAVAILABLE`: the desktop inserts phones' texts, it sends none. */
export const PHONE_TEXT_UNAVAILABLE = "phone_text: 电脑接收手机发来的文字，不向其他设备发送";
/** `live_error` of a take whose scene asked for a streaming mode the live preview cannot serve
 *  (the core's `SCENE_MODE_NOT_READY`, §18.4). */
export const MOCK_SCENE_MODE_NOT_READY =
  "场景要求边说边识别，但实时预览未就绪（已关闭或实时识别模型未下载），本次按整段输出处理";

/** The core's text for a take with nothing left to insert (`FailureCode::NoSpeech`). */
export const MOCK_NO_SPEECH = "没有听到声音";
/** `DictationError::Busy`: a take is running. */
export const MOCK_DICTATION_BUSY = "已有听写正在进行";
/** `voltip_core::audio_cache::RECORDING_GONE` (docs/dictation.md §26). */
export const MOCK_RECORDING_GONE = "这段录音已不在录音缓存中";
/** A 重新识别 on models that cannot run (no key, not installed): the core names the issue. */
export const MOCK_RETAKE_NOT_READY = "服务商缺少 API 密钥（在「语音模型」或「AI 模型」页填写）";
/** A kept recording's size per second: the core's 16 kHz mono 16-bit WAV. */
export const MOCK_WAV_BYTES_PER_SECOND = 32_000;
/** A kept long take's size per second: the core's Ogg Opus parts, about 21.5 kbit/s
 *  (docs/dictation.md §26.2). */
export const MOCK_OPUS_BYTES_PER_SECOND = 2_700;
/** A take longer than this is a long take (`voltip_core::dictation::long::IN_MEMORY`, §22). */
export const MOCK_IN_MEMORY_MS = 120_000;

/** What every simulated dictation "hears" (raw ASR) and what the LLM pass turns it into. */
export const MOCK_DICTATION_RAW = "把这段逻辑抽成一个 helper 然后在 session assembly 里复用";
export const MOCK_DICTATION_TEXT = "把这段逻辑抽成一个 helper，然后在 session_assembly 里复用。";

/** Voice edit (docs/dictation.md §19): what the mock hears as the instruction, what its LLM answers
 *  for any selection, and how long the selection copy at the press takes. */
export const MOCK_EDIT_INSTRUCTION = "改得更正式一点";
export const MOCK_EDIT_TEXT = "各位同事：会议改至周四上午十点，请准时参加。";
export const MOCK_COPY_MS = 60;
/** The core's texts for the edit refusals (`DictationError` display, §19.4). */
export const MOCK_NO_SELECTION = "没有选中文本";
export const MOCK_EDIT_UNAVAILABLE = "edit: 语音编辑需要 AI 润色服务：请先配置润色的 API 密钥";
export const MOCK_EDIT_IN_TERMINAL = "终端里不支持语音编辑：终端里的选区不能被替换";

/** Simulated device start-up (docs/dictation.md §11 `CaptureReady`): the first samples arrive this
 *  long after `dictation_start`; `listening.ready` flips and `started_at` is re-taken. */
export const MOCK_MIC_READY_MS = 150;

/** How long the mock's connectivity self-check takes. */
export const MOCK_CONNECTIVITY_MS = 600;
/** Cadence of the simulated streaming partials once the device is ready (the core throttles at
 *  ≥ 80 ms; the preview pretends to decode a chunk every 400 ms). */
export const MOCK_LIVE_STEP_MS = 400;
/** The streaming recogniser's script over the sample sentence: the current sentence grows three
 *  times, an endpoint commits it (with punctuation, as the streaming model emits it), then the
 *  second sentence grows. Each step is one `LiveText` the pill renders. */
export const MOCK_LIVE_SCRIPT: readonly LiveText[] = [
  { committed: [], current: "把这段", injected: 0 },
  { committed: [], current: "把这段逻辑抽成", injected: 0 },
  { committed: [], current: "把这段逻辑抽成一个 helper", injected: 0 },
  {
    committed: [{ text: "把这段逻辑抽成一个 helper，", start_ms: 0, end_ms: 1600 }],
    current: "",
    injected: 0,
  },
  {
    committed: [{ text: "把这段逻辑抽成一个 helper，", start_ms: 0, end_ms: 1600 }],
    current: "然后在 session",
    injected: 0,
  },
  {
    committed: [{ text: "把这段逻辑抽成一个 helper，", start_ms: 0, end_ms: 1600 }],
    current: "然后在 session assembly 里复用",
    injected: 0,
  },
];

function hexFromRandom(random: () => number, bytes: number): string {
  let out = "";
  for (let i = 0; i < bytes; i += 1) {
    out += Math.floor(random() * 256)
      .toString(16)
      .padStart(2, "0");
  }
  return out;
}

function fingerprintOf(random: () => number): string {
  const hex = hexFromRandom(random, 8).toUpperCase();
  const groups = hex.match(/.{2}/g) ?? [];
  return `${groups.slice(0, 4).join(":")} · ${groups.slice(4, 8).join(":")}`;
}

/** The `voltip_platform::HostOs` a device platform maps to (phones gate nothing here). */
export function hostOsOf(platform: DeviceIdentityPublic["platform"]): HostOs {
  switch (platform) {
    case "macos":
      return "macos";
    case "windows":
      return "windows";
    case "linux":
      return "linux";
    default:
      return "other";
  }
}

/** The mock's default permission report: the happy path of each host. macOS has granted both,
 *  Windows has granted the microphone (its only gate), Linux and phones gate nothing. */
export function mockPermissions(host: HostOs): PermissionReport {
  const report = notApplicablePermissions(host);
  if (host === "macos") return { ...report, microphone: "granted", accessibility: "granted" };
  if (host === "windows") return { ...report, microphone: "granted" };
  return report;
}

export function desktopIdentity(): DeviceIdentityPublic {
  return {
    device_id: "3f9a0c27-11d0-4b8e-9a02-6b8e41a2c27b",
    name: "Surface-Laptop",
    platform: "windows",
    public_key: MOCK_PUBLIC_KEYS.desktop,
    fingerprint: "A7:C4:19:8E · 3D:F2:61:09",
  };
}

export function phoneIdentity(): DeviceIdentityPublic {
  return {
    device_id: "7d2e0c31-a1c4-4f02-9b08-f44e7b08f44e",
    name: "Pixel 8",
    platform: "android",
    public_key: MOCK_PUBLIC_KEYS.phone,
    fingerprint: "5B:0F:E2:91 · C3:7A:0D:44",
  };
}

export function phonePeer(): DeviceInfo {
  const id = phoneIdentity();
  return { device_id: id.device_id, name: id.name, platform: id.platform };
}

export function desktopPeer(): DeviceInfo {
  const id = desktopIdentity();
  return { device_id: id.device_id, name: id.name, platform: id.platform };
}

/** Deterministic pseudo-random generator (mulberry32) for reproducible codes in tests and demos. */
export function seededRandom(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** The browser has no OS hotkey; the mock reports a pretend registration so the settings page renders the real layout. */
/** What the browser preview reports as its hotkey backend: nothing is registered with the OS. */
export const MOCK_HOTKEY_BACKEND = "mock · browser preview (no OS hotkey)";

/** What the mock's hotkey can do: a Windows session, where every capability is there. */
export const MOCK_HOTKEY_CAPABILITIES: HotkeyCapabilities = {
  global: true,
  everywhere: true,
  hold: true,
  toggle_command: "voltip-desktop --toggle",
  edit_toggle_command: "voltip-desktop --edit-toggle",
  // A Windows session's lone keys (docs/dictation.md §13.1): no Fn, which only macOS has.
  solo_keys: SOLO_KEYS.filter((key) => key !== "fn"),
};

/** The lone-key half of the mock's hotkey status: watched when the session offers the key. */
function mockSoloStatus(key: SoloKey | null): Pick<HotkeyStatus, "solo_registered" | "solo_error"> {
  if (key === null) return {};
  return MOCK_HOTKEY_CAPABILITIES.solo_keys.includes(key)
    ? { solo_registered: key }
    : { solo_error: mockSoloKeyError(key) };
}

/** `HotkeyStatus.solo_error` for a lone key this session cannot watch (the desktop shell's text). */
export function mockSoloKeyError(key: SoloKey): string {
  return `${key}: 这台电脑上没有这个键`;
}

/** Microphones the browser preview pretends to have (the sample device first). */
export const MOCK_AUDIO_DEVICES: readonly AudioDevice[] = [
  {
    id: "Fifine K669 USB Microphone",
    name: "Fifine K669 USB Microphone",
    is_default: true,
    sample_rate_hz: 48_000,
    channels: 1,
  },
  {
    id: "Realtek(R) Audio",
    name: "Realtek(R) Audio",
    is_default: false,
    sample_rate_hz: 48_000,
    channels: 2,
  },
];
/** Output devices the browser preview pretends to have (docs/dictation.md §22), default first. */
export const MOCK_AUDIO_OUTPUTS: readonly AudioDevice[] = [
  {
    id: "Realtek(R) Audio Speakers",
    name: "扬声器 (Realtek(R) Audio)",
    is_default: true,
    sample_rate_hz: 48_000,
    channels: 2,
  },
  {
    id: "Sony WH-1000XM5",
    name: "Sony WH-1000XM5",
    is_default: false,
    sample_rate_hz: 48_000,
    channels: 2,
  },
];
/** Frame cadence of the synthetic meter (the native meter runs at 30 Hz too). */
export const MOCK_METER_INTERVAL_MS = 1000 / 30;

/** Deterministic breathing level in dBFS so screenshots and tests are stable. */
export function mockLevel(seq: number): {
  rms_dbfs: number;
  peak_dbfs: number;
} {
  const phase = seq / 18;
  const rms = -34 + 14 * Math.abs(Math.sin(phase)) + 3 * Math.abs(Math.sin(phase * 2.7));
  return { rms_dbfs: rms, peak_dbfs: rms + 8 };
}

const MOCK_MODIFIERS = new Set([
  "ctrl",
  "control",
  "alt",
  "option",
  "shift",
  "meta",
  "cmd",
  "command",
  "super",
  "win",
]);

/** The core's alias table for the modifiers (`voltip_core::Hotkey`), for comparing two chords. */
const MOCK_MODIFIER_NAMES: Readonly<Record<string, string>> = {
  ctrl: "ctrl",
  control: "ctrl",
  alt: "alt",
  option: "alt",
  shift: "shift",
  meta: "meta",
  cmd: "meta",
  command: "meta",
  super: "meta",
  win: "meta",
};

/** Mirrors `voltip_core::Hotkey::parse`: at least one modifier and exactly one key; the reason text
 *  on refusal. */
function mockChordProblem(hotkey: string): string | undefined {
  const parts = hotkey.split("+").map((p) => p.trim());
  const modifiers = parts.filter((p) => MOCK_MODIFIERS.has(p.toLowerCase()));
  const keys = parts.filter((p) => !MOCK_MODIFIERS.has(p.toLowerCase()));
  if (parts.some((p) => p.length === 0) || modifiers.length === 0 || keys.length !== 1)
    return "不是可用的组合键（需要至少一个修饰键和一个按键）";
  return undefined;
}

/** A chord's parts lower-cased, modifier aliases folded and sorted, for comparing two chords. */
function canonicalChord(hotkey: string): string {
  const parts = hotkey
    .split("+")
    .map((p) => p.trim().toLowerCase())
    .map((p) => MOCK_MODIFIER_NAMES[p] ?? p);
  parts.sort();
  return parts.join("+");
}

/** Mirrors `voltip_core::Hotkey::same_chord`: the same modifiers (aliases folded, any order) and
 *  the same key (case-insensitive). */
export function mockSameChord(a: string, b: string): boolean {
  return canonicalChord(a) === canonicalChord(b);
}

export class MockBackend implements Backend {
  private state: UiState;
  private readonly listeners = new Set<EventListener>();
  private readonly ttlSecs: number;
  private readonly autoPeer: MockPeer | false;
  private readonly expectedCode: string | undefined;
  private readonly now: () => number;
  private readonly random: () => number;
  private readonly role: "desktop" | "phone";
  private countdown: ReturnType<typeof setInterval> | undefined;
  private pending = new Set<ReturnType<typeof setTimeout>>();
  private dictationTimers = new Set<ReturnType<typeof setTimeout>>();
  /** Device start-up and streaming partials of the current take; cleared by stop / cancel. */
  private liveTimers = new Set<ReturnType<typeof setTimeout>>();
  private updateTimers = new Set<ReturnType<typeof setTimeout>>();
  /** One in-flight download timer per model id. */
  private modelTimers = new Map<string, ReturnType<typeof setTimeout>>();
  /** Activation bookkeeping (docs/dictation.md §13): when the hotkey press that started the
   *  current take landed (`hold_or_toggle` measures the hold against it), and the pending
   *  `extra_recording_ms` stop. */
  private pressedAt: number | undefined;
  private extraStop: ReturnType<typeof setTimeout> | undefined;
  /** The output mode the current take runs (decided at start, like the core, §12) and how many
   *  characters `live_inject` has pasted so far (what a cancel reports). */
  private takeMode: OutputMode = "whole_take";
  private injectedChars = 0;
  /** Voice edit (docs/dictation.md §19): what the kind of the current (or last) take is, the text
   *  the foreground application has selected (`setSelection`), and the selection the current
   *  edit take copied at its press. */
  private takeKind: TakeKind = "dictation";
  private selection: string | null = null;
  private copiedSelection: string | undefined;
  /** The fake foreground probe's answer (docs/dictation.md §18.2). */
  private foregroundApp: ForegroundApp | null;
  /** The current take's context and scene (decided at start, like the core; cleared at idle). */
  private takeContext: TakeContext | undefined;
  /** The paired phone the current take's audio comes from (docs/dictation.md §20). */
  private takeRemote: string | undefined;
  /** What `audioOutputs` answers. */
  private outputs: AudioOutputs;
  /** What a take on this computer records (docs/dictation.md §22), and a long take's recognition
   *  (`simulateLongTakeProgress`); both cleared at idle, the count already when the take ends. */
  private takeSource: RecordingSource | undefined;
  private takeSegments: SegmentProgress | undefined;
  private takeScene: Scene | undefined;
  /** The engines' preset and the custom presets as of the take's start (docs/dictation.md §21), and
   *  the preset the status names once the clean-up is under way. */
  private takePresetId: PresetId = "proofread";
  private takePresets: readonly CustomPreset[] = [];
  private takePreset: PresetRef | undefined;
  /** `live_error` of a scene's streaming mode the take cannot serve. */
  private takeModeError: string | undefined;
  /** The take's kept recording (docs/dictation.md §26), and whether the take recognises one
   *  again; the status carries both until it is idle. */
  private takeRecording: string | undefined;
  private takeRetake = false;
  /** docs/dictation.md §28: the shell shows result cards (the desktop's pill; the phone has none). */
  private readonly resultCards: boolean;
  /** The result card that stays (§28), with the dwell it had for 识别结果卡片常驻 switched off. */
  private pinned: { ms: number; session: number } | undefined;
  /** What the next paste take's insert check finds (§28); `undefined` = no check. */
  private insertCheck: InsertCheck | undefined;
  /** Every text the result card's 复制 and 粘贴 handed on, in order (tests). */
  readonly overlayCopies: string[] = [];
  readonly overlayPastes: string[] = [];
  /** How often the result card's 打开历史 ran (tests). */
  historyOpens = 0;
  private meters = new Set<ReturnType<typeof setInterval>>();
  private permissions: PermissionReport | (() => PermissionReport);
  private readonly preflight: InjectPreflight;
  private pasteOutcome: PasteOutcome;
  /** Every text a `pasteText` handed on, in order (tests). */
  readonly pastes: string[] = [];
  /** Every `permissionsRequest` made, in order (tests). */
  readonly permissionRequests: Permission[] = [];
  /** Secret-store entries holding a user key (`keyEntry`). */
  private readonly userKeys = new Set<string>();
  private readonly builtIn: { asr?: BuiltInService; llm?: BuiltInService };
  private readonly probeModels: Partial<Record<ProviderId, readonly string[]>> | false;
  private readonly engineOverrides: Partial<EngineStatus>;
  /** docs/dictation.md §3.5: the selected model of a service that ran out of quota, and when it
   *  is tried again (`simulateQuotaExhausted`); `engines_quota_reset` forgets it. */
  private readonly quotaOut = new Map<ServiceKind, number>();
  /** Every `providerConsoleOpen` of a key page, in order (tests). */
  readonly consolesOpened: ProviderId[] = [];
  /** The site of each of `consolesOpened`, `undefined` for none (tests). */
  readonly consoleSites: (SiteId | undefined)[] = [];
  /** Every `providerConsoleOpen` of a quota page, in order (tests). */
  readonly quotaPagesOpened: ProviderId[] = [];
  /** How many `provider_models_refresh` requests left (tests). */
  modelRefreshes = 0;
  /** `project_link_open` calls, for tests. */
  readonly linksOpened: ProjectLink[] = [];
  /** `guide_open` calls (page and language), for tests. */
  readonly guidesOpened: { page: GuidePage; locale: string }[] = [];
  /** `model_folder_open` calls (model ids), for tests. */
  readonly modelFoldersOpened: string[] = [];
  /** What the phone's `update_install` opened: `store` for the listing, else the release's version. */
  readonly updatePagesOpened: string[] = [];
  /** `relay_reconnect` calls (docs/pairing.md 「重连」), for tests. */
  relayChecks = 0;
  /** `model_link_open` calls, the addresses opened, for tests. */
  readonly modelLinksOpened: string[] = [];
  /** What imports find wrong per model id (`simulateImportProblems`). */
  private readonly importProblems = new Map<string, { missing: string[]; mismatched: string[] }>();
  /** Reports `feedbackSubmit` accepted, in order. */
  readonly feedbackSent: FeedbackDraft[] = [];
  /** The files staged for the next report (`feedback_attachment_add`), in order. */
  readonly feedbackStaged: StagedAttachment[] = [];
  /** The last staged file's number. */
  private lastAttachment = 0;
  /** The phone's clipboard (`phone_clipboard_read`; a take the phone recognised and a copy from its
   *  history land here). */
  phoneClipboard: string | null;
  /** Texts the phone handed to the share sheet (`phone_share_text`), in order. */
  readonly shared: string[] = [];
  /** The last id a phone text took (`SentTexts::next_id` in the core). */
  private lastTextId = 0;
  private readonly feedback: "configured" | FeedbackError;
  private readonly probeTimers = new Set<ReturnType<typeof setTimeout>>();
  /** The models each provider listed for its saved endpoint and key, by `provider:kind`
   *  (docs/dictation.md §3.3): `provider_models_refresh` and a 测试连接 with the saved values. */
  private readonly listedModels = new Map<`${ProviderId}:${ServiceKind}`, ListedModels>();
  /** `history_process` requests running (docs/dictation.md §22). */
  private readonly processing = new Map<
    number,
    { id: string; timer: ReturnType<typeof setTimeout> }
  >();
  /** Every event emitted, oldest first; handy for asserting ordering in tests. */
  readonly log: UiEvent[] = [];
  /** What each queued import was asked for and how far it got (docs/dictation.md §31), by job. */
  private readonly importRuns = new Map<string, MockImportRun>();
  /** The running import's next step, and its job. */
  private importTimer: ReturnType<typeof setTimeout> | undefined;
  private importRunning: string | undefined;
  /** What paths hold for the import (`simulateImportFile`). */
  private readonly importFiles = new Map<string, MockImportFile>();
  /** What the next `importPick` picks (`simulateImportPick`). */
  private nextImportPick: string[] | undefined;
  private readonly dropListeners = new Set<DropListener>();
  /** Every open dialog `importPick` showed, by its title and filter names (tests). */
  readonly importPicks: { title: string; media: string; all: string }[] = [];
  /** The whole history, newest first: what the core keeps in `history.sqlite3`. The state holds
   *  its newest `HISTORY_RECENT` and the total, as `UiState` does (docs/dictation.md §4.4). */
  private history: HistoryEntry[] = [];
  /** The accepted suggestions 撤销 can still take back (docs/dictation.md §16.10), oldest first. */
  private readonly suggestionUndos = new Map<
    string,
    { before?: DictionaryEntry; after: DictionaryEntry }
  >();

  constructor(options: MockBackendOptions = {}) {
    this.ttlSecs = options.ttlSecs ?? 120;
    this.autoPeer = options.autoPeer ?? false;
    this.role = options.role ?? "desktop";
    this.expectedCode = options.expectedCode;
    this.now = options.now ?? (() => Date.now());
    this.random = options.random ?? seededRandom(0x5eed);
    const identity =
      options.identity ?? (this.role === "phone" ? phoneIdentity() : desktopIdentity());
    // The built-in service is compiled in for the preview unless a test says otherwise.
    this.builtIn = options.builtIn ?? {
      asr: { model: MOCK_ENGINE_BUILTIN.asr_model, key: true },
      llm: {
        model: MOCK_ENGINE_BUILTIN.refine_model,
        models: [MOCK_ENGINE_BUILTIN.refine_model, "openai/gpt-oss-120b", "openai/gpt-oss-20b"],
        key: true,
      },
    };
    for (const { provider, kind } of options.providerKeys ?? []) {
      const entry = keyEntry(provider, kind);
      if (entry !== undefined) this.userKeys.add(entry);
    }
    this.probeModels = options.probeModels ?? {};
    this.feedback = options.feedback ?? "configured";
    this.phoneClipboard =
      options.phoneClipboard === undefined ? MOCK_PHONE_CLIPBOARD : options.phoneClipboard;
    this.outputs =
      options.audioOutputs ??
      (this.role === "phone"
        ? { system_audio: { state: "unsupported" }, devices: [] }
        : {
            system_audio: { state: "available" },
            devices: MOCK_AUDIO_OUTPUTS.map((d) => ({ ...d })),
          });
    this.engineOverrides = options.engines ?? {};
    this.foregroundApp = options.foregroundApp ?? null;
    const host = hostOsOf(identity.platform);
    this.permissions = options.permissions ?? mockPermissions(host);
    this.preflight = options.injectPreflight ?? uncheckedPreflight(host);
    this.pasteOutcome = options.pasteOutcome ?? { kind: "pasted" };
    this.resultCards = this.role !== "phone";
    this.insertCheck = options.insertCheck;
    // The phone's own history starts empty: it holds the takes it recognises itself (§20.7); the
    // samples are a computer's dictations.
    this.history = options.history ?? (this.role === "phone" ? [] : sampleHistory(this.now()));
    const settings: Settings = { ...defaultSettings(), ...options.settings };
    // The phone has no local models (docs/dictation.md §10): an empty catalogue, commands refused.
    const models: ModelState[] =
      this.role === "phone"
        ? []
        : MOCK_MODEL_CATALOGUE.map((row) => ({
            ...row,
            active: false,
            state: options.models?.[row.id] ?? { kind: "not_installed" },
          }));
    this.state = {
      identity,
      settings,
      secret_backend: this.role === "phone" ? "android-keystore" : "credential-manager",
      app_version: MOCK_CURRENT_VERSION,
      relay: options.relay ?? {
        state: "disconnected",
        attempts: 0,
        source: "none",
      },
      pairing: idleSnapshot(),
      devices: options.devices ?? [],
      hotkey: {
        ...emptyHotkeyStatus(),
        registered: options.settings?.hotkey ?? DEFAULT_HOTKEY,
        ...(settings.edit_hotkey === null ? {} : { edit_registered: settings.edit_hotkey }),
        ...(this.role === "phone" ? {} : mockSoloStatus(settings.solo_key)),
        ...(this.role === "phone" || settings.copy_last_hotkey == null
          ? {}
          : { copy_last_registered: settings.copy_last_hotkey }),
        backend: MOCK_HOTKEY_BACKEND,
        ...(this.role === "phone" ? {} : { capabilities: { ...MOCK_HOTKEY_CAPABILITIES } }),
      },
      dictation: idleDictation(),
      sent_texts: [],
      history_recent: this.history.slice(0, HISTORY_RECENT),
      history_total: this.history.length,
      // The phone keeps no recordings (docs/dictation.md §26).
      audio_cache: cacheView(this.role !== "phone", structuredClone(options.audioCache ?? [])),
      // The phone imports no files (docs/dictation.md §31).
      imports: {
        available: this.role !== "phone",
        extensions: this.role === "phone" ? [] : [...MOCK_IMPORT_EXTENSIONS],
        jobs: structuredClone(options.imports ?? []),
      },
      engines: emptyEngineStatus(),
      update: options.update ?? idleUpdate(),
      models,
      models_location:
        this.role === "phone"
          ? emptyModelsLocation()
          : {
              available: true,
              dir: MOCK_MODELS_ROOT,
              custom: false,
              default_dir: MOCK_MODELS_ROOT,
              moving: null,
              last: null,
              ...options.modelsLocation,
            },
      // The phone makes no automatic backups (docs/dictation.md §4.6.1).
      auto_backup:
        this.role === "phone"
          ? emptyAutoBackup()
          : {
              available: true,
              folder: settings.backup.folder ?? MOCK_BACKUPS_DIR,
              default_folder: MOCK_BACKUPS_DIR,
              custom: settings.backup.folder !== null,
              last: null,
              failure: null,
              notice: null,
              ...options.autoBackup,
            },
      dictionary: [...(options.dictionary ?? [])],
      rules: [...(options.rules ?? [])],
      vocabulary_ignored: [...(options.vocabularyIgnored ?? [])],
      // The desktop's list always holds the built-in scenes, appended off after the user's (§18.10).
      scenes: [...(options.scenes ?? [])],
      presets: [...(options.presets ?? [])],
      // What the desktop shell reports about the machine (§10.6); nothing on the phone.
      hardware:
        this.role === "phone" ? { cpu_threads: 0, gpus: [] } : (options.hardware ?? MOCK_HARDWARE),
      connectivity: { running: false },
      mirrors: (options.mirrors ?? []).map((m) => structuredClone(m.view)),
      phone_outbox_too_large: [],
      serve: options.serve ?? this.serveStatusFor(settings.serve),
      network: { ...(options.network ?? { proxy_password: false }) },
      mcp: options.mcp ?? this.mcpStatusFor(settings.mcp),
      usage: {
        ...emptyUsageView(),
        builtin: this.builtIn.asr !== undefined || this.builtIn.llm !== undefined,
        ...structuredClone(options.usage ?? {}),
      },
    };
    this.proxyPassword = options.network?.proxy_password === true ? "saved" : undefined;
    for (const m of options.mirrors ?? []) this.mirrors.set(m.view.desktop, structuredClone(m));
    this.state.engines = this.resolveEngines(settings.engines);
    this.state.models = this.modelsFor(settings.engines);
    this.state.scenes = this.withBuiltinScenes(this.state.scenes);
  }

  getState(): Promise<UiState> {
    return Promise.resolve(structuredClone(this.state));
  }

  on(listener: EventListener): Unsubscribe {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  /** Current cached state (synchronous; tests only). */
  peek(): UiState {
    return structuredClone(this.state);
  }

  audioDevices(): Promise<AudioDevice[]> {
    return Promise.resolve(MOCK_AUDIO_DEVICES.map((d) => ({ ...d })));
  }

  audioOutputs(): Promise<AudioOutputs> {
    return Promise.resolve(structuredClone(this.outputs));
  }

  /** Synthetic meter: a breathing level on the requested device, 30 frames a second. A device
   *  that is not connected meters the default input, as the desktop shell does (2026-09-28: the
   *  chosen microphone may be unplugged). The phone's shell opens no microphone to meter: its
   *  frames are those of its own take — streamed to a computer, or recognised on the phone
   *  (§20.7) — so the phone role only sends them while one listens. */
  meter(deviceId: string | undefined, onFrame: FrameListener): Promise<Unsubscribe> {
    const phone = this.role === "phone";
    const device =
      MOCK_AUDIO_DEVICES.find((d) => d.id === deviceId) ??
      MOCK_AUDIO_DEVICES.find((d) => d.is_default);
    let seq = 0;
    const timer = setInterval(() => {
      const listening =
        this.state.phone_take?.state.state === "listening" ||
        this.state.dictation.phase.phase === "listening";
      if (phone && !listening) return;
      seq += 1;
      const level = mockLevel(seq);
      const frame: LevelFrame = {
        ...level,
        clipping: level.peak_dbfs >= -0.1,
        sample_rate_hz: device?.sample_rate_hz ?? 48_000,
        channels: device?.channels ?? 1,
        seq,
      };
      onFrame(frame);
    }, MOCK_METER_INTERVAL_MS);
    this.meters.add(timer);
    return Promise.resolve(() => {
      clearInterval(timer);
      this.meters.delete(timer);
    });
  }

  /** Number of meters currently streaming (tests). */
  activeMeters(): number {
    return this.meters.size;
  }

  /** `vocabulary_preview` with the core's semantics (regex rules on the JavaScript engine). */
  async vocabularyPreview(text: string, draft?: PreviewDraft): Promise<VocabularyPreview> {
    await Promise.resolve();
    return previewVocabulary(this.state.dictionary, this.state.rules, text, draft);
  }

  /** `vocabulary_backtest` (docs/dictation.md §16.9) with the core's semantics over the newest
   *  recognised takes in memory: dictations only, no text a phone sent (`recent_texts`). */
  async vocabularyBacktest(args: VocabularyBacktestArgs): Promise<VocabularyBacktest> {
    await Promise.resolve();
    const limit = args.limit ?? BACKTEST_LIMIT;
    if (!Number.isInteger(limit) || limit < 1 || limit > MAX_BACKTEST_LIMIT) {
      throw new Error(`vocabulary_backtest: limit 1–${MAX_BACKTEST_LIMIT}`);
    }
    const exclude = args.historyId == null ? undefined : uuidArg(args.historyId);
    const takes = recentTextsOf(this.history, limit);
    return backtestVocabulary(this.state.dictionary, takes, args.draft, exclude);
  }

  /** `vocabulary_suggestions` (docs/dictation.md §16.10) with the core's semantics over the newest
   *  recognised takes in memory; `history_off` while history recording is off. */
  async vocabularySuggestions(limit?: number): Promise<SuggestionsAnswer> {
    await Promise.resolve();
    const max = limit ?? MAX_SUGGESTIONS;
    if (!Number.isInteger(max) || max < 1 || max > MAX_SUGGESTIONS) {
      throw new Error(`vocabulary_suggestions: limit 1–${MAX_SUGGESTIONS}`);
    }
    if (!this.state.settings.history.enabled) return { status: "history_off" };
    const takes = recentTextsOf(this.history, MAX_BACKTEST_LIMIT);
    const { dictionary, rules, vocabulary_ignored: ignored } = this.state;
    return suggestVocabulary(dictionary, rules, takes, ignored, max);
  }

  /** 加入词典 on a suggestion, as the core checks and saves it; the undo is kept in memory. */
  async vocabularySuggestionAccept(args: SuggestionAcceptArgs): Promise<AcceptOutcome> {
    await Promise.resolve();
    const [heard, polished] = validateSuggestionPair(args.heard, args.polished);
    const historyId = uuidArg(args.historyId);
    const { dictionary, rules } = this.state;
    const now = this.now();
    const result = acceptSuggestion(dictionary, rules, heard, polished, historyId, now, () =>
      this.uuid(),
    );
    if ("kind" in result) return { kind: "refused", refusal: result };
    checkDictionary(result.dictionary);
    this.emit({ type: "dictionary", entries: result.dictionary });
    const id = this.uuid();
    this.suggestionUndos.set(id, { before: result.before, after: result.after });
    for (const old of this.suggestionUndos.keys()) {
      if (this.suggestionUndos.size <= MAX_SUGGESTION_UNDOS) break;
      this.suggestionUndos.delete(old);
    }
    return {
      kind: "accepted",
      undo: {
        id,
        entry_id: result.after.id,
        term: result.after.term,
        created: result.before === undefined,
        previous_heard_as: result.before?.heard_as ?? [],
      },
    };
  }

  /** 撤销 of an accepted suggestion: once, while the entry is as the accept left it. */
  async vocabularySuggestionUndo(id: string): Promise<UndoOutcome> {
    await Promise.resolve();
    const record = this.suggestionUndos.get(uuidArg(id));
    if (record === undefined) return { kind: "expired" };
    this.suggestionUndos.delete(id);
    const current = this.state.dictionary.find((e) => e.id === record.after.id);
    if (current === undefined) return { kind: "gone" };
    if (JSON.stringify(current) !== JSON.stringify(record.after)) return { kind: "changed" };
    const { before } = record;
    const entries =
      before === undefined
        ? this.state.dictionary.filter((e) => e.id !== record.after.id)
        : this.state.dictionary.map((e) => (e.id === before.id ? before : e));
    this.emit({ type: "dictionary", entries });
    return { kind: "undone" };
  }

  /** `rules_warnings`: the rules page's hints with the core's semantics. */
  async rulesWarnings(): Promise<RuleWarning[]> {
    await Promise.resolve();
    return ruleWarnings(this.state.rules, this.state.dictionary);
  }

  /** `rules_export`: the rules as the core's TOML text. */
  async rulesExport(): Promise<string> {
    await Promise.resolve();
    return exportRulesToml(this.state.rules);
  }

  /** `recent_apps`: the apps the history saw, newest first (none on the phone, which names no
   *  application). */
  async recentApps(): Promise<AppRef[]> {
    await Promise.resolve();
    return recentAppsOf(this.history);
  }

  /** `history_query`: the core's filters, search and order over the whole list (the phone's is
   *  the takes it recognised itself, docs/dictation.md §20.7). */
  async historyQuery(args: HistoryQueryArgs): Promise<HistoryPage> {
    await Promise.resolve();
    return historyPageOf(this.history, args);
  }

  /** `history_export` (docs/dictation.md §22): the preview's save dialog takes the offered name,
   *  the phone's share sheet opens (`shared`); what was asked for is kept in `exports`. */
  async historyExport(
    id: string,
    format: ExportFormat,
    fileName: string,
    subtitles?: SegmentText,
  ): Promise<ExportOutcome> {
    await Promise.resolve();
    const entry = this.history.find((e) => e.id === id);
    if (entry === undefined)
      return {
        kind: "failed",
        code: "gone",
        detail: "the entry is not in the history",
      };
    if (format === "srt" && !(entry.segments ?? []).some((s) => s.text.trim().length > 0))
      return {
        kind: "failed",
        code: "empty",
        detail: "the entry has no segments",
      };
    this.exports.push({ id, format, fileName, ...(subtitles === undefined ? {} : { subtitles }) });
    if (this.role === "phone") return { kind: "shared" };
    return { kind: "saved", path: `${MOCK_EXPORT_DIR}/${fileName}.${format}` };
  }

  /** Every `historyExport` that saved or was shared, in order (tests). */
  readonly exports: {
    id: string;
    format: ExportFormat;
    fileName: string;
    subtitles?: SegmentText;
  }[] = [];

  // ---- backup and migration (docs/dictation.md §4.6) -------------------------------------------

  /** The backups `backupExport` wrote, oldest first: the preview's files. */
  private readonly backupFiles: MockBackupFile[] = [];
  /** What the next `backupOpen` finds instead (tests and the dev pages). */
  private nextBackupOpen: MockBackupFile | BackupOpenOutcome | undefined;
  /** The backup the open dialog read, under its preview's token. */
  private openedBackup: { token: string; file: MockBackupFile } | undefined;
  /** The file names `backupExport` saved under, in order (tests). */
  readonly backupExports: string[] = [];

  /** `backup_export`: the save dialog takes the offered name; the file holds every part as the
   *  state and the history hold it. The phone has no backups. */
  async backupExport(fileName: string): Promise<BackupExportOutcome> {
    await Promise.resolve();
    if (this.role === "phone") return { kind: "failed", code: "unsupported", detail: "" };
    const file = this.backupFile();
    this.backupFiles.push(file);
    this.backupExports.push(fileName);
    return {
      kind: "saved",
      path: `${MOCK_EXPORT_DIR}/${fileName}.${BACKUP_EXTENSION}`,
      counts: file.manifest.counts,
      bytes: JSON.stringify(file).length,
    };
  }

  /** The next `backupOpen` finds `next` (a file, or what the dialog answers: `cancelled`, a
   *  refusal). Without one it finds the last backup exported, else another computer's sample. */
  simulateBackupFile(next: MockBackupFile | BackupOpenOutcome) {
    this.nextBackupOpen = structuredClone(next);
  }

  /** `backup_open`: the preview of the file the open dialog picked; nothing changes yet. */
  async backupOpen(_title: string): Promise<BackupOpenOutcome> {
    await Promise.resolve();
    if (this.role === "phone") return { kind: "failed", code: "unsupported", detail: "" };
    this.openedBackup = undefined;
    const next = this.nextBackupOpen;
    this.nextBackupOpen = undefined;
    if (next !== undefined && "kind" in next) return next;
    const file = next ?? this.backupFiles.at(-1) ?? sampleBackup(this.now());
    const token = this.uuid();
    this.openedBackup = { token, file: structuredClone(file) };
    return { kind: "preview", token, manifest: structuredClone(file.manifest) };
  }

  /** `backup_import`: the parts restored as the core restores them (`./backup-merge`), each list's
   *  event and the history's after a change; a token works once. */
  async backupImport(
    token: string,
    parts: BackupParts,
    mode: ImportMode,
  ): Promise<BackupImportOutcome> {
    await Promise.resolve();
    if (this.role === "phone") return { kind: "failed", code: "unsupported" };
    const opened = this.openedBackup;
    if (opened?.token !== token) return { kind: "failed", code: "expired" };
    // docs/dictation.md §4.6.1: what a replacing import would replace is backed up first; nothing
    // is replaced while a take runs or when that backup cannot be written, and the preview stays
    // for another try.
    let snapshot: string | undefined;
    if (mode === "replace" && Object.values(parts).some(Boolean)) {
      const phase = this.state.dictation.phase.phase;
      if (phase === "listening" || phase === "processing") return { kind: "failed", code: "busy" };
      const written = this.writeAutoBackup("import");
      if (written.kind === "failed")
        return { kind: "failed", code: "snapshot_failed", detail: written.detail };
      snapshot = written.path;
    }
    // As in the shell, the backup is let go of as the import runs: a token works once.
    this.openedBackup = undefined;
    const { file } = opened;
    const reports: BackupPartReport[] = [];
    const changed = (report: BackupPartReport) => mode === "replace" || report.restored > 0;
    let samePreset = new Map<string, string>();
    if (parts.presets) {
      const merged = mergePresets(this.state.presets, file.presets, mode);
      samePreset = merged.same;
      reports.push(merged.report);
      if (changed(merged.report)) this.emit({ type: "presets", presets: merged.list });
    }
    if (parts.scenes) {
      const merged = mergeScenes(this.state.scenes, file.scenes, mode, samePreset);
      reports.push(merged.report);
      if (changed(merged.report))
        this.emit({ type: "scenes", scenes: this.withBuiltinScenes(merged.list) });
    }
    if (parts.dictionary) {
      const merged = mergeDictionary(this.state.dictionary, file.dictionary, mode);
      reports.push(merged.report);
      if (changed(merged.report)) this.emit({ type: "dictionary", entries: merged.list });
    }
    if (parts.rules) {
      const merged = mergeRules(this.state.rules, file.rules, mode);
      reports.push(merged.report);
      if (changed(merged.report)) this.emit({ type: "rules", rules: merged.list });
    }
    if (parts.history) {
      const merged = mergeHistory(
        this.history,
        file.history,
        mode,
        this.state.settings.history.keep,
      );
      reports.push(merged.report);
      if (changed(merged.report) || merged.report.removed > 0) this.setHistory(merged.list);
    }
    return {
      kind: "done",
      report: { mode, parts: reports, ...(snapshot === undefined ? {} : { snapshot }) },
    };
  }

  /** `backup_discard`: the preview was closed. */
  async backupDiscard(token: string): Promise<void> {
    await Promise.resolve();
    if (this.openedBackup?.token === token) this.openedBackup = undefined;
  }

  // ---- automatic backups (docs/dictation.md §4.6.1) --------------------------------------------

  /** The automatic backups the preview wrote, newest first, the newest `AUTO_BACKUP_KEEP` (tests). */
  readonly autoBackups: AutoBackupFile[] = [];
  /** `backup_folder_open` calls: the folder each showed (tests). */
  readonly backupFoldersOpened: string[] = [];
  /** What the next 自动备份位置 dialog answers: a folder, `null` (closed), or a folder that
   *  cannot be written. */
  private nextBackupFolder: string | null | { unwritable: string } = MOCK_CHOSEN_BACKUPS_DIR;
  /** The automatic backups fail with this message until cleared (tests). */
  private autoBackupFailure: string | undefined;

  /** What the next 更改… dialog answers (tests): a folder, `null` (closed), or `{ unwritable }`. */
  simulateBackupFolder(next: string | null | { unwritable: string }) {
    this.nextBackupFolder = next;
  }

  /** The automatic backups fail with `detail` (the system's message) until `undefined` (tests). */
  simulateAutoBackupFails(detail: string | undefined) {
    this.autoBackupFailure = detail;
  }

  /** The week's backup as the core makes it once one is due (tests and the dev pages). */
  simulateWeeklyBackup() {
    this.writeAutoBackup("weekly");
  }

  /** `backup_folder_choose`: the folder dialog, then a trial file in the folder chosen. */
  backupFolderChoose(_title: string): Promise<BackupFolderOutcome> {
    if (this.role === "phone") return Promise.reject(new Error("backup: 手机端不提供自动备份"));
    const next = this.nextBackupFolder;
    if (next === null) return Promise.resolve({ kind: "cancelled" });
    if (typeof next !== "string")
      return Promise.resolve({ kind: "failed", detail: next.unwritable });
    this.setBackupFolder(next);
    return Promise.resolve({ kind: "chosen", folder: next });
  }

  /** `backup_folder_open`: the automatic backups' folder in the file manager. */
  backupFolderOpen(): Promise<void> {
    if (this.role === "phone") return Promise.reject(new Error("backup: 手机端不提供自动备份"));
    this.backupFoldersOpened.push(this.state.auto_backup.folder);
    return Promise.resolve();
  }

  private emitAutoBackup(change: Partial<AutoBackupView>) {
    this.emit({ type: "auto_backup", ...this.state.auto_backup, ...change });
  }

  /** As the core sets it: the default folder is saved as none; a change forgets the folder's
   *  backups and its failure until the new folder is looked at. */
  private setBackupFolder(folder: string | null) {
    const saved = folder === MOCK_BACKUPS_DIR ? null : folder;
    if (saved === this.state.settings.backup.folder) return;
    this.emit({
      type: "settings",
      ...this.state.settings,
      backup: { ...this.state.settings.backup, folder: saved },
    });
    this.emitAutoBackup({
      folder: saved ?? MOCK_BACKUPS_DIR,
      custom: saved !== null,
      last: null,
      failure: null,
    });
  }

  /** One automatic backup of `kind`, named as the core names it, the newest five kept; a failure
   *  is shown until one succeeds. */
  private writeAutoBackup(
    kind: AutoBackupKind,
  ): { kind: "written"; path: string } | { kind: "failed"; detail: string } {
    const at = this.now();
    if (this.autoBackupFailure !== undefined) {
      const failure = { kind, at_ms: at, detail: this.autoBackupFailure };
      // The update's failure shows on home after the restart; the preview does not restart.
      this.emitAutoBackup({ failure, ...(kind === "update" ? { notice: failure } : {}) });
      return { kind: "failed", detail: this.autoBackupFailure };
    }
    const file: AutoBackupFile = { name: autoBackupName(kind, at), at_ms: at, kind };
    this.autoBackups.unshift(file);
    this.autoBackups.splice(AUTO_BACKUP_KEEP);
    this.emitAutoBackup({ last: file, failure: null });
    return { kind: "written", path: `${this.state.auto_backup.folder}/${file.name}` };
  }

  /** What `backupExport` writes. */
  private backupFile(): MockBackupFile {
    const content = structuredClone({
      history: this.history,
      dictionary: this.state.dictionary,
      rules: this.state.rules,
      scenes: this.state.scenes,
      presets: this.state.presets,
    });
    return {
      manifest: {
        format: "voltip-backup",
        version: 1,
        app_version: this.state.app_version,
        created_at_ms: this.now(),
        device: this.state.identity?.name ?? "",
        platform: this.state.identity?.platform ?? "other",
        counts: backupCounts(content),
      },
      ...content,
    };
  }

  /** `history_entry`: one entry, `null` once it is gone. */
  async historyEntry(id: string): Promise<HistoryEntry | null> {
    await Promise.resolve();
    return this.history.find((e) => e.id === id) ?? null;
  }

  /** The phone's copies by computer key (docs/dictation.md §20.8). */
  private readonly mirrors = new Map<string, MockMirror>();

  /** `mirror_history_query`: a page of the copy, read like the phone's own; empty without one. */
  async mirrorHistoryQuery(desktop: string, args: HistoryQueryArgs): Promise<HistoryPage> {
    await Promise.resolve();
    return historyPageOf(this.mirrors.get(desktop)?.history ?? [], args);
  }

  /** `mirror_history_entry`: one entry of the copy and whether it arrived shortened. */
  async mirrorHistoryEntry(desktop: string, id: string): Promise<MirrorEntry | null> {
    await Promise.resolve();
    const mirror = this.mirrors.get(desktop);
    const entry = mirror?.history.find((e) => e.id === id);
    if (mirror === undefined || entry === undefined) return null;
    return {
      entry: structuredClone(entry),
      shortened: (mirror.shortened ?? []).includes(id),
    };
  }

  /** `mirror_profile`: the computer's settings as the copy holds them. */
  async mirrorProfile(desktop: string): Promise<MirrorProfile | null> {
    await Promise.resolve();
    return structuredClone(this.mirrors.get(desktop)?.profile ?? null);
  }

  /** The core reported its copies (tests; replaces a copy's view and, when given, its content). */
  simulateMirror(mirror: MockMirror) {
    this.mirrors.set(mirror.view.desktop, structuredClone(mirror));
    this.emit({
      type: "mirrors",
      mirrors: [...this.mirrors.values()].map((m) => structuredClone(m.view)),
    });
  }

  /** The core reported records too large to upload (tests). */
  simulateTooLarge(ids: readonly string[]) {
    this.emit({ type: "phone_outbox", too_large: [...ids] });
  }

  /** From now on a take's clean-up fails with `failure` (`null`: it works again), as the core
   *  reports it on `done` and in the history (tests and the dev pages, docs/dictation.md §3.6). */
  simulateRefineFailure(failure: RefineFailure | null) {
    this.refineFailure = failure ?? undefined;
  }

  /** From now on another window is in front when a take's text is ready (`false`: the same one
   *  again): with `switched_window = copy_only` the text is copied, and `done`, the history and the
   *  longer dwell say so, as the core does (tests and the dev pages, docs/dictation.md §14.3). */
  simulateWindowSwitch(switched: boolean) {
    this.windowSwitched = switched;
  }

  /** The lone key's input hook went away and the shell is bringing it back (`loss`), or it came
   *  back at `atMs` (`null`), as the desktop shell reports it (tests and the dev pages,
   *  docs/dictation.md §29). */
  simulateSoloHook(loss: SoloLoss | null, atMs: number = this.now()) {
    const { solo_loss: _was, ...hotkey } = this.state.hotkey;
    const key = this.state.settings.solo_key;
    this.emit(
      loss === null
        ? {
            type: "hotkey",
            ...hotkey,
            ...(key === null ? {} : { solo_registered: key }),
            solo_pressed: false,
            solo_recovered_at: atMs,
          }
        : { type: "hotkey", ...hotkey, solo_loss: loss, solo_pressed: false },
    );
  }

  /** The built-in clean-up service turned a take down (`null`: the notice went; tests and the dev
   *  pages, docs/dictation.md §3.6). */
  simulateRefineNotice(failure: RefineFailure | null) {
    this.emit({
      type: "refine_notice",
      notice: failure === null ? null : { failure, at_ms: Date.now() },
    });
  }

  /** `history_stats`: the dictations between the page's local midnights, and in total. */
  async historyStats(boundaries: readonly number[]): Promise<HistoryStats> {
    await Promise.resolve();
    return historyStatsOf(this.history, boundaries);
  }

  /** `history_hits`: hits per dictionary entry and rule over the whole history. */
  async historyHits(): Promise<HistoryHits> {
    await Promise.resolve();
    return historyHitsOf(this.history);
  }

  /** Replace the whole history and tell the UI its newest entries and the total. */
  private setHistory(entries: HistoryEntry[]) {
    this.history = entries;
    this.emit({
      type: "history",
      recent: entries.slice(0, HISTORY_RECENT),
      total: entries.length,
    });
  }

  /** The fake foreground probe (docs/dictation.md §18.2): what the next take finds in front
   *  (`null` = no answer). */
  setForegroundApp(app: ForegroundApp | null): void {
    this.foregroundApp = app;
  }

  updateStatus(): Promise<UpdateStatus> {
    return Promise.resolve(structuredClone(this.state.update));
  }

  /** The seeded report, or the seeded probe's answer (a throwing probe rejects, like a failed IPC). */
  permissionsStatus(): Promise<PermissionReport> {
    try {
      const report = typeof this.permissions === "function" ? this.permissions() : this.permissions;
      return Promise.resolve(structuredClone(report));
    } catch (e: unknown) {
      return Promise.reject(e instanceof Error ? e : new Error(String(e)));
    }
  }

  /** Records the request; with a static report the user is assumed to accept, so the permission
   *  reads `granted` from the next poll on (a `not_applicable` one stays as it is). */
  permissionsRequest(permission: Permission): Promise<void> {
    this.permissionRequests.push(permission);
    if (
      typeof this.permissions !== "function" &&
      this.permissions[permission] !== "not_applicable"
    ) {
      this.permissions = { ...this.permissions, [permission]: "granted" };
    }
    return Promise.resolve();
  }

  /** Replace the seeded permission report (tests: the OS state changed under the poller). */
  setPermissions(report: PermissionReport | (() => PermissionReport)): void {
    this.permissions = report;
  }

  injectPreflight(): Promise<InjectPreflight> {
    return Promise.resolve(structuredClone(this.preflight));
  }

  /** `paste_text`: the refusals of the shell and the core (empty or too long text; a take under
   *  way, not queued), then the seeded outcome. The phone has no window to paste into: it puts the
   *  text on its clipboard (docs/dictation.md §20.7). Writes no history. */
  pasteText(text: string): Promise<PasteOutcome> {
    if (text.trim().length === 0 || Array.from(text).length > MAX_PASTE_TEXT_CHARS) {
      return pasteFailed("invalid");
    }
    if (this.role === "phone") {
      this.phoneClipboard = text;
      return Promise.resolve({ kind: "copied", reason: "clipboard_only" });
    }
    const phase = this.state.dictation.phase.phase;
    if (phase === "listening" || phase === "processing") return pasteFailed("busy");
    this.pastes.push(text);
    return Promise.resolve(structuredClone(this.pasteOutcome));
  }

  /** What the next valid paste ends with (tests: the window changed, the paste fell back). */
  setPasteOutcome(outcome: PasteOutcome): void {
    this.pasteOutcome = outcome;
  }

  /** `overlay_copy` (docs/dictation.md §28): the card's text onto the (fake) clipboard. */
  overlayCopy(text: string): Promise<void> {
    if (this.role === "phone") return Promise.reject(new Error("overlay_copy: 手机没有悬浮窗"));
    this.overlayCopies.push(text);
    return Promise.resolve();
  }

  /** `overlay_paste` (§28): the refusals of `pasteText`, then the seeded outcome. */
  overlayPaste(text: string): Promise<PasteOutcome> {
    if (text.trim().length === 0 || Array.from(text).length > MAX_PASTE_TEXT_CHARS) {
      return pasteFailed("invalid");
    }
    if (this.role === "phone") return pasteFailed("unsupported");
    const phase = this.state.dictation.phase.phase;
    if (phase === "listening" || phase === "processing") return pasteFailed("busy");
    this.overlayPastes.push(text);
    return Promise.resolve(structuredClone(this.pasteOutcome));
  }

  /** `open_history` (§28): the shell would bring the main window up on the history page. */
  openHistory(): Promise<void> {
    this.historyOpens += 1;
    return Promise.resolve();
  }

  /** `presets_builtin`: the built-in presets' texts (the phone has its own presets since
   *  2026-10-01, user decision). */
  async presetsBuiltin(): Promise<BuiltinPresetText[]> {
    await Promise.resolve();
    return MOCK_BUILTIN_PRESET_TEXTS.map((text) => ({ ...text }));
  }

  private readonly handlers: {
    [K in MutationCommand]: (args: CommandArgs[K] | undefined) => void;
  } = {
    pairing_start: () => {
      if (this.refuseTooOld()) return;
      this.startPairing();
    },
    pairing_join_code: (args) => {
      if (this.refuseTooOld()) return;
      this.joinWithCode(required(args).code);
    },
    pairing_join_ticket: (args) => {
      if (this.refuseTooOld()) return;
      this.joinWithTicket(required(args).uri);
    },
    settings_set_pairing_always_on: (args) => {
      const { enabled } = required(args);
      if (this.role === "phone") {
        this.emit({ type: "error", message: ALWAYS_ON_DESKTOP_ONLY });
        return;
      }
      this.emit({
        type: "settings",
        ...this.state.settings,
        pairing_always_on: enabled,
      });
      const phase = this.state.pairing.state.state;
      if (enabled && (phase === "idle" || phase === "expired")) {
        this.startPairing();
      } else if (!enabled && (phase === "creating_session" || phase === "waiting_for_peer")) {
        // Nobody joined yet: the window closes. A pairing under way runs to its end.
        this.clearTimers();
        this.emit({ type: "pairing", ...idleSnapshot() });
      }
    },
    relay_reconnect: () => {
      // The core's link checks its socket; nothing to show unless it was gone.
      this.relayChecks += 1;
    },
    pairing_confirm: () => {
      this.confirmLocal();
    },
    pairing_reject: () => {
      this.finish({ state: "rejected" });
    },
    pairing_cancel: () => {
      this.finish({ state: "failed", reason: { kind: "cancelled" } });
    },
    pairing_reset: () => {
      this.clearTimers();
      this.emit({ type: "pairing", ...idleSnapshot() });
      // Always on, the core opens the next session on its next tick.
      if (this.alwaysOn()) {
        this.later(0, () => {
          this.startPairing();
        });
      }
    },
    device_forget: (args) => {
      const { publicKey } = required(args);
      this.emit({
        type: "devices",
        devices: this.state.devices.filter((d) => d.device.public_key !== publicKey),
      });
    },
    device_sync_set: (args) => {
      const { publicKey, on } = required(args);
      const target = this.state.devices.find((d) => d.device.public_key === publicKey);
      if (target === undefined) throw new Error("未知设备");
      if (target.device.sync === on) return;
      const syncing = this.state.devices.filter(
        (d) => d !== target && isPhone(d) && d.device.sync,
      ).length;
      if (on && isPhone(target) && syncing >= MAX_SYNC_PEERS) {
        throw new Error(`最多与 ${MAX_SYNC_PEERS} 部手机同步`);
      }
      this.emit({
        type: "devices",
        devices: this.state.devices.map((d) =>
          d === target
            ? {
                ...d,
                device: {
                  ...d.device,
                  sync: on,
                  sync_gen: d.device.sync_gen + 1,
                },
              }
            : d,
        ),
      });
    },
    device_rename: (args) => {
      const { name } = required(args);
      if (this.state.identity) this.emit({ type: "identity", ...this.state.identity, name });
    },
    send_text: (args) => {
      const { publicKey, body } = required(args);
      const target = this.state.devices.find((d) => d.device.public_key === publicKey);
      if (target?.connection.state !== "online") {
        this.emit({ type: "error", message: "设备当前不在线，消息未发送" });
        return;
      }
      this.later(MESSAGE_ECHO_MS, () => {
        this.emit({ type: "message", from: publicKey, body });
      });
    },
    settings_set_relay: (args) => {
      const { url, enabled } = required(args);
      const settings: Settings = {
        ...this.state.settings,
        relay_enabled: enabled,
      };
      if (url === null) delete settings.relay_url;
      else settings.relay_url = url;
      this.emit({ type: "settings", ...settings });
      this.applyRelaySettings(settings);
    },
    settings_set_theme: (args) => {
      const { theme, followSystem } = required(args);
      this.emit({
        type: "settings",
        ...this.state.settings,
        theme,
        follow_system_theme: followSystem,
      });
    },
    settings_set_hotkey: (args) => {
      const { hotkey } = required(args);
      // Mirrors voltip_core::Hotkey: at least one modifier and exactly one key; never the edit chord.
      const problem = mockChordProblem(hotkey);
      if (problem !== undefined) {
        this.emit({ type: "error", message: `hotkey: ${hotkey} ${problem}` });
        return;
      }
      const edit = this.state.settings.edit_hotkey;
      if (edit !== null && mockSameChord(hotkey, edit)) {
        this.emit({
          type: "error",
          message: `hotkey: ${hotkey} 已用作「编辑选中文本」的快捷键`,
        });
        return;
      }
      const copyLast = this.state.settings.copy_last_hotkey;
      if (copyLast != null && mockSameChord(hotkey, copyLast)) {
        this.emit({
          type: "error",
          message: `hotkey: ${hotkey} 已用作「复制上一条结果」的快捷键`,
        });
        return;
      }
      this.emit({ type: "settings", ...this.state.settings, hotkey });
      this.emit({ type: "hotkey", ...this.hotkeyStatus(false) });
    },
    settings_set_solo_key: (args) => {
      // Mirrors `SetSoloKey` (docs/dictation.md §13.1): any key is stored; the shell then says
      // whether this session can watch it.
      const { key } = required(args);
      this.emit({ type: "settings", ...this.state.settings, solo_key: key });
      this.emit({ type: "hotkey", ...this.hotkeyStatus(false) });
    },
    settings_set_microphone: (args) => {
      // Mirrors `SetMicrophone`: a device id of 1–1024 bytes or `null` (the default input); the
      // core refuses anything else with an `error` event and keeps the choice.
      const { device } = required(args);
      if (
        device !== null &&
        (device.trim().length === 0 || new TextEncoder().encode(device).length > 1024)
      ) {
        this.emit({
          type: "error",
          message: "microphone: 麦克风标识须为 1–1024 字节，留空则使用系统默认输入",
        });
        return;
      }
      this.emit({
        type: "settings",
        ...this.state.settings,
        microphone: device,
      });
    },
    settings_set_recording: (args) => {
      // Mirrors `SetRecording` (docs/dictation.md §22): a length outside the choices or an output
      // device id that is not 1–1024 bytes is refused with an `error` event, the setting kept.
      const { recording } = required(args);
      if (!(MAX_MINUTES_CHOICES as readonly number[]).includes(recording.max_minutes)) {
        this.emit({
          type: "error",
          message: `recording.max_minutes: 最长录音时长须为 ${MAX_MINUTES_CHOICES.join(" / ")} 分钟之一`,
        });
        return;
      }
      const device = recording.output_device;
      if (
        device !== null &&
        (device.trim().length === 0 || new TextEncoder().encode(device).length > 1024)
      ) {
        this.emit({
          type: "error",
          message: "recording.output_device: 输出设备标识须为 1–1024 字节，留空则使用系统默认输出",
        });
        return;
      }
      this.emit({
        type: "settings",
        ...this.state.settings,
        recording: { ...recording },
      });
    },
    settings_set_edit_hotkey: (args) => {
      // Mirrors `SetEditHotkey` (docs/dictation.md §19): the same validation, never the dictation
      // chord, `null` switches the key off.
      const { hotkey } = required(args);
      if (hotkey !== null) {
        const problem = mockChordProblem(hotkey);
        if (problem !== undefined) {
          this.emit({ type: "error", message: `hotkey: ${hotkey} ${problem}` });
          return;
        }
        if (mockSameChord(hotkey, this.state.settings.hotkey)) {
          this.emit({
            type: "error",
            message: `edit_hotkey: ${hotkey} 已用作听写快捷键`,
          });
          return;
        }
        const copyLast = this.state.settings.copy_last_hotkey;
        if (copyLast != null && mockSameChord(hotkey, copyLast)) {
          this.emit({
            type: "error",
            message: `edit_hotkey: ${hotkey} 已用作「复制上一条结果」的快捷键`,
          });
          return;
        }
      }
      this.emit({
        type: "settings",
        ...this.state.settings,
        edit_hotkey: hotkey,
      });
      this.emit({ type: "hotkey", ...this.hotkeyStatus(false) });
    },
    settings_set_copy_last_hotkey: (args) => {
      // Mirrors `SetCopyLastHotkey` (docs/dictation.md §28): validated like the dictation chord,
      // never the dictation's or the voice edit's; `null` assigns none.
      const { hotkey } = required(args);
      if (hotkey !== null) {
        const problem = mockChordProblem(hotkey);
        if (problem !== undefined) {
          this.emit({ type: "error", message: `hotkey: ${hotkey} ${problem}` });
          return;
        }
        if (mockSameChord(hotkey, this.state.settings.hotkey)) {
          this.emit({ type: "error", message: `copy_last_hotkey: ${hotkey} 已用作听写快捷键` });
          return;
        }
        const edit = this.state.settings.edit_hotkey;
        if (edit !== null && mockSameChord(hotkey, edit)) {
          this.emit({
            type: "error",
            message: `copy_last_hotkey: ${hotkey} 已用作「编辑选中文本」的快捷键`,
          });
          return;
        }
      }
      const { copy_last_hotkey: _old, ...rest } = this.state.settings;
      this.emit({
        type: "settings",
        ...rest,
        ...(hotkey === null ? {} : { copy_last_hotkey: hotkey }),
      });
      this.emit({ type: "hotkey", ...this.hotkeyStatus(false) });
    },
    hotkey_capture: (args) => {
      // Mirrors the desktop shell: recording suspends the registration, closing restores it.
      const { active } = required(args);
      this.emit({ type: "hotkey", ...this.hotkeyStatus(active) });
    },
    devices_refresh: () => {
      this.emit({ type: "devices", devices: this.state.devices });
    },
    connectivity_check: () => {
      if (this.state.connectivity.running) {
        this.emit({ type: "error", message: "connectivity: 自检正在进行" });
        return;
      }
      this.emit({
        type: "connectivity",
        ...this.state.connectivity,
        running: true,
      });
      this.later(MOCK_CONNECTIVITY_MS, () => {
        this.emit({
          type: "connectivity",
          running: false,
          report: this.connectivityReport(),
        });
      });
    },
    phone_text_send: (args) => {
      // Mirrors `PhoneTextSend` (docs/dictation.md §20.6): the phone lists it as sending, the
      // desktop inserts it after `MOCK_TEXT_MS`.
      if (this.role !== "phone") {
        this.emit({ type: "error", message: PHONE_TEXT_UNAVAILABLE });
        return;
      }
      const { publicKey, body, source } = required(args);
      if (body.trim().length === 0) {
        this.emit({ type: "error", message: "phone text: 没有要发送的文字" });
        return;
      }
      // Characters as Rust counts them (code points), not UTF-16 units.
      if (Array.from(body).length > MAX_PHONE_TEXT_CHARS) {
        this.emit({
          type: "error",
          message: `phone text: 文字太长（最多 ${MAX_PHONE_TEXT_CHARS} 字）`,
        });
        return;
      }
      const target = this.state.devices.find((d) => d.device.public_key === publicKey);
      if (target?.connection.state !== "online") {
        this.emit({ type: "error", message: "设备不在线" });
        return;
      }
      // Like the core's counter: it outlives 清空, so the desktop never sees an id twice.
      this.lastTextId = Math.max(this.lastTextId, ...this.state.sent_texts.map((t) => t.id)) + 1;
      const id = this.lastTextId;
      const text: SentText = {
        id,
        device: publicKey,
        device_name: target.device.name,
        body,
        source,
        sent_at: this.now(),
        state: { state: "sending" },
      };
      this.emit({
        type: "sent_texts",
        texts: [text, ...this.state.sent_texts].slice(0, 50),
      });
      this.later(MOCK_TEXT_MS, () => {
        this.emit({
          type: "sent_texts",
          texts: this.state.sent_texts.map((t) =>
            t.id === id && t.device === publicKey
              ? { ...t, state: { state: "delivered", pasted: true } }
              : t,
          ),
        });
      });
    },
    sent_texts_clear: () => {
      this.emit({ type: "sent_texts", texts: [] });
    },
    phone_share_text: (args) => {
      // The shell's own command (docs/dictation.md §20.7): the share sheet opens, nothing in the
      // core changes. The phone refuses what a paste would refuse.
      const { text } = required(args);
      if (text.trim().length === 0 || Array.from(text).length > MAX_PASTE_TEXT_CHARS)
        throw new Error(`share: 文字为空或超过 ${MAX_PASTE_TEXT_CHARS} 字`);
      this.shared.push(text);
    },
    phone_take_start: (args) => {
      const { publicKey } = required(args);
      // Mirrors `PhoneTakeStart`: one take at a time, to an online desktop; the desktop answers.
      const running = this.state.phone_take;
      if (running !== undefined && !phoneTakeFinal(running.state)) {
        this.emit({ type: "error", message: "phone take: 已有一次录音在进行" });
        return;
      }
      const target = this.state.devices.find((d) => d.device.public_key === publicKey);
      if (target?.connection.state !== "online") {
        this.emit({ type: "error", message: "设备不在线" });
        return;
      }
      const take = (running?.take ?? 0) + 1;
      this.emit({
        type: "phone_take",
        take: {
          device: publicKey,
          take,
          started_at: this.now(),
          state: { state: "starting" },
        },
      });
      this.later(MOCK_MIC_READY_MS, () => {
        if (
          this.state.phone_take?.take === take &&
          this.state.phone_take.state.state === "starting"
        )
          // The desktop's first status says it decodes Opus (docs/dictation.md §20.1).
          this.emitPhoneTake({ state: "listening" }, true);
      });
    },
    phone_take_stop: () => {
      const t = this.state.phone_take;
      if (t === undefined || phoneTakeFinal(t.state)) {
        this.emit({ type: "error", message: "phone take: 没有进行中的录音" });
        return;
      }
      this.emitPhoneTake({ state: "processing" });
      const spokenMs = Math.max(0, this.now() - t.started_at);
      this.later(MOCK_ASR_MS + MOCK_REFINE_MS, () => {
        if (
          this.state.phone_take?.take === t.take &&
          this.state.phone_take.state.state === "processing"
        ) {
          this.emitPhoneTake({
            state: "done",
            text: MOCK_DICTATION_TEXT,
            pasted: true,
          });
          // The phone keeps its own record of a delivered take, marked with the computer it went
          // to (docs/dictation.md §20.7, user decision 2026-10-03), as the core does.
          const computer = this.state.devices.find((d) => d.device.public_key === t.device);
          this.recordHistory({
            id: this.uuid(),
            at_ms: this.now(),
            raw_text: MOCK_DICTATION_TEXT,
            text: MOCK_DICTATION_TEXT,
            refined: false,
            asr_model: "",
            duration_ms: spokenMs,
            asr_ms: 0,
            outcome: { kind: "inserted", via: "paste" },
            starred: false,
            mode: "whole_take",
            kind: "dictation",
            origin: { device: computer?.device.name ?? t.device, kind: "sent" },
          });
        }
      });
    },
    phone_take_cancel: () => {
      const t = this.state.phone_take;
      if (t === undefined || phoneTakeFinal(t.state)) {
        this.emit({ type: "error", message: "phone take: 没有进行中的录音" });
        return;
      }
      this.emitPhoneTake({ state: "cancelled" });
    },
    dictation_start: () => {
      this.startDictation("dictation");
    },
    dictation_stop: () => {
      this.stopDictation();
    },
    dictation_cancel: () => {
      this.cancelDictation();
    },
    dictation_dismiss: (args) => {
      // Mirrors `DictationDismiss` (docs/dictation.md §28): that take's finished card goes at
      // once; a take under way, or the card of a later take, is not touched.
      const { session } = required(args);
      if (session !== this.state.dictation.session) return;
      const phase = this.state.dictation.phase.phase;
      if (phase !== "done" && phase !== "failed" && phase !== "cancelled") return;
      this.clearDictationTimers();
      this.pinned = undefined;
      this.emitPhase({ phase: "idle" });
    },
    settings_set_engines: (args) => {
      const { engines } = required(args);
      // Mirrors `SetEngines`: an unknown local model id is refused, nothing changes.
      if (engines.local_model != null && !this.catalogueRow(engines.local_model)) {
        this.emit({
          type: "error",
          message: `engines: 未知的本地模型 ${engines.local_model}`,
        });
        return;
      }
      // Mirrors `SetEngines` (docs/dictation.md §3.7): only the custom provider chooses its
      // clean-up interface.
      for (const [id, choice] of Object.entries(engines.providers ?? {})) {
        if (
          id !== "custom" &&
          (choice?.llm_api !== undefined || choice?.llm_reasoning !== undefined)
        ) {
          this.emit({ type: "error", message: `${id}.llm_api: 只有自定义服务商可以选择接口类型` });
          return;
        }
      }
      const settings: Settings = {
        ...this.state.settings,
        engines: { ...engines },
      };
      this.emit({ type: "settings", ...settings });
      this.emit({ type: "engines", ...this.resolveEngines(settings.engines) });
      this.emit({ type: "models", models: this.modelsFor(settings.engines) });
      // §28: 识别结果卡片常驻 switched off, a card that stays leaves after its usual time from now.
      if (!settings.engines.result_cards_stay && this.pinned !== undefined) {
        const { ms, session } = this.pinned;
        this.pinned = undefined;
        this.dwell(ms, session);
      }
    },
    model_download: (args) => {
      this.downloadModel(required(args).id);
    },
    model_cancel: (args) => {
      this.cancelModel(required(args).id);
    },
    model_remove: (args) => {
      this.removeModel(required(args).id);
    },
    model_import: (args) => {
      this.importModel(required(args).id);
    },
    models_location_reset: () => {
      this.relocateModels(this.state.models_location.default_dir);
    },
    models_location_cancel: () => {
      this.cancelModelsMove();
    },
    provider_key_set: (args) => {
      const { provider, kind, value } = required(args);
      const entry = keyEntry(provider, kind);
      if (entry === undefined) {
        this.emit({
          type: "error",
          message: `${provider}: 此服务商不需要密钥`,
        });
        return;
      }
      if (value !== null && value.trim().length > 0) this.userKeys.add(entry);
      else this.userKeys.delete(entry);
      // Another key may be another account, with its own models.
      this.listedModels.delete(`${provider}:asr`);
      this.listedModels.delete(`${provider}:llm`);
      this.emit({
        type: "engines",
        ...this.resolveEngines(this.state.settings.engines),
      });
    },
    provider_probe: (args) => {
      const { provider, kind, baseUrl, key } = required(args);
      this.probe(provider, kind, baseUrl ?? undefined, key ?? undefined);
    },
    provider_models_refresh: (args) => {
      const { provider, kind } = required(args);
      this.refreshModels(provider, kind);
    },
    engines_quota_reset: (args) => {
      const { kind } = required(args);
      this.quotaOut.delete(kind);
      this.emit({
        type: "engines",
        ...this.resolveEngines(this.state.settings.engines),
      });
    },
    history_delete: (args) => {
      const { id } = required(args);
      this.setHistory(this.history.filter((e) => e.id !== id));
      // docs/dictation.md §26: the entry's recording goes with it.
      this.followHistory();
    },
    history_clear: () => {
      this.setHistory([]);
      this.setRecordings([]);
    },
    settings_set_audio_cache: (args) => {
      const { enabled, keep, maxMb } = required(args);
      // Mirrors `SetAudioCache`: out of range is refused; off deletes the recordings, tighter
      // limits drop the oldest at once.
      if (!Number.isInteger(keep) || keep < AUDIO_CACHE_MIN_KEEP || keep > AUDIO_CACHE_MAX_KEEP) {
        this.emit({
          type: "error",
          message: `audio_cache.keep: ${AUDIO_CACHE_MIN_KEEP}–${AUDIO_CACHE_MAX_KEEP}`,
        });
        return;
      }
      if (!Number.isInteger(maxMb) || maxMb < AUDIO_CACHE_MIN_MB || maxMb > AUDIO_CACHE_MAX_MB) {
        this.emit({
          type: "error",
          message: `audio_cache.max_mb: ${AUDIO_CACHE_MIN_MB}–${AUDIO_CACHE_MAX_MB}`,
        });
        return;
      }
      this.emit({
        type: "settings",
        ...this.state.settings,
        audio_cache: { enabled, keep, max_mb: maxMb },
      });
      const recordings = this.state.audio_cache.recordings;
      this.setRecordings(enabled ? this.withinLimits(recordings) : []);
    },
    audio_cache_clear: () => {
      this.setRecordings([]);
    },
    audio_cache_delete: (args) => {
      const { id } = required(args);
      this.setRecordings(this.state.audio_cache.recordings.filter((r) => r.id !== id));
    },
    settings_set_backup: (args) => {
      const { weekly } = required(args);
      this.emit({
        type: "settings",
        ...this.state.settings,
        backup: { ...this.state.settings.backup, weekly },
      });
    },
    backup_folder_reset: () => {
      this.setBackupFolder(null);
    },
    backup_notice_dismiss: () => {
      if (this.state.auto_backup.notice !== null) this.emitAutoBackup({ notice: null });
    },
    audio_retake: (args) => {
      const { id, asr, refine } = required(args);
      this.retake(id, {
        ...(asr === undefined ? {} : { asr }),
        ...(refine === undefined ? {} : { refine }),
      });
    },
    import_add: (args) => {
      this.importAdd(required(args));
    },
    import_cancel: (args) => {
      this.importDrop(uuidArg(required(args).id));
    },
    import_remove: (args) => {
      this.importDrop(uuidArg(required(args).id));
    },
    import_retry: (args) => {
      this.importRetry(uuidArg(required(args).id));
    },
    history_star: (args) => {
      const { id, starred } = required(args);
      this.setHistory(this.history.map((e) => (e.id === id ? { ...e, starred } : e)));
    },
    history_process: (args) => {
      const { requestId, id, preset } = required(args);
      this.processEntry(requestId, id, preset);
    },
    history_process_cancel: (args) => {
      const { requestId } = required(args);
      const running = this.processing.get(requestId);
      if (running === undefined) return;
      clearTimeout(running.timer);
      this.processing.delete(requestId);
      this.emit({
        type: "history_process",
        request_id: requestId,
        id: running.id,
        state: { state: "cancelled" },
      });
    },
    settings_set_locale: (args) => {
      const { locale } = required(args);
      this.emit({ type: "settings", ...this.state.settings, locale });
    },
    settings_set_auto_update: (args) => {
      const { enabled } = required(args);
      this.emit({
        type: "settings",
        ...this.state.settings,
        auto_update: enabled,
      });
    },
    settings_set_history: (args) => {
      const { enabled, keep } = required(args);
      // Mirrors `SetHistory`: out of range is refused, a smaller keep trims at once.
      if (!Number.isInteger(keep) || keep < HISTORY_MIN_KEEP || keep > HISTORY_LIMIT) {
        this.emit({
          type: "error",
          message: `history.keep: ${HISTORY_MIN_KEEP}–${HISTORY_LIMIT}`,
        });
        return;
      }
      this.emit({
        type: "settings",
        ...this.state.settings,
        history: { enabled, keep },
      });
      if (this.history.length > keep) {
        this.setHistory(this.history.slice(0, keep));
        this.followHistory();
      }
    },
    settings_set_overlay: (args) => {
      const { placement } = required(args);
      this.emit({
        type: "settings",
        ...this.state.settings,
        overlay: placement,
      });
    },
    hotkey_edge: (args) => {
      this.hotkeyEdge(required(args));
    },
    settings_set_activation: (args) => {
      const { activation, holdThresholdMs, extraRecordingMs } = required(args);
      // Mirrors `SetActivation`: both timings are capped, the settings stay as they were.
      if (holdThresholdMs > MAX_ACTIVATION_MS || extraRecordingMs > MAX_ACTIVATION_MS) {
        this.emit({
          type: "error",
          message: `activation: 时长不能超过 ${MAX_ACTIVATION_MS} ms（hold_threshold_ms ${holdThresholdMs}，extra_recording_ms ${extraRecordingMs}）`,
        });
        return;
      }
      this.emit({
        type: "settings",
        ...this.state.settings,
        activation,
        hold_threshold_ms: holdThresholdMs,
        extra_recording_ms: extraRecordingMs,
      });
    },
    update_check: () => {
      this.checkForUpdate();
    },
    update_install: () => {
      this.installUpdate();
    },
    // docs/dictation.md §16.4: a draft that is wrong on its own throws (the command rejects, as
    // `into_core` does); a clash with the rest of the list is an `error` event, list unchanged.
    dictionary_add: (args) => {
      const { entry, historyId } = required(args);
      const draft = validateDictionaryDraft(entry);
      const source: EntrySource =
        historyId == null
          ? { kind: "manual" }
          : { kind: "history", history_id: uuidArg(historyId) };
      const now = this.now();
      this.commitDictionary([
        ...this.state.dictionary,
        {
          id: this.uuid(),
          ...draft,
          source,
          created_at_ms: now,
          updated_at_ms: now,
        },
      ]);
    },
    dictionary_update: (args) => {
      const { id, entry } = required(args);
      uuidArg(id);
      const draft = validateDictionaryDraft(entry);
      if (!this.state.dictionary.some((e) => e.id === id)) {
        this.emit({
          type: "error",
          message: `dictionary: 没有 id 为 ${id} 的词条`,
        });
        return;
      }
      this.commitDictionary(
        this.state.dictionary.map((e) =>
          e.id === id ? { ...e, ...draft, updated_at_ms: this.now() } : e,
        ),
      );
    },
    dictionary_remove: (args) => {
      const { id } = required(args);
      uuidArg(id);
      if (!this.state.dictionary.some((e) => e.id === id)) {
        this.emit({
          type: "error",
          message: `dictionary: 没有 id 为 ${id} 的词条`,
        });
        return;
      }
      this.commitDictionary(this.state.dictionary.filter((e) => e.id !== id));
    },
    vocabulary_suggestion_ignore: (args) => {
      const { heard, polished } = required(args);
      const [h, p] = validateSuggestionPair(heard, polished);
      const others = this.state.vocabulary_ignored.filter((i) => i.heard !== h || i.polished !== p);
      const pairs = [...others, { heard: h, polished: p, at_ms: this.now() }].slice(-MAX_IGNORED);
      this.emit({ type: "vocabulary_ignored", pairs });
    },
    vocabulary_suggestion_unignore: (args) => {
      const { heard, polished } = required(args);
      const [h, p] = validateSuggestionPair(heard, polished);
      const pairs = this.state.vocabulary_ignored.filter((i) => i.heard !== h || i.polished !== p);
      this.emit({ type: "vocabulary_ignored", pairs });
    },
    dictionary_reorder: (args) => {
      const { ids } = required(args);
      const next = permute(this.state.dictionary, ids.map(uuidArg));
      if (next === undefined) {
        this.emit({
          type: "error",
          message: "dictionary: 新的顺序必须恰好包含现有的全部词条",
        });
        return;
      }
      this.commitDictionary(next);
    },
    rules_add: (args) => {
      const { rule } = required(args);
      const draft = validateRuleDraft(rule);
      const now = this.now();
      this.commitRules([...this.state.rules, this.ruleFrom(draft, this.uuid(), now)]);
    },
    rules_update: (args) => {
      const { id, rule } = required(args);
      uuidArg(id);
      const draft = validateRuleDraft(rule);
      const current = this.state.rules.find((r) => r.id === id);
      if (current === undefined) {
        this.emit({ type: "error", message: `rules: 没有 id 为 ${id} 的规则` });
        return;
      }
      // As the core: the source stays, and turning a rule that waits for approval on approves it.
      this.commitRules(
        this.state.rules.map((r) =>
          r.id === id
            ? this.ruleFrom(draft, id, r.created_at_ms, r.source, r.pending && !draft.enabled)
            : r,
        ),
      );
    },
    rules_remove: (args) => {
      const { id } = required(args);
      uuidArg(id);
      if (!this.state.rules.some((r) => r.id === id)) {
        this.emit({ type: "error", message: `rules: 没有 id 为 ${id} 的规则` });
        return;
      }
      this.commitRules(this.state.rules.filter((r) => r.id !== id));
    },
    rules_reorder: (args) => {
      const { ids } = required(args);
      const next = permute(this.state.rules, ids.map(uuidArg));
      if (next === undefined) {
        this.emit({
          type: "error",
          message: "rules: 新的顺序必须恰好包含现有的全部规则",
        });
        return;
      }
      this.commitRules(next);
    },
    rules_import: (args) => {
      const { toml, mode } = required(args);
      this.importRules(parseRulesToml(toml), mode);
    },
    // docs/dictation.md §18.6: the same split as the vocabulary — a draft wrong on its own throws,
    // a clash with the list (name, cap, unknown id) is an `error` event with the list unchanged.
    scenes_add: (args) => {
      const { scene } = required(args);
      // A phone's scenes need not name an application (`scenes_need_apps`, user decision
      // 2026-10-01); a desktop refuses one that names none, like the bridge.
      const draft = validateSceneDraft(scene, this.role !== "phone");
      const now = this.now();
      // Before the first built-in scene: the user's scenes match first unless moved.
      const scenes = [...this.state.scenes];
      const at = scenes.findIndex((s) => s.builtin !== undefined);
      scenes.splice(at < 0 ? scenes.length : at, 0, this.sceneFrom(draft, this.uuid(), now));
      this.commitScenes(scenes);
    },
    scenes_update: (args) => {
      const { id, scene } = required(args);
      uuidArg(id);
      // The bridge lets an update list no application; the core knows which scenes may.
      const draft = validateSceneDraft(scene, false);
      const current = this.state.scenes.find((s) => s.id === id);
      if (current === undefined) {
        this.emit({
          type: "error",
          message: `scenes: 没有 id 为 ${id} 的场景`,
        });
        return;
      }
      if (current.builtin !== undefined && draft.name !== current.name) {
        this.emit({ type: "error", message: "scenes: 内置场景不能改名" });
        return;
      }
      if (this.role !== "phone" && current.builtin === undefined && draft.match.apps.length === 0) {
        this.emit({
          type: "error",
          message: `scenes: 场景「${draft.name}」至少要有一个应用`,
        });
        return;
      }
      this.commitScenes(
        this.state.scenes.map((s) =>
          s.id === id ? this.sceneFrom(draft, id, s.created_at_ms, s.builtin) : s,
        ),
      );
    },
    scenes_remove: (args) => {
      const { id } = required(args);
      uuidArg(id);
      const current = this.state.scenes.find((s) => s.id === id);
      if (current === undefined) {
        this.emit({
          type: "error",
          message: `scenes: 没有 id 为 ${id} 的场景`,
        });
        return;
      }
      if (current.builtin !== undefined) {
        this.emit({
          type: "error",
          message: "scenes: 内置场景不能删除，可以关闭",
        });
        return;
      }
      this.commitScenes(this.state.scenes.filter((s) => s.id !== id));
    },
    scenes_restore: (args) => {
      const { id } = required(args);
      uuidArg(id);
      const current = this.state.scenes.find((s) => s.id === id);
      if (current === undefined) {
        this.emit({
          type: "error",
          message: `scenes: 没有 id 为 ${id} 的场景`,
        });
        return;
      }
      const platform = this.builtinPlatform();
      const row = MOCK_BUILTIN_SCENES.find((r) => r.id === current.builtin);
      if (row === undefined || platform === undefined) {
        this.emit({
          type: "error",
          message: "scenes: 只有内置场景可以恢复默认",
        });
        return;
      }
      const template = structuredClone(row.templates[platform]);
      this.commitScenes(
        this.state.scenes.map((s) =>
          s.id === id
            ? {
                ...s,
                match: template.match,
                overrides: template.overrides,
                updated_at_ms: this.now(),
              }
            : s,
        ),
      );
    },
    scenes_reorder: (args) => {
      const { ids } = required(args);
      const next = permute(this.state.scenes, ids.map(uuidArg));
      if (next === undefined) {
        this.emit({
          type: "error",
          message: "scenes: 新的顺序必须恰好包含现有的全部场景",
        });
        return;
      }
      this.commitScenes(next);
    },
    // docs/dictation.md §21: the bridge refuses a draft wrong on its own (the invoke throws), the
    // core a clash with the list (name, cap, unknown id) with an `error` event.
    presets_add: (args) => {
      const { preset } = required(args);
      const draft = validatePresetDraft(preset);
      const now = this.now();
      this.commitPresets([...this.state.presets, this.presetFrom(draft, this.uuid(), now)]);
    },
    presets_update: (args) => {
      const { id, preset } = required(args);
      uuidArg(id);
      const draft = validatePresetDraft(preset);
      const current = this.state.presets.find((p) => p.id === id);
      if (current === undefined) {
        this.emit({
          type: "error",
          message: `presets: 没有 id 为 ${id} 的预设`,
        });
        return;
      }
      this.commitPresets(
        this.state.presets.map((p) =>
          p.id === id ? this.presetFrom(draft, id, p.created_at_ms) : p,
        ),
      );
    },
    presets_remove: (args) => {
      const { id } = required(args);
      uuidArg(id);
      if (!this.state.presets.some((p) => p.id === id)) {
        this.emit({
          type: "error",
          message: `presets: 没有 id 为 ${id} 的预设`,
        });
        return;
      }
      this.commitPresets(this.state.presets.filter((p) => p.id !== id));
    },
    presets_try: (args) => {
      const { id, preset, prompt, text } = required(args);
      this.tryPreset(id, presetTrial(preset, prompt), presetTryText(text));
    },
    settings_set_context_sharing: (args) => {
      const { appName, windowTitle } = required(args);
      this.emit({
        type: "settings",
        ...this.state.settings,
        context_sharing: { app_name: appName, window_title: windowTitle },
      });
    },
    settings_set_phone_talk: (args) => {
      const { mode } = required(args);
      if (mode !== "hold" && mode !== "tap")
        throw new Error(`phone talk: 未知的说话方式 ${String(mode)}`);
      this.emit({ type: "settings", ...this.state.settings, phone_talk: mode });
    },
    settings_set_pinned_scene: (args) => {
      const { id } = required(args);
      if (id !== null) uuidArg(id);
      const { pinned_scene: _old, ...rest } = this.state.settings;
      this.emit({
        type: "settings",
        ...rest,
        ...(id === null ? {} : { pinned_scene: id }),
      });
    },
    settings_set_serve: (args) => {
      const { enabled, port, preset, scene } = required(args);
      this.requireServe();
      if (!Number.isInteger(port) || port < MIN_SERVE_PORT || port > 65_535)
        throw new Error(`serve: 端口须在 ${MIN_SERVE_PORT}–65535 之间`);
      if (scene !== null && !this.state.scenes.some((s) => s.id === scene))
        throw new Error(`serve: 没有 id 为 ${scene} 的场景`);
      if (
        preset !== null &&
        !isBuiltinPreset(preset) &&
        !this.state.presets.some((p) => p.id === preset)
      )
        throw new Error(`serve: 没有 id 为 ${preset} 的预设`);
      const serve: ServeSettings = {
        enabled,
        port,
        ...(preset === null ? {} : { preset }),
        ...(scene === null ? {} : { scene }),
      };
      const before = this.state.settings.serve;
      this.emit({ type: "settings", ...this.state.settings, serve });
      // As the core: a listener that (re)starts is `starting` first.
      if (serve.enabled && (!before.enabled || before.port !== serve.port))
        this.emit({ type: "serve", available: true, phase: "starting" });
      this.emit({ type: "serve", ...this.serveStatusFor(serve) });
    },
    serve_copy_token: () => {
      this.requireServe();
      this.serveTokenCopies += 1;
    },
    serve_rotate_token: () => {
      this.requireServe();
      this.serveTokenRotations += 1;
    },
    // docs/dictation.md §25: refusals of the settings are `error` events, as the core reports them.
    settings_set_mcp: (args) => {
      const { enabled, port, read, historyRange, write } = required(args);
      this.requireMcp();
      if (!Number.isInteger(port) || port < MIN_SERVE_PORT || port > 65_535) {
        this.emit({ type: "error", message: `mcp: 端口须在 ${MIN_SERVE_PORT}–65535 之间` });
        return;
      }
      if (port === this.state.settings.serve.port) {
        this.emit({ type: "error", message: `mcp: 端口 ${port} 已用于本机服务，请换一个端口` });
        return;
      }
      const mcp: McpSettings = { enabled, port, read, history_range: historyRange, write };
      const before = this.state.settings.mcp;
      this.emit({ type: "settings", ...this.state.settings, mcp });
      // As the core: a listener that (re)starts is `starting` first; the levels apply at once.
      if (mcp.enabled && (!before.enabled || before.port !== mcp.port)) {
        const { address: _a, error: _e, ...rest } = this.state.mcp;
        this.emit({ type: "mcp", ...rest, phase: "starting" });
      }
      if (mcp.enabled !== before.enabled || mcp.port !== before.port)
        this.emit({ type: "mcp", ...this.mcpStatusFor(mcp, this.state.mcp) });
    },
    mcp_copy_token: () => {
      this.requireMcp();
      this.mcpTokenCopies += 1;
    },
    mcp_rotate_token: () => {
      this.requireMcp();
      this.mcpTokenRotations += 1;
    },
    mcp_copy_setup: (args) => {
      const { client } = required(args);
      this.requireMcp();
      this.mcpSetupCopies.push(client);
    },
    mcp_undo: (args) => {
      const { id } = required(args);
      uuidArg(id);
      this.requireMcp();
      this.undoMcp(id);
    },
    refine_notice_close: () => {
      if (this.state.refine_notice !== undefined)
        this.emit({ type: "refine_notice", notice: null });
    },
    // docs/dictation.md §27: the preview's service answers at once with what it has.
    usage_refresh: () => {
      this.emit({ type: "usage", ...this.state.usage });
    },
    usage_notice_dismiss: (args) => {
      const { notice } = required(args);
      const dismissed = this.state.usage.dismissed.includes(notice)
        ? this.state.usage.dismissed
        : [...this.state.usage.dismissed, notice];
      this.emit({ type: "usage", ...this.state.usage, dismissed });
    },
    settings_set_proxy: (args) => {
      const { mode, server, username, noProxy, password } = required(args);
      const checked = checkProxySettings({ mode, server, username, no_proxy: noProxy });
      if ("problem" in checked) throw new Error(proxyProblemCoreText(checked.problem));
      const problem =
        password === null ? undefined : proxyPasswordProblem(password, checked.proxy.username);
      if (problem !== undefined) throw new Error(proxyProblemCoreText(problem));
      if (password !== null) this.proxyPassword = password === "" ? undefined : password;
      this.emit({ type: "settings", ...this.state.settings, proxy: checked.proxy });
      this.emit({
        type: "network",
        ...this.state.network,
        proxy_password: this.proxyPassword !== undefined,
      });
    },
    network_refresh: () => {
      this.emit({ type: "network", ...this.state.network });
    },
  };

  /** The manual proxy's password as the preview's keychain holds it (never in the state). */
  private proxyPassword: string | undefined;

  /** Tests: what the preview's keychain holds for the proxy. */
  savedProxyPassword(): string | undefined {
    return this.proxyPassword;
  }

  /** Tests and the preview: the system's proxy changes; `network_refresh` reports it. */
  setSystemProxy(proxy: string | undefined) {
    const { system_proxy: _old, ...rest } = this.state.network;
    this.state.network = { ...rest, ...(proxy === undefined ? {} : { system_proxy: proxy }) };
  }

  /** `simulateRefineFailure`: how the next takes' clean-up fails, if it does. */
  private refineFailure: RefineFailure | undefined;
  private windowSwitched = false;

  /** How often 复制令牌 was pressed (tests; the token itself never reaches the interface). */
  serveTokenCopies = 0;
  /** How often 重新生成 was pressed (tests). */
  serveTokenRotations = 0;

  /** How often the MCP service's 复制令牌 was pressed (tests; the token never reaches the page). */
  mcpTokenCopies = 0;
  /** How often the MCP service's 重新生成 was pressed (tests). */
  mcpTokenRotations = 0;
  /** Which setup commands 复制命令 put on the clipboard, in order (tests). */
  mcpSetupCopies: McpSetupClient[] = [];
  /** What undoing each AI tool's write puts back (the core keeps it with the activity). */
  private mcpUndo = new Map<
    string,
    | { list: "entry"; before?: DictionaryEntry; after: DictionaryEntry }
    | { list: "rule"; before?: ReplacementRule; after: ReplacementRule }
  >();

  /** The MCP service runs on a computer only (docs/dictation.md §25). */
  private requireMcp() {
    if (this.role === "phone") throw new Error(MCP_UNAVAILABLE);
  }

  /** What the preview's MCP service does with `mcp`: a port in `MOCK_TAKEN_PORTS` cannot be bound;
   *  the clients and the activity stay. */
  private mcpStatusFor(mcp: McpSettings, previous?: McpStatus): McpStatus {
    if (this.role === "phone") return { available: false, phase: "off", clients: [], activity: [] };
    const keep = { clients: previous?.clients ?? [], activity: previous?.activity ?? [] };
    if (!mcp.enabled) return { available: true, phase: "off", ...keep };
    if (MOCK_TAKEN_PORTS.includes(mcp.port))
      return {
        available: true,
        phase: "failed",
        error: `无法监听 127.0.0.1:${mcp.port}：Address already in use`,
        ...keep,
      };
    return { available: true, phase: "running", address: mcpAddress(mcp.port), ...keep };
  }

  /** Note an AI tool's call in the activity, newest first, 50 at most (docs/dictation.md §25). */
  private noteMcp(
    client: string,
    action: McpAction,
    extra: Partial<Pick<McpActivity, "subject" | "count" | "range" | "pending">> = {},
  ): McpActivity {
    const activity: McpActivity = {
      id: this.uuid(),
      at_ms: this.now(),
      client,
      action,
      pending: false,
      undoable: isMcpWrite({ action }),
      undone: false,
      ...extra,
    };
    const clients = [
      { name: client, last_seen_ms: activity.at_ms },
      ...this.state.mcp.clients.filter((c) => c.name !== client),
    ];
    this.emit({
      type: "mcp",
      ...this.state.mcp,
      clients,
      activity: [activity, ...this.state.mcp.activity].slice(0, 50),
    });
    return activity;
  }

  /** The preview's AI tool adds an entry (docs/dictation.md §25): its source names the tool, the
   *  activity can undo it. Returns the entry's id; throws what the dictionary refuses. */
  simulateMcpEntry(client: string, entry: DictionaryDraft): string {
    const draft = validateDictionaryDraft(entry);
    const now = this.now();
    const added: DictionaryEntry = {
      id: this.uuid(),
      ...draft,
      source: { kind: "mcp", client },
      created_at_ms: now,
      updated_at_ms: now,
    };
    const entries = [...this.state.dictionary, added];
    checkDictionary(entries);
    this.emit({ type: "dictionary", entries });
    const activity = this.noteMcp(client, "add_entry", { subject: added.term });
    this.mcpUndo.set(activity.id, { list: "entry", after: added });
    return added.id;
  }

  /** The preview's AI tool creates a rule: waiting for approval unless the settings let AI tools
   *  turn rules on. Returns the rule's id. */
  simulateMcpRule(client: string, rule: RuleDraft): string {
    const draft = validateRuleDraft(rule);
    const pending = this.state.settings.mcp.write !== "enable_rules";
    const added = this.ruleFrom(draft, this.uuid(), this.now(), { kind: "mcp", client }, pending);
    const rules = [...this.state.rules, added];
    checkRules(rules);
    this.emit({ type: "rules", rules });
    const activity = this.noteMcp(client, "add_rule", { subject: added.name, pending });
    this.mcpUndo.set(activity.id, { list: "rule", after: added });
    return added.id;
  }

  /** The preview's AI tool reads something (`count` items). */
  simulateMcpRead(client: string, action: McpAction, count?: number) {
    const range =
      action === "search_history" || action === "read_entry"
        ? this.state.settings.mcp.history_range
        : undefined;
    this.noteMcp(client, action, {
      ...(count === undefined ? {} : { count }),
      ...(range === undefined ? {} : { range }),
    });
  }

  /** `mcp_undo`: put back what a write changed while the item is as the write left it. */
  private undoMcp(id: string) {
    const activity = this.state.mcp.activity.find((a) => a.id === id);
    const undo = this.mcpUndo.get(id);
    if (activity === undefined) {
      this.emit({ type: "error", message: "mcp: 没有这条活动记录" });
      return;
    }
    if (activity.undone) {
      this.emit({ type: "error", message: "mcp: 这条活动已经撤销" });
      return;
    }
    if (undo === undefined || !activity.undoable) {
      this.emit({ type: "error", message: "mcp: 这条活动不能撤销" });
      return;
    }
    if (undo.list === "entry") {
      const current = this.state.dictionary.find((e) => e.id === undo.after.id);
      if (current === undefined) {
        this.emit({ type: "error", message: "mcp: 无法撤销：这个词条已经删除" });
        return;
      }
      if (JSON.stringify(current) !== JSON.stringify(undo.after)) {
        this.emit({ type: "error", message: "mcp: 无法撤销：这个词条之后又被修改过" });
        return;
      }
      const before = undo.before;
      this.emit({
        type: "dictionary",
        entries:
          before === undefined
            ? this.state.dictionary.filter((e) => e.id !== current.id)
            : this.state.dictionary.map((e) => (e.id === current.id ? before : e)),
      });
    } else {
      const current = this.state.rules.find((r) => r.id === undo.after.id);
      if (current === undefined) {
        this.emit({ type: "error", message: "mcp: 无法撤销：这条规则已经删除" });
        return;
      }
      if (JSON.stringify(current) !== JSON.stringify(undo.after)) {
        this.emit({ type: "error", message: "mcp: 无法撤销：这条规则之后又被修改过" });
        return;
      }
      const before = undo.before;
      this.emit({
        type: "rules",
        rules:
          before === undefined
            ? this.state.rules.filter((r) => r.id !== current.id)
            : this.state.rules.map((r) => (r.id === current.id ? before : r)),
      });
    }
    this.mcpUndo.delete(id);
    this.emit({
      type: "mcp",
      ...this.state.mcp,
      activity: this.state.mcp.activity.map((a) =>
        a.id === id ? { ...a, undone: true, undoable: false } : a,
      ),
    });
  }

  /** The service runs on a computer only (docs/dictation.md §23.6). */
  private requireServe() {
    if (this.role === "phone") throw new Error("serve: 本机服务仅在电脑上提供");
  }

  /** What the preview's service does with `serve`: a port in `MOCK_TAKEN_PORTS` cannot be bound. */
  private serveStatusFor(serve: ServeSettings): ServeStatus {
    if (this.role === "phone") return { available: false, phase: "off" };
    if (!serve.enabled) return { available: true, phase: "off" };
    if (MOCK_TAKEN_PORTS.includes(serve.port))
      return {
        available: true,
        phase: "failed",
        error: `无法监听 127.0.0.1:${serve.port}：Address already in use`,
      };
    return { available: true, phase: "running", address: `http://127.0.0.1:${serve.port}/v1` };
  }

  // ---- scenes (docs/dictation.md §18) ------------------------------------------------------------

  /** Which desktop's default applications the built-in scenes get (the identity's). */
  private builtinPlatform(): "windows" | "macos" | "linux" | "android" | undefined {
    // The phone keeps the built-in scenes without applications (user decision 2026-10-01).
    if (this.role === "phone") return "android";
    const platform = this.state.identity?.platform;
    return platform === "windows" || platform === "macos" || platform === "linux"
      ? platform
      : undefined;
  }

  /** `SceneStore::fill_builtin`: the categories `scenes` lacks, appended switched off (§18.10). The
   *  preview's ids are fixed per category (the core's are random and kept in `scenes.json`). */
  private withBuiltinScenes(scenes: Scene[]): Scene[] {
    const platform = this.builtinPlatform();
    if (platform === undefined) return scenes;
    const out = [...scenes];
    MOCK_BUILTIN_SCENES.forEach((row, i) => {
      if (out.some((s) => s.builtin === row.id)) return;
      const now = this.now();
      out.push({
        id: builtinSceneId(i),
        ...structuredClone(row.templates[platform]),
        created_at_ms: now,
        updated_at_ms: now,
        builtin: row.id,
      });
    });
    return out;
  }

  /** `scenes_builtin`: every built-in scene's term pack. */
  async scenesBuiltin(): Promise<BuiltinSceneTerms[]> {
    await Promise.resolve();
    return MOCK_BUILTIN_SCENES.map((row) => ({
      id: row.id,
      terms: [...row.terms],
    }));
  }

  private commitScenes(scenes: Scene[]) {
    try {
      checkScenes(scenes);
    } catch (e) {
      this.emit({
        type: "error",
        message: e instanceof Error ? e.message : String(e),
      });
      return;
    }
    this.emit({ type: "scenes", scenes });
  }

  private sceneFrom(
    draft: SceneDraft,
    id: string,
    createdAtMs: number,
    builtin?: BuiltinScene,
  ): Scene {
    return {
      id,
      ...draft,
      created_at_ms: createdAtMs,
      updated_at_ms: this.now(),
      ...(builtin === undefined ? {} : { builtin }),
    };
  }

  // ---- presets (docs/dictation.md §21) -----------------------------------------------------------

  private commitPresets(presets: CustomPreset[]) {
    try {
      checkPresets(presets);
    } catch (e) {
      this.emit({
        type: "error",
        message: e instanceof Error ? e.message : String(e),
      });
      return;
    }
    this.emit({ type: "presets", presets });
  }

  private presetFrom(draft: PresetDraft, id: string, createdAtMs: number): CustomPreset {
    return {
      id,
      ...draft,
      created_at_ms: createdAtMs,
      updated_at_ms: this.now(),
    };
  }

  /** 试一试 (`presets_try`): refused at once without a clean-up, like the core; otherwise the
   *  canned clean-up answers after `MOCK_REFINE_MS`. Nothing is saved or recorded. */
  /** `CoreCommand::HistoryProcess` (docs/dictation.md §22): a part every `MOCK_REFINE_MS`, 要点纪要
   *  one request more when it has several parts, then the result stored with the entry. */
  private processEntry(requestId: number, id: string, preset: string) {
    const event = (state: ProcessState) => {
      this.emit({ type: "history_process", request_id: requestId, id, state });
    };
    const entry = this.history.find((e) => e.id === id);
    if (!this.state.engines.refine_ready)
      return event({ state: "failed", reason: MOCK_PROCESS_UNCONFIGURED });
    if (entry === undefined) return event({ state: "failed", reason: MOCK_PROCESS_ENTRY_GONE });
    const resolved = resolvePreset(preset, this.state.presets).preset;
    const parts = Math.max(1, Math.ceil(Array.from(entry.text).length / MOCK_PART_MAX_CHARS));
    const total = parts + (resolved.id === "notes" && parts > 1 ? 1 : 0);
    let done = 0;
    event({ state: "running", done, total });
    const step = () => {
      done += 1;
      if (done < total) {
        event({ state: "running", done, total });
        this.processing.set(requestId, {
          id,
          timer: setTimeout(step, MOCK_REFINE_MS),
        });
        return;
      }
      this.processing.delete(requestId);
      const current = this.history.find((e) => e.id === id);
      if (current === undefined) return event({ state: "failed", reason: MOCK_PROCESS_ENTRY_GONE });
      const processed = {
        text: mockPresetOutput(
          isBuiltinPreset(resolved.id) ? resolved.id : undefined,
          current.text,
        ),
        preset: resolved,
        at_ms: this.now(),
      };
      this.setHistory(this.history.map((e) => (e.id === id ? { ...e, processed } : e)));
      event({ state: "done", processed });
    };
    this.processing.set(requestId, {
      id,
      timer: setTimeout(step, MOCK_REFINE_MS),
    });
  }

  private tryPreset(id: number, trial: PresetTrial, text: string) {
    const engines = this.state.engines;
    if (!engines.refine_ready) {
      this.emit({
        type: "preset_try",
        id,
        outcome: { status: "failed", reason: PRESET_TRY_UNCONFIGURED },
      });
      return;
    }
    const builtin =
      "preset" in trial ? resolvePreset(trial.preset, this.state.presets).preset.id : undefined;
    const handle = setTimeout(() => {
      this.probeTimers.delete(handle);
      const outcome: PresetTryOutcome = {
        status: "ok",
        text: mockPresetOutput(builtin, text),
        latency_ms: MOCK_REFINE_MS,
        model: engines.refine_model,
      };
      this.emit({ type: "preset_try", id, outcome });
    }, MOCK_REFINE_MS);
    this.probeTimers.add(handle);
  }

  /** The take's preset as the status and the history name it (the core's `take_preset`): the
   *  scene's, else the engines' as of the start, resolved against the presets of the start. */
  private resolveTakePreset(): PresetRef {
    const id = this.takeScene?.overrides.refine_preset ?? this.takePresetId;
    return resolvePreset(id, this.takePresets).preset;
  }

  // ---- personal dictionary and replacement rules (docs/dictation.md §16) -------------------------

  private commitDictionary(entries: DictionaryEntry[]) {
    try {
      checkDictionary(entries);
    } catch (e) {
      this.emit({
        type: "error",
        message: e instanceof Error ? e.message : String(e),
      });
      return;
    }
    this.emit({ type: "dictionary", entries });
  }

  private commitRules(rules: ReplacementRule[]) {
    try {
      checkRules(rules);
    } catch (e) {
      this.emit({
        type: "error",
        message: e instanceof Error ? e.message : String(e),
      });
      return;
    }
    this.emit({ type: "rules", rules });
  }

  private ruleFrom(
    draft: RuleDraft,
    id: string,
    createdAtMs: number,
    source: EntrySource = { kind: "manual" },
    pending = false,
  ): ReplacementRule {
    return {
      id,
      ...draft,
      // A rule waiting for approval is off until the user turns it on (docs/dictation.md §25).
      enabled: draft.enabled && !pending,
      created_at_ms: createdAtMs,
      updated_at_ms: this.now(),
      source,
      pending,
    };
  }

  /** `replace`: the file is the list; `merge`: same-name rules updated in place, the rest appended. */
  private importRules(drafts: RuleDraft[], mode: ImportMode) {
    const now = this.now();
    if (mode === "replace") {
      this.commitRules(drafts.map((d) => this.ruleFrom(d, this.uuid(), now)));
      return;
    }
    const rules = [...this.state.rules];
    for (const draft of drafts) {
      const at = rules.findIndex((r) => r.name === draft.name);
      const existing = rules[at];
      if (existing === undefined) rules.push(this.ruleFrom(draft, this.uuid(), now));
      else
        rules[at] = this.ruleFrom(
          draft,
          existing.id,
          existing.created_at_ms,
          existing.source,
          existing.pending && !draft.enabled,
        );
    }
    this.commitRules(rules);
  }

  async invoke<C extends MutationCommand>(name: C, ...args: ArgsOf<C>): Promise<void> {
    // Like the desktop shell: the desktop records a phone's takes and streams none itself (§20),
    // and has no share sheet (§20.7).
    if (this.role === "desktop" && name.startsWith("phone_take_"))
      throw new Error(PHONE_TAKE_UNAVAILABLE);
    if (this.role === "desktop" && name === "phone_share_text") throw new Error(SHARE_UNAVAILABLE);
    const payload: CommandArgs[C] | undefined = args[0];
    this.handlers[name](payload);
    await Promise.resolve();
  }

  // ---- remote-side simulation ------------------------------------------------------------------

  /** The other device joined the session: Noise XX runs, then both sides see the safety code.
   *  `scanned`: the phone read the computer's QR code and proved it, so the computer confirms by
   *  itself (docs/pairing.md 「扫码配对只在手机上确认」); otherwise both users confirm. */
  simulatePeerJoined(
    peer: DeviceInfo = this.role === "phone" ? desktopPeer() : phonePeer(),
    scanned = false,
  ) {
    const phase = this.state.pairing.state.state;
    if (phase !== "waiting_for_peer" && phase !== "creating_session") return;
    this.stopCountdown();
    this.emitPairing({ state: { state: "key_exchange" } });
    this.later(KEY_EXCHANGE_MS, () => {
      // The computer's side is the one that confirms by itself.
      const computer = this.role === "phone" ? { peer_confirmed: true } : { local_confirmed: true };
      this.emitPairing({
        state: { state: "awaiting_verification" },
        safety_code: this.safetyCode(),
        peer,
        scanned,
        ...(scanned ? computer : {}),
      });
      if (this.autoPeer) {
        this.later(this.autoPeer.confirmAfterMs, () => {
          this.simulatePeerConfirmed();
        });
      }
    });
  }

  simulatePeerConfirmed() {
    if (this.state.pairing.state.state !== "awaiting_verification") return;
    this.emitPairing({ peer_confirmed: true });
    if (this.state.pairing.local_confirmed) this.becomeTrusted();
  }

  simulatePeerRejected() {
    if (this.state.pairing.state.state === "idle") return;
    this.finish({ state: "rejected" });
  }

  simulatePeerLeft() {
    this.finish({ state: "failed", reason: { kind: "peer_left" } });
  }

  /** The selected model of `kind` ran out of quota (docs/dictation.md §3.5): the engines' status
   *  says it is tried again at `retryAtMs` while its fallback models run. */
  simulateQuotaExhausted(kind: ServiceKind, retryAtMs: number) {
    this.quotaOut.set(kind, retryAtMs);
    this.emit({ type: "engines", ...this.resolveEngines(this.state.settings.engines) });
  }

  /** The built-in service reports another day (docs/dictation.md §27). */
  simulateUsage(patch: Partial<UsageView>) {
    this.emit({ type: "usage", ...this.state.usage, ...structuredClone(patch) });
  }

  simulateRelay(patch: Partial<RelayStatus>) {
    this.emit({ type: "relay", ...this.state.relay, ...patch });
  }

  simulateDeviceConnection(publicKey: string, connection: DeviceConnection) {
    const devices = this.state.devices.map((d) =>
      d.device.public_key === publicKey
        ? {
            ...d,
            connection,
            device:
              connection.state === "online"
                ? { ...d.device, last_seen: Math.floor(this.now() / 1000) }
                : d.device,
          }
        : d,
    );
    this.emit({ type: "devices", devices });
  }

  /** A trusted device presented a different key: flag it, never auto-trust (R-PR-6). */
  simulateIdentityChanged(publicKey: string, presentedFingerprint: string) {
    const previous = this.state.devices.find((d) => d.device.public_key === publicKey);
    if (!previous) return;
    this.emit({
      type: "identity_changed",
      previous: previous.device,
      presented_fingerprint: presentedFingerprint,
    });
    this.simulateDeviceConnection(publicKey, {
      state: "identity_changed",
      presented_fingerprint: presentedFingerprint,
    });
  }

  simulateMessage(from: string, body: string) {
    this.emit({ type: "message", from, body });
  }

  simulateError(message: string) {
    this.emit({ type: "error", message });
  }

  /** Stop every timer; call from test teardown or when the app unmounts. */
  destroy() {
    this.clearTimers();
    for (const { timer } of this.processing.values()) clearTimeout(timer);
    this.processing.clear();
    this.clearDictationTimers();
    this.clearLiveTimers();
    this.clearExtraStop();
    this.clearUpdateTimers();
    this.stopImportTimer();
    this.dropListeners.clear();
    for (const t of this.modelTimers.values()) clearTimeout(t);
    this.modelTimers.clear();
    this.stopModelsMove();
    this.listeners.clear();
  }

  // ---- local model simulation (docs/dictation.md §10) -------------------------------------------

  /** The core's test hook: a download that failed mid-way (network, checksum), or whose package
   *  could not be used (`pkg`, docs/dictation.md §29.12). The `.part` files stay, so
   *  `model_download` on the same id is the retry. */
  simulateModelFailed(id: string, message: string, pkg?: PackageProblem) {
    if (!this.catalogueRow(id)) return;
    this.stopModelTimer(id);
    this.setModelState(
      id,
      pkg === undefined ? { kind: "failed", message } : { kind: "failed", message, package: pkg },
    );
  }

  private catalogueRow(id: string) {
    return MOCK_MODEL_CATALOGUE.find((row) => row.id === id);
  }

  /** The catalogue id the engine settings resolve to in local mode (the first row by default). */
  private resolvedLocalModel(settings: EngineSettings): string {
    return settings.local_model ?? MOCK_MODEL_CATALOGUE[0]?.id ?? "";
  }

  private modelState(id: string): ModelInstallState | undefined {
    return this.state.models.find((m) => m.id === id)?.state;
  }

  /** The catalogue list with `active` recomputed from the given settings. */
  private modelsFor(settings: EngineSettings): ModelState[] {
    const local =
      settings.asr_provider === "local" ||
      (settings.asr_provider === "builtin" && this.builtIn.asr === undefined);
    const active = local ? this.resolvedLocalModel(settings) : undefined;
    return this.state.models.map((m) => ({ ...m, active: m.id === active }));
  }

  private setModelState(id: string, state: ModelInstallState) {
    const models = this.state.models.map((m) => (m.id === id ? { ...m, state } : m));
    this.emit({ type: "models", models });
    // Readiness and the on-device card follow the install state of the selected model and of the
    // streaming model, whichever provider is in use (the core's rescan re-reports the engines);
    // progress ticks change neither.
    const engines = this.resolveEngines(this.state.settings.engines);
    if (JSON.stringify(engines) !== JSON.stringify(this.state.engines))
      this.emit({ type: "engines", ...engines });
  }

  private downloadModel(id: string) {
    if (this.refuseWhileMoving()) return;
    const row = this.catalogueRow(id);
    const current = this.modelState(id);
    if (!row || !current) {
      this.emit({
        type: "error",
        message:
          this.role === "phone" ? "models: 手机端不支持本地模型" : `models: 未知的本地模型 ${id}`,
      });
      return;
    }
    if (
      current.kind === "installed" ||
      current.kind === "downloading" ||
      current.kind === "verifying"
    )
      return;
    this.stopModelTimer(id);
    const total = row.size_bytes;
    // A model that comes as a package downloads the package (docs/dictation.md §29.12).
    const file = row.package?.name ?? MOCK_MODEL_FILE;
    const tick = (n: number) => {
      if (n > MOCK_MODEL_TICKS) {
        this.setModelState(id, { kind: "verifying" });
        this.laterModel(id, () => {
          this.setModelState(id, {
            kind: "installed",
            path: `${MOCK_MODELS_ROOT}/${id}`,
            installed_at: Math.floor(this.now() / 1000),
          });
        });
        return;
      }
      this.setModelState(id, {
        kind: "downloading",
        received: Math.round((total * n) / MOCK_MODEL_TICKS),
        total,
        file,
      });
      this.laterModel(id, () => {
        tick(n + 1);
      });
    };
    // The first progress event carries 0 bytes so the UI flips to the download row at once.
    this.setModelState(id, {
      kind: "downloading",
      received: 0,
      total,
      file,
    });
    this.laterModel(id, () => {
      tick(1);
    });
  }

  private cancelModel(id: string) {
    const current = this.modelState(id);
    if (current?.kind !== "downloading" && current?.kind !== "verifying") return;
    this.stopModelTimer(id);
    this.setModelState(id, { kind: "not_installed" });
  }

  private removeModel(id: string) {
    if (this.refuseWhileMoving()) return;
    const current = this.modelState(id);
    if (current === undefined || current.kind === "not_installed") return;
    this.stopModelTimer(id);
    this.setModelState(id, { kind: "not_installed" });
  }

  /** `model_import` (docs/dictation.md §10): a verifying beat, then installed, or what
   *  {@link simulateImportProblems} said the directory lacks. */
  private importModel(id: string) {
    if (this.refuseWhileMoving()) return;
    const current = this.modelState(id);
    if (!this.catalogueRow(id) || current === undefined) {
      this.emit({
        type: "error",
        message:
          this.role === "phone" ? "models: 手机端不支持本地模型" : `models: 未知的本地模型 ${id}`,
      });
      return;
    }
    if (
      current.kind === "installed" ||
      current.kind === "downloading" ||
      current.kind === "verifying"
    )
      return;
    this.stopModelTimer(id);
    this.setModelState(id, { kind: "verifying" });
    this.laterModel(id, () => {
      const problems = this.importProblems.get(id);
      this.setModelState(
        id,
        problems === undefined
          ? {
              kind: "installed",
              path: `${MOCK_MODELS_ROOT}/${id}`,
              installed_at: Math.floor(this.now() / 1000),
            }
          : { kind: "import_incomplete", ...problems },
      );
    });
  }

  /** What the next imports of `id` find wrong (tests); `undefined`: the files are right. */
  simulateImportProblems(
    id: string,
    problems: { missing: string[]; mismatched: string[] } | undefined,
  ) {
    if (problems === undefined) this.importProblems.delete(id);
    else this.importProblems.set(id, problems);
  }

  private laterModel(id: string, fn: () => void) {
    this.stopModelTimer(id);
    const handle = setTimeout(() => {
      this.modelTimers.delete(id);
      fn();
    }, MOCK_MODEL_TICK_MS);
    this.modelTimers.set(id, handle);
  }

  private stopModelTimer(id: string) {
    const handle = this.modelTimers.get(id);
    if (handle !== undefined) clearTimeout(handle);
    this.modelTimers.delete(id);
  }

  // ---- moving the model library (docs/dictation.md §10.9) -------------------------------------

  private modelsMoveTimer: ReturnType<typeof setTimeout> | undefined;
  /** What the next folder dialog answers: a folder, or `null` for closed. */
  private nextModelsFolder: string | null = MOCK_CHOSEN_MODELS_DIR;
  /** The next move fails with this message instead of finishing (tests). */
  private modelsMoveFailure: string | undefined;

  /** What the next 更改位置 dialog answers (tests): a folder, or `null` (closed). */
  simulateModelsFolder(next: string | null) {
    this.nextModelsFolder = next;
  }

  /** The next move fails with `message`, the library where it was (tests). */
  simulateModelsMoveFails(message: string | undefined) {
    this.modelsMoveFailure = message;
  }

  /** `models_location_choose`: the folder dialog, then the move; `false` when it was closed. */
  modelsLocationChoose(_title: string): Promise<boolean> {
    if (this.role === "phone") return Promise.reject(new Error("models: 手机端不支持本地模型"));
    const folder = this.nextModelsFolder;
    if (folder === null) return Promise.resolve(false);
    this.relocateModels(folder);
    return Promise.resolve(true);
  }

  private refuseWhileMoving(): boolean {
    if (this.state.models_location.moving === null) return false;
    this.emit({ type: "error", message: "models: 正在移动模型，请稍后再试" });
    return true;
  }

  private emitModelsLocation(change: Partial<ModelsLocationView>) {
    this.emit({ type: "models_location", ...this.state.models_location, ...change });
  }

  /** As the core moves it: refused while a model downloads or a move runs; nothing to do when
   *  the library is there already; otherwise `MOCK_MODEL_TICKS` progress beats over the installed
   *  models' size, then every card names the new folder. */
  private relocateModels(to: string) {
    const location = this.state.models_location;
    if (!location.available) {
      this.emit({ type: "error", message: "models: 手机端不支持本地模型" });
      return;
    }
    if (location.moving !== null) {
      this.emit({ type: "error", message: "models: 正在移动模型" });
      return;
    }
    const busy = this.state.models.some(
      (m) => m.state.kind === "downloading" || m.state.kind === "verifying",
    );
    if (busy) {
      this.emit({ type: "error", message: "models: 正在下载模型，请等下载完成或取消后再移动" });
      return;
    }
    if (to === location.dir) {
      this.emitModelsLocation({ last: null });
      return;
    }
    const total = this.state.models
      .filter((m) => m.state.kind === "installed")
      .reduce((sum, m) => sum + m.size_bytes, 0);
    this.emitModelsLocation({ moving: { to, moved_bytes: 0, total_bytes: total }, last: null });
    const beat = (tick: number) => {
      this.modelsMoveTimer = setTimeout(() => {
        this.modelsMoveTimer = undefined;
        if (tick < MOCK_MODEL_TICKS) {
          const moved = Math.round((total * tick) / MOCK_MODEL_TICKS);
          this.emitModelsLocation({ moving: { to, moved_bytes: moved, total_bytes: total } });
          beat(tick + 1);
          return;
        }
        const failure = this.modelsMoveFailure;
        this.modelsMoveFailure = undefined;
        if (failure !== undefined) {
          this.emitModelsLocation({ moving: null, last: { kind: "failed", message: failure } });
          return;
        }
        const models = this.state.models.map((m) => ({
          ...m,
          dir: `${to}/${m.id}`,
          state: m.state.kind === "installed" ? { ...m.state, path: `${to}/${m.id}` } : m.state,
        }));
        this.emit({ type: "models", models });
        this.emitModelsLocation({
          dir: to,
          custom: to !== this.state.models_location.default_dir,
          moving: null,
          last: { kind: "moved", dir: to, deferred: 0 },
        });
      }, MOCK_MODEL_TICK_MS);
    };
    beat(1);
  }

  private cancelModelsMove() {
    if (this.state.models_location.moving === null) {
      this.emit({ type: "error", message: "models: 没有在移动模型" });
      return;
    }
    this.stopModelsMove();
    this.emitModelsLocation({ moving: null, last: { kind: "cancelled" } });
  }

  private stopModelsMove() {
    if (this.modelsMoveTimer !== undefined) clearTimeout(this.modelsMoveTimer);
    this.modelsMoveTimer = undefined;
  }

  // ---- update simulation -------------------------------------------------------------------------

  /** The core's test hook for the states the mock never reaches on its own (`failed`, `disabled`). */
  simulateUpdate(status: UpdateStatus) {
    this.clearUpdateTimers();
    this.emit({ type: "update", ...status });
  }

  private checkForUpdate() {
    const current = this.state.update.state;
    // From a store, the store updates (docs/dictation.md §20.9): nothing to ask.
    if (current === "disabled" || current === "checking" || current === "store") return;
    if (current === "downloading" || current === "installing") return;
    this.clearUpdateTimers();
    this.emit({ type: "update", state: "checking" });
    this.laterUpdate(MOCK_UPDATE_CHECK_MS, () => {
      this.emit({
        type: "update",
        state: "available",
        version: MOCK_AVAILABLE_VERSION,
        current: MOCK_CURRENT_VERSION,
        notes: MOCK_UPDATE_NOTES,
        date: "2026-09-25",
      });
    });
  }

  private installUpdate() {
    const current = this.state.update;
    if (this.role === "phone") {
      // docs/dictation.md §20.9: the phone downloads nothing itself. The page that installs the
      // update opens in the browser: the store listing, or the APK of the newer release.
      if (current.state === "store") this.updatePagesOpened.push("store");
      else if (current.state === "available") this.updatePagesOpened.push(current.version);
      else throw new Error(PHONE_NOTHING_TO_INSTALL);
      return;
    }
    if (current.state === "ready") {
      // docs/dictation.md §4.6.1: the backup before the update installs; a failed one does not
      // stop it.
      this.writeAutoBackup("update");
      this.emit({
        type: "update",
        state: "installing",
        version: current.version,
      });
      return;
    }
    if (current.state !== "available") return;
    this.clearUpdateTimers();
    const version = current.version;
    const tick = (n: number) => {
      if (n > MOCK_UPDATE_TICKS) {
        this.emit({ type: "update", state: "ready", version });
        this.laterUpdate(MOCK_UPDATE_TICK_MS, () => {
          this.writeAutoBackup("update");
          this.emit({ type: "update", state: "installing", version });
        });
        return;
      }
      this.emit({
        type: "update",
        state: "downloading",
        version,
        received: Math.round((MOCK_UPDATE_TOTAL_BYTES * n) / MOCK_UPDATE_TICKS),
        total: MOCK_UPDATE_TOTAL_BYTES,
      });
      this.laterUpdate(MOCK_UPDATE_TICK_MS, () => {
        tick(n + 1);
      });
    };
    tick(1);
  }

  private laterUpdate(ms: number, fn: () => void) {
    const handle = setTimeout(() => {
      this.updateTimers.delete(handle);
      fn();
    }, ms);
    this.updateTimers.add(handle);
  }

  private clearUpdateTimers() {
    for (const t of this.updateTimers) clearTimeout(t);
    this.updateTimers.clear();
  }

  /** `Effect::Record`: nothing when history is off, the newest `keep` otherwise; a recording of
   *  an entry that dropped out goes too (§26). */
  private recordHistory(entry: HistoryEntry) {
    const { enabled, keep } = this.state.settings.history;
    if (!enabled) return;
    this.setHistory([entry, ...this.history].slice(0, keep));
    this.followHistory();
  }

  // ---- providers (docs/dictation.md §3.3) ----------------------------------------------------------

  providerConsoleOpen(
    provider: ProviderId,
    page: ConsolePage = "key",
    site?: SiteId,
  ): Promise<void> {
    const spec = providerSpec(provider);
    if (page === "quota") {
      if (spec.quota !== true) return Promise.reject(new Error(`${provider}: no quota page`));
      this.quotaPagesOpened.push(provider);
      return Promise.resolve();
    }
    if (!spec.console) return Promise.reject(new Error(`${provider}: no key page`));
    this.consolesOpened.push(provider);
    this.consoleSites.push(site);
    return Promise.resolve();
  }

  projectLinkOpen(link: ProjectLink): Promise<void> {
    this.linksOpened.push(link);
    return Promise.resolve();
  }

  guideOpen(page: GuidePage, locale: string): Promise<void> {
    this.guidesOpened.push({ page, locale });
    return Promise.resolve();
  }

  // ---- manual model download (docs/dictation.md §10) -------------------------------------------

  modelFolderOpen(id: string): Promise<void> {
    if (!this.state.models.some((m) => m.id === id))
      return Promise.reject(new Error(`model_folder_open: no model ${id}`));
    this.modelFoldersOpened.push(id);
    return Promise.resolve();
  }

  modelLinkOpen(id: string, file: string, source: number): Promise<void> {
    const model = this.state.models.find((m) => m.id === id);
    // `file` may name the package the model comes in (docs/dictation.md §29.12).
    const urls =
      model?.package?.name === file
        ? model.package.urls
        : model?.files.find((f) => f.name === file)?.urls;
    const url = urls?.[source];
    if (url === undefined)
      return Promise.reject(new Error(`model_link_open: ${id} ${file} ${source}`));
    this.modelLinksOpened.push(url);
    return Promise.resolve();
  }

  // ---- feedback (docs/feedback.md) -------------------------------------------------------------

  /** `feedback_diagnostics`; the phone sends feedback of its own too (user decision 2026-10-01). */
  feedbackDiagnostics(locale: string): Promise<FeedbackInfo> {
    const engines = this.state.engines;
    const onDevice = engines.asr_provider === "local";
    const host = hostOsOf(this.state.identity?.platform ?? "windows");
    const phone = this.role === "phone";
    const diagnostics: FeedbackDiagnostics = {
      app_version: MOCK_CURRENT_VERSION,
      os: phone ? "android" : host === "other" ? "windows" : host,
      arch: phone ? "aarch64" : "x86_64",
      locale,
      asr_provider: engines.asr_provider,
      output_mode: engines.effective_output_mode,
      ...(onDevice && typeof engines.local_model === "string"
        ? { local_model: engines.local_model }
        : {}),
      ...(onDevice ? { compute: this.state.settings.engines.local_device } : {}),
      ...(engines.refine_enabled && engines.llm_provider !== undefined
        ? { llm_provider: engines.llm_provider }
        : {}),
    };
    return Promise.resolve({
      configured: this.feedback !== "not_configured",
      diagnostics,
    });
  }

  feedbackSubmit(draft: FeedbackDraft): Promise<FeedbackReceipt> {
    const message = draft.message.trim();
    const contact = draft.contact?.trim() ?? "";
    if (
      message.length === 0 ||
      message.length > FEEDBACK_MESSAGE_MAX ||
      contact.length > FEEDBACK_CONTACT_MAX
    )
      return Promise.reject(new Error("invalid"));
    const ids = draft.attachments ?? [];
    if (!ids.every((id) => this.feedbackStaged.some((a) => a.id === id)))
      return Promise.reject(new Error("invalid"));
    // What only a report with files can meet (the endpoint's 507, an unfinished upload).
    const onlyWithFiles = this.feedback === "storage_full" || this.feedback === "attachments";
    const failure = onlyWithFiles && ids.length === 0 ? "configured" : this.feedback;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.probeTimers.delete(timer);
        if (failure !== "configured" && failure !== "attachments") {
          reject(new Error(failure));
          return;
        }
        this.feedbackSent.push({
          ...draft,
          message,
          contact: contact.length > 0 ? contact : null,
        });
        // The report went out: the shell forgets its files, uploaded or not.
        for (const id of ids) void this.feedbackAttachmentRemove(id);
        if (failure === "attachments") reject(new Error(failure));
        else resolve({ id: `feedback-${this.feedbackSent.length}` });
      }, MOCK_FEEDBACK_MS);
      this.probeTimers.add(timer);
    });
  }

  /** `feedback_attachment_add`: the shell's checks, in its order (`feedback::Attachments::add`). */
  feedbackAttachmentAdd(file: AttachmentFile): Promise<StagedAttachment> {
    const name = cleanAttachmentName(file.name);
    if (name === undefined) return refuseAttachment("attachment_name");
    if (!(FEEDBACK_ATTACHMENT_TYPES as readonly string[]).includes(file.type))
      return refuseAttachment("attachment_type");
    const limit = file.type.startsWith("video/")
      ? FEEDBACK_MAX_VIDEO_BYTES
      : FEEDBACK_MAX_IMAGE_BYTES;
    if (file.bytes.length === 0 || file.bytes.length > limit)
      return refuseAttachment("attachment_too_large");
    if (this.feedbackStaged.length >= FEEDBACK_MAX_ATTACHMENTS)
      return refuseAttachment("attachment_too_many");
    const staged = this.feedbackStaged.reduce((n, a) => n + a.size, 0);
    if (staged + file.bytes.length > FEEDBACK_MAX_ATTACHMENT_TOTAL_BYTES)
      return refuseAttachment("attachment_total");
    this.lastAttachment += 1;
    const entry = {
      id: `attachment-${this.lastAttachment}`,
      name,
      type: file.type,
      size: file.bytes.length,
    };
    this.feedbackStaged.push(entry);
    return Promise.resolve({ ...entry });
  }

  /** `feedback_attachment_remove`: an id that is not staged is no error. */
  feedbackAttachmentRemove(id: string): Promise<void> {
    const at = this.feedbackStaged.findIndex((a) => a.id === id);
    if (at >= 0) this.feedbackStaged.splice(at, 1);
    return Promise.resolve();
  }

  /** `feedback_attachments_clear`. */
  feedbackAttachmentsClear(): Promise<void> {
    this.feedbackStaged.length = 0;
    return Promise.resolve();
  }

  phoneClipboardRead(): Promise<string | null> {
    if (this.role !== "phone") return Promise.reject(new Error(PHONE_TEXT_UNAVAILABLE));
    return Promise.resolve(this.phoneClipboard);
  }

  /** `provider_probe`: the same refusals as the core before any request, then the preset list (or
   *  `probeModels`) after `MOCK_PROBE_MS`. */
  private probe(provider: ProviderId, kind: ServiceKind, baseUrl?: string, key?: string) {
    const failed = (reason: ProbeFailure) =>
      this.emit({
        type: "provider_probe",
        provider,
        kind,
        result: "failed",
        reason,
      });
    const spec = providerSpec(provider);
    const preset = spec[kind];
    if (preset === undefined || provider === "local") return failed("unsupported");
    if (provider === "builtin" && this.builtIn[kind] === undefined) return failed("unsupported");
    let savedUrl: string | undefined;
    if (provider !== "builtin") {
      const saved = this.savedTarget(provider, kind);
      const url = baseUrl?.trim() || saved.url;
      if (url === undefined || !/^https?:\/\/[^/]+/.test(url)) return failed("invalid_url");
      const hasKey = (key?.trim().length ?? 0) > 0 || saved.key;
      if (spec.key === "required" && !hasKey) return failed("key_missing");
      // The saved values, as the core sees them: no key typed, the saved address or none.
      if ((key?.trim().length ?? 0) === 0 && url === saved.url) savedUrl = url;
    }
    const handle = setTimeout(() => {
      this.probeTimers.delete(handle);
      if (this.probeModels === false) return failed("unreachable");
      const models = this.listOf(provider, kind);
      if (savedUrl !== undefined) this.storeList(provider, kind, savedUrl, models);
      this.emit({
        type: "provider_probe",
        provider,
        kind,
        result: "ok",
        models,
        latency_ms: MOCK_PROBE_MS,
      });
    }, MOCK_PROBE_MS);
    this.probeTimers.add(handle);
  }

  /** The saved endpoint of a provider's service and whether its key is saved (`probe_target`
   *  with no draft). */
  private savedTarget(provider: ProviderId, kind: ServiceKind): { url?: string; key: boolean } {
    const preset = providerSpec(provider)[kind];
    const saved = this.state.settings.engines.providers?.[provider];
    const url =
      (kind === "asr" ? saved?.asr_url : saved?.llm_url)?.trim() ||
      (preset !== undefined && preset.baseUrl.length > 0 ? preset.baseUrl : undefined);
    const entry = keyEntry(provider, kind);
    return {
      ...(url === undefined ? {} : { url }),
      key: entry !== undefined && this.userKeys.has(entry),
    };
  }

  /** What the pretend provider lists: `probeModels`, else the presets (the built-in service's own
   *  models), sorted. */
  private listOf(provider: ProviderId, kind: ServiceKind): string[] {
    const pretend = this.probeModels === false ? undefined : this.probeModels[provider];
    const listed = [
      ...(pretend ??
        (provider === "builtin"
          ? (this.builtIn[kind]?.models ?? [this.builtIn[kind]?.model ?? ""])
          : (providerSpec(provider)[kind]?.models ?? []))),
    ];
    listed.sort();
    return listed;
  }

  private storeList(provider: ProviderId, kind: ServiceKind, url: string, models: string[]) {
    const slot = `${provider}:${kind}` as const;
    const old = this.listedModels.get(slot);
    if (old?.url === url && JSON.stringify(old.models) === JSON.stringify(models)) return;
    this.listedModels.set(slot, { url, models });
    this.emit({ type: "engines", ...this.resolveEngines(this.state.settings.engines) });
  }

  /** `provider_models_refresh`: the saved endpoint and key's list after `MOCK_PROBE_MS`; nothing
   *  for the built-in and on-device providers, without a saved key or address, or while the
   *  provider is unreachable. */
  private refreshModels(provider: ProviderId, kind: ServiceKind) {
    const spec = providerSpec(provider);
    if (provider === "builtin" || provider === "local" || spec[kind] === undefined) return;
    const saved = this.savedTarget(provider, kind);
    const url = saved.url;
    if (url === undefined || (spec.key === "required" && !saved.key)) return;
    this.modelRefreshes += 1;
    const handle = setTimeout(() => {
      this.probeTimers.delete(handle);
      if (this.probeModels === false) return;
      // The address may have changed meanwhile: the list is the endpoint's it came from.
      if (this.savedTarget(provider, kind).url !== url) return;
      this.storeList(provider, kind, url, this.listOf(provider, kind));
    }, MOCK_PROBE_MS);
    this.probeTimers.add(handle);
  }

  // ---- dictation simulation ----------------------------------------------------------------------

  /** Resolve the settings against the pretend built-in service and the stored keys, exactly as
   *  the core reports them (`resolveEngineStatus`, shared with nothing else: the real app asks
   *  the core). */
  private resolveEngines(settings: EngineSettings): EngineStatus {
    const localId = this.resolvedLocalModel(settings);
    const localRow = this.catalogueRow(localId);
    const status = resolveEngineStatus({
      settings,
      userKeys: this.userKeys,
      builtIn: this.builtIn,
      local: {
        id: localId,
        name: localRow?.name ?? localId,
        installed: this.modelState(localId)?.kind === "installed",
      },
      liveReady:
        settings.live_preview && this.modelState(MOCK_STREAMING_MODEL_ID)?.kind === "installed",
      quotaOut: Object.fromEntries(this.quotaOut),
      listed: Object.fromEntries(this.listedModels),
    });
    return { ...status, ...this.engineOverrides };
  }

  private emitPhase(phase: DictationPhase, session = this.state.dictation.session) {
    // The take's context rides next to the phase until the take is over (§18.6), and so does the
    // phone a take's audio comes from (§20).
    if (phase.phase === "idle") {
      this.takeContext = undefined;
      this.takeRemote = undefined;
      this.takePreset = undefined;
      this.takeSource = undefined;
      this.takeRecording = undefined;
      this.takeRetake = false;
    }
    if (phase.phase !== "listening" && phase.phase !== "processing") this.takeSegments = undefined;
    const context = this.takeContext === undefined ? {} : { context: this.takeContext };
    const remote = this.takeRemote === undefined ? {} : { remote: this.takeRemote };
    const preset = this.takePreset === undefined ? {} : { preset: this.takePreset };
    const source = this.takeSource === undefined ? {} : { source: this.takeSource };
    const segments = this.takeSegments === undefined ? {} : { segments: this.takeSegments };
    const recording = this.takeRecording === undefined ? {} : { recording: this.takeRecording };
    const retake = this.takeRetake ? { retake: true } : {};
    this.emit({
      type: "dictation",
      session,
      phase,
      ...context,
      kind: this.takeKind,
      ...remote,
      ...preset,
      ...source,
      ...segments,
      ...recording,
      ...retake,
    });
  }

  // ---- importing audio and video files (docs/dictation.md §31) ---------------------------------

  /** `imports_state`: the import queue. */
  importsState(): Promise<ImportsView> {
    return Promise.resolve(structuredClone(this.state.imports));
  }

  /** The next `importPick` picks `paths`; `[]` is a dialog that was cancelled. */
  simulateImportPick(paths: readonly string[]) {
    this.nextImportPick = [...paths];
  }

  /** `import_pick`: the files the open dialog picked, `MOCK_IMPORT_PICK` unless told otherwise.
   *  The phone has no such dialog. */
  async importPick(title: string, media: string, all: string): Promise<string[]> {
    await Promise.resolve();
    if (!this.state.imports.available) return [];
    this.importPicks.push({ title, media, all });
    const picked = this.nextImportPick ?? [...MOCK_IMPORT_PICK];
    this.nextImportPick = undefined;
    return picked;
  }

  onFileDrop(listener: DropListener): Unsubscribe {
    this.dropListeners.add(listener);
    return () => {
      this.dropListeners.delete(listener);
    };
  }

  /** How many listen for dropped files (tests: a page subscribes after it renders). */
  fileDropListeners(): number {
    return this.dropListeners.size;
  }

  /** Files dropped on the window, as the shell reports them (`tauri://drag-drop`). */
  simulateFileDrop(paths: readonly string[]) {
    for (const listener of this.dropListeners) listener([...paths]);
  }

  /** What `path` holds for the import from now on; the rest as `MOCK_IMPORT_FILE`. */
  simulateImportFile(path: string, file: Partial<MockImportFile>) {
    this.importFiles.set(path, { ...MOCK_IMPORT_FILE, ...file });
  }

  /** `import_add` as the core checks it (`Runtime::import_add`): refused on the phone, with the
   *  history off, with the built-in service and with models that cannot run; otherwise every file
   *  joins the queue. */
  private importAdd({ paths, asr, refine, language }: CommandArgs["import_add"]) {
    const refused = this.importRefusal(asr, refine);
    if (refused !== undefined) {
      this.emit({ type: "error", message: refused });
      return;
    }
    const hint = language?.trim() ?? "";
    const jobs = paths.map((path): ImportJob => {
      const id = this.uuid();
      this.importRuns.set(id, {
        path,
        asr,
        ...(refine === undefined ? {} : { refine }),
        ...(hint === "" ? {} : { language: hint }),
        decoded: false,
        recognized: 0,
      });
      const { size } = this.importFile(path);
      return { id, name: baseName(path), size, paused: false, state: { kind: "queued" } };
    });
    this.setImports([...this.state.imports.jobs, ...jobs]);
    this.startImport();
  }

  /** Why an import with these models is refused, as the core words it; `undefined` when it may
   *  run. An installed local model, a cloud provider with the user's own key, or Ollama and the
   *  custom endpoint as the user set them up (D20) will do. */
  private importRefusal(asr: RecognitionChoice, refine: PolishModel | undefined) {
    if (!this.state.imports.available) return MOCK_IMPORT_UNAVAILABLE;
    if (!this.state.settings.history.enabled) return MOCK_IMPORT_HISTORY_OFF;
    if ((asr.kind === "remote" && asr.provider === "builtin") || refine?.provider === "builtin")
      return MOCK_IMPORT_BUILTIN;
    // §31.4 (D20): a cloud provider is the user's own by their key; Ollama and the custom endpoint
    // are as the user set them up, key or not.
    const missingKey = (provider: ProviderId, kind: ServiceKind) => {
      if (providerSpec(provider).key !== "required") return false;
      const entry = keyEntry(provider, kind);
      return entry === undefined || !this.userKeys.has(entry);
    };
    if (
      (asr.kind === "remote" && missingKey(asr.provider, "asr")) ||
      (refine !== undefined && missingKey(refine.provider, "llm"))
    )
      return MOCK_IMPORT_NO_KEY;
    // An import never falls back to another model (docs/dictation.md §31).
    const {
      asr_fallback: _asr,
      llm_fallback: _llm,
      ...engines
    } = applyRetakeModels(this.state.settings.engines, {
      asr,
      ...(refine === undefined ? {} : { refine: { kind: "remote", ...refine } }),
    });
    const status = this.resolveEngines(engines);
    if (!status.asr_ready || (refine !== undefined && !status.refine_ready))
      return MOCK_RETAKE_NOT_READY;
    return undefined;
  }

  /** `import_cancel` and `import_remove`: the job leaves the queue at once; a running one stops
   *  and writes nothing. */
  private importDrop(id: string) {
    if (!this.state.imports.jobs.some((j) => j.id === id)) return;
    if (this.importRunning === id) {
      this.stopImportTimer();
      this.importRunning = undefined;
    }
    this.importRuns.delete(id);
    this.setImports(this.state.imports.jobs.filter((j) => j.id !== id));
    this.startImport();
  }

  /** `import_retry`: a failed job waits again, going on from the segments it recognised when it
   *  can, otherwise from the start. A job the mock started with has nothing to run. */
  private importRetry(id: string) {
    const job = this.state.imports.jobs.find((j) => j.id === id);
    const run = this.importRuns.get(id);
    if (job?.state.kind !== "failed" || run === undefined) return;
    if (!job.state.resumable) {
      run.decoded = false;
      run.recognized = 0;
    }
    this.setJob(id, { state: { kind: "queued" }, paused: false });
    this.startImport();
  }

  /** Start the first waiting job when none runs (`Runtime::start_import`); one whose models cannot
   *  run now (a key removed since, the history turned off) fails at once and the next is tried. */
  private startImport() {
    if (this.importRunning !== undefined) return;
    for (const job of this.state.imports.jobs) {
      const run = this.importRuns.get(job.id);
      if (job.state.kind !== "queued" || run === undefined) continue;
      const refused = this.importRefusal(run.asr, run.refine);
      if (refused !== undefined) {
        const failure: ImportFailure = { code: "recognition", detail: refused };
        this.setJob(job.id, { state: { kind: "failed", failure, resumable: run.decoded } });
        continue;
      }
      this.importRunning = job.id;
      if (run.decoded) this.importRecognizing(job.id, run);
      else this.importDecode(job.id, run, 0);
      return;
    }
  }

  /** Decoding, a quarter at a time; a file the decoder cannot read fails after the first. */
  private importDecode(id: string, run: MockImportRun, quarter: number) {
    const file = this.importFile(run.path);
    if (quarter === 0) {
      this.setJob(id, { state: { kind: "decoding" }, paused: false });
      this.importLater(() => this.importDecode(id, run, 1));
      return;
    }
    if (file.failure !== undefined && DECODE_FAILURES.has(file.failure.code)) {
      this.importFailed(id, file.failure, false);
      return;
    }
    this.setJob(id, {
      state: { kind: "decoding", pct: quarter * 25 },
      duration_ms: file.duration_ms,
    });
    if (quarter < 4) {
      this.importLater(() => this.importDecode(id, run, quarter + 1));
      return;
    }
    run.decoded = true;
    this.importLater(() => this.importRecognizing(id, run));
  }

  /** Recognition from the first segment not recognised yet. */
  private importRecognizing(id: string, run: MockImportRun) {
    const total = mockImportSegments(this.importFile(run.path).duration_ms);
    this.setJob(id, { state: { kind: "recognizing", done: run.recognized, total }, paused: false });
    this.importLater(() => this.importRecognize(id, run, total));
  }

  /** One segment recognised, after any dictation that runs (dictation always goes first). */
  private importRecognize(id: string, run: MockImportRun, total: number) {
    if (this.importWaits(id, () => this.importRecognize(id, run, total))) return;
    const file = this.importFile(run.path);
    if (file.failure?.code === "recognition" && (file.failAt ?? 0) === run.recognized) {
      // It fails once: 重试 goes on from this segment.
      const { failure, failAt: _failAt, ...rest } = file;
      this.importFiles.set(run.path, rest);
      this.importFailed(id, failure, true);
      return;
    }
    run.recognized += 1;
    this.setJob(id, { state: { kind: "recognizing", done: run.recognized, total }, paused: false });
    if (run.recognized < total) {
      this.importLater(() => this.importRecognize(id, run, total));
    } else if (file.failure?.code === "no_speech") {
      this.importFailed(id, file.failure, false);
    } else if (run.refine === undefined) {
      this.importLater(() => this.importFinish(id, run, total));
    } else {
      this.setJob(id, { state: { kind: "polishing", done: 0, total } });
      this.importLater(() => this.importPolish(id, run, total, 0));
    }
  }

  /** One segment polished, after any dictation that runs. */
  private importPolish(id: string, run: MockImportRun, total: number, done: number) {
    if (this.importWaits(id, () => this.importPolish(id, run, total, done))) return;
    this.setJob(id, { state: { kind: "polishing", done: done + 1, total }, paused: false });
    if (done + 1 < total) this.importLater(() => this.importPolish(id, run, total, done + 1));
    else this.importLater(() => this.importFinish(id, run, total));
  }

  /** A job waits while a dictation runs, saying so; `true` while it does. */
  private importWaits(id: string, next: () => void): boolean {
    const phase = this.state.dictation.phase.phase;
    if (phase !== "listening" && phase !== "processing") return false;
    const job = this.state.imports.jobs.find((j) => j.id === id);
    if (job?.paused === false) this.setJob(id, { paused: true });
    this.importLater(next);
    return true;
  }

  /** The job's entry goes into the history (`Runtime::finish_import`): its segments keep their
   *  times, and their polished text when it was polished. */
  private importFinish(id: string, run: MockImportRun, total: number) {
    this.importRunning = undefined;
    if (!this.state.settings.history.enabled) {
      const failure: ImportFailure = { code: "storage", detail: MOCK_IMPORT_HISTORY_OFF };
      run.decoded = false;
      run.recognized = 0;
      this.setJob(id, { state: { kind: "failed", failure, resumable: false }, paused: false });
      this.startImport();
      return;
    }
    const { duration_ms } = this.importFile(run.path);
    const vocabulary = Vocabulary.compile(this.state.dictionary, this.state.rules);
    const polished = run.refine !== undefined;
    const hits: VocabularyHits = { corrections: [], rules: [] };
    const segments: LiveSegment[] = [];
    const raws: string[] = [];
    const finals: string[] = [];
    for (let i = 0; i < total; i++) {
      const line = MOCK_IMPORT_SEGMENTS[i % MOCK_IMPORT_SEGMENTS.length] ?? {
        raw: "",
        refined: "",
      };
      const corrected = vocabulary.correct(line.raw);
      hits.corrections.push(...corrected.hits);
      const ruled = vocabulary.applyRules(polished ? line.refined : corrected.text);
      hits.rules.push(...ruled.hits);
      segments.push({
        text: line.raw,
        ...(polished ? { refined: ruled.text } : {}),
        start_ms: i * MOCK_IMPORT_SEGMENT_MS,
        end_ms: Math.min(duration_ms, (i + 1) * MOCK_IMPORT_SEGMENT_MS),
      });
      raws.push(line.raw);
      finals.push(ruled.text);
    }
    const asrModel =
      run.asr.kind === "remote"
        ? run.asr.model
        : (this.catalogueRow(run.asr.id)?.name ?? run.asr.id);
    const entry: HistoryEntry = {
      id: this.uuid(),
      at_ms: this.now(),
      raw_text: raws.join(""),
      text: finals.join(""),
      refined: polished,
      asr_model: asrModel,
      ...(run.refine === undefined
        ? {}
        : {
            refine_model: run.refine.model,
            refine_ms: MOCK_REFINE_MS * total,
            preset: resolvePreset(DEFAULT_PRESET, []).preset,
          }),
      duration_ms,
      asr_ms: MOCK_ASR_MS * total,
      outcome: { kind: "inserted", via: "clipboard" },
      starred: false,
      mode: "whole_take",
      segments,
      ...(hits.corrections.length + hits.rules.length > 0 ? { vocabulary: hits } : {}),
      kind: "dictation",
      file: { name: baseName(run.path), duration_ms },
    };
    this.recordHistory(entry);
    this.importRuns.delete(id);
    this.setJob(id, {
      state: { kind: "done", entry: entry.id, polish_failed: 0 },
      paused: false,
    });
    this.startImport();
  }

  /** The running job stopped; the next one starts. A failure that cannot go on starts again. */
  private importFailed(id: string, failure: ImportFailure, resumable: boolean) {
    const run = this.importRuns.get(id);
    if (run !== undefined && !resumable) {
      run.decoded = false;
      run.recognized = 0;
    }
    this.importRunning = undefined;
    this.setJob(id, { state: { kind: "failed", failure, resumable }, paused: false });
    this.startImport();
  }

  /** What `path` holds (`simulateImportFile`, else `MOCK_IMPORT_FILE`). */
  private importFile(path: string): MockImportFile {
    return this.importFiles.get(path) ?? MOCK_IMPORT_FILE;
  }

  /** Replace the queue and tell the UI, as `CoreEvent::Imports`. */
  private setImports(jobs: ImportJob[]) {
    const { available, extensions } = this.state.imports;
    this.emit({ type: "imports", available, extensions, jobs });
  }

  /** Change one job and tell the UI. */
  private setJob(id: string, change: Partial<Omit<ImportJob, "id">>) {
    this.setImports(
      this.state.imports.jobs.map((job) => (job.id === id ? { ...job, ...change } : job)),
    );
  }

  /** The running import's next step. */
  private importLater(fn: () => void) {
    this.stopImportTimer();
    this.importTimer = setTimeout(() => {
      this.importTimer = undefined;
      fn();
    }, MOCK_IMPORT_STEP_MS);
  }

  private stopImportTimer() {
    if (this.importTimer !== undefined) clearTimeout(this.importTimer);
    this.importTimer = undefined;
  }

  // ---- audio cache (docs/dictation.md §26) -----------------------------------------------------

  /** Whether a take keeps its recording: the desktop, the cache on, and the history too. */
  private keepsAudio(): boolean {
    const { audio_cache, history } = this.state.settings;
    return this.state.audio_cache.available && audio_cache.enabled && history.enabled;
  }

  /** Replace the kept recordings (newest first) and tell the UI, as `CoreEvent::AudioCache`. */
  private setRecordings(recordings: KeptRecording[]) {
    const current = this.state.audio_cache.recordings;
    if (JSON.stringify(current) === JSON.stringify(recordings)) return;
    const { available } = this.state.audio_cache;
    this.emit({ type: "audio_cache", ...cacheView(available, recordings) });
  }

  /** The newest recordings within the settings' count and size, as `AudioCache::enforce`. */
  private withinLimits(recordings: KeptRecording[]): KeptRecording[] {
    const { keep, max_mb } = this.state.settings.audio_cache;
    const kept = recordings.slice(0, keep);
    let total = kept.reduce((sum, r) => sum + r.bytes, 0);
    while (kept.length > 0 && total > max_mb * AUDIO_CACHE_MB) {
      total -= kept.pop()?.bytes ?? 0;
    }
    return kept;
  }

  /** `Effect::Keep`: a take about to be recognised keeps its recording, the status names it; a
   *  long take its compressed copy (§26.2). */
  private keepTakeRecording(durationMs: number) {
    if (!this.keepsAudio()) return;
    const id = this.uuid();
    this.takeRecording = id;
    const app = this.takeContext?.app;
    const long = durationMs > MOCK_IN_MEMORY_MS;
    const perSecond = long ? MOCK_OPUS_BYTES_PER_SECOND : MOCK_WAV_BYTES_PER_SECOND;
    const recording: KeptRecording = {
      id,
      at_ms: this.now(),
      duration_ms: durationMs,
      bytes: Math.max(44, Math.round((durationMs / 1000) * perSecond)),
      ...(long ? { format: "opus" as const } : {}),
      ...(app === undefined ? {} : { app }),
      status: { kind: "pending" },
    };
    this.setRecordings(this.withinLimits([recording, ...this.state.audio_cache.recordings]));
  }

  /** `Effect::Kept`: what became of the take's recording. A failure never replaces an entry;
   *  an entry the history did not keep takes the recording away. */
  private settleRecording(status: KeptStatus) {
    const id = this.takeRecording;
    if (id === undefined) return;
    const recordings = this.state.audio_cache.recordings;
    if (status.kind === "recognized" && !this.history.some((e) => e.id === status.entry)) {
      this.setRecordings(recordings.filter((r) => r.id !== id));
      return;
    }
    this.setRecordings(
      recordings.map((r) =>
        r.id !== id || (r.status.kind === "recognized" && status.kind !== "recognized")
          ? r
          : { ...r, status },
      ),
    );
  }

  /** The recordings whose entry the history no longer has go too. */
  private followHistory() {
    const ids = new Set(this.history.map((e) => e.id));
    this.setRecordings(
      this.state.audio_cache.recordings.filter(
        (r) => r.status.kind !== "recognized" || ids.has(r.status.entry),
      ),
    );
  }

  /** `audio_retake`: a take without a capture that starts in processing; its text goes to the
   *  clipboard and into the history (a new entry the recording then points at). `models` are
   *  other models than the settings' for this take only (§26.4); one that cannot run refuses it. */
  private retake(id: string, models: RetakeModels = {}) {
    const recording = this.state.audio_cache.recordings.find((r) => r.id === id);
    if (recording === undefined) {
      this.emit({ type: "error", message: MOCK_RECORDING_GONE });
      return;
    }
    const current = this.state.dictation.phase.phase;
    if (current === "listening" || current === "processing") {
      this.emit({ type: "error", message: MOCK_DICTATION_BUSY });
      return;
    }
    const chosen = models.asr !== undefined || models.refine !== undefined;
    const engines = chosen
      ? this.resolveEngines(applyRetakeModels(this.state.settings.engines, models))
      : this.state.engines;
    if (
      chosen &&
      (!engines.asr_ready || (models.refine?.kind === "remote" && !engines.refine_ready))
    ) {
      this.emit({ type: "error", message: MOCK_RETAKE_NOT_READY });
      return;
    }
    this.clearDictationTimers();
    this.clearLiveTimers();
    this.clearExtraStop();
    this.pinned = undefined;
    const session = this.state.dictation.session + 1;
    this.takeKind = "dictation";
    this.takeMode = "whole_take";
    this.injectedChars = 0;
    this.takeModeError = undefined;
    this.takePresetId = this.state.settings.engines.refine_preset;
    this.takePresets = this.state.presets;
    this.takePreset = undefined;
    this.takeSource = undefined;
    this.takeSegments = undefined;
    this.takeRemote = undefined;
    this.takeRecording = id;
    this.takeRetake = true;
    const scene =
      recording.app === undefined ? undefined : matchScene(this.state.scenes, recording.app);
    this.takeScene = scene;
    this.takeContext =
      recording.app === undefined
        ? undefined
        : { app: recording.app, ...(scene === undefined ? {} : { scene: sceneRefOf(scene) }) };
    // A chosen clean-up (or none) wins over the scene's, as a chosen model does (§26.4).
    const refine =
      models.refine === undefined
        ? (scene?.overrides.refine_enabled ?? engines.refine_enabled)
        : models.refine.kind === "remote";
    const refineFailure = refine ? this.refineFailure : undefined;
    const polished = refine && refineFailure === undefined;
    const startedAt = this.now();
    this.emitPhase(
      {
        phase: "processing",
        stage: "transcribing",
        started_at: startedAt,
        stage_started_at: startedAt,
      },
      session,
    );
    const finish = () => {
      if (this.state.dictation.session !== session) return;
      const vocabulary = Vocabulary.compile(this.state.dictionary, this.state.rules);
      const corrected = vocabulary.correct(MOCK_DICTATION_RAW);
      const refined = polished ? vocabulary.correct(MOCK_DICTATION_TEXT).text : corrected.text;
      const ruled = vocabulary.applyRules(refined);
      const text = ruled.text;
      const preset = polished ? this.resolveTakePreset() : undefined;
      const hits = { corrections: corrected.hits, rules: ruled.hits };
      const entry: HistoryEntry = {
        id: this.uuid(),
        at_ms: this.now(),
        raw_text: MOCK_DICTATION_RAW,
        text,
        refined: polished,
        asr_model: engines.asr_model,
        ...(polished ? { refine_model: engines.refine_model, refine_ms: MOCK_REFINE_MS } : {}),
        ...(refineFailure === undefined ? {} : { refine_failure: refineFailure }),
        ...(refineFailure === "rate_limited" ||
        refineFailure === "quota" ||
        refineFailure === "failed"
          ? { refine_error: MOCK_REFINE_FAILED }
          : {}),
        duration_ms: recording.duration_ms,
        asr_ms: MOCK_ASR_MS,
        outcome: { kind: "inserted", via: "clipboard" },
        starred: false,
        mode: "whole_take",
        ...(hits.corrections.length + hits.rules.length > 0 ? { vocabulary: hits } : {}),
        ...(this.takeContext === undefined ? {} : { app: this.takeContext.app }),
        ...(scene === undefined ? {} : { scene: this.takeContext?.scene ?? sceneRefOf(scene) }),
        ...(preset === undefined ? {} : { preset }),
        kind: "dictation",
      };
      this.recordHistory(entry);
      this.settleRecording({ kind: "recognized", entry: entry.id });
      this.emitPhase({
        phase: "done",
        text,
        raw_text: MOCK_DICTATION_RAW,
        chars: Array.from(text).length,
        via: "clipboard",
        refined: polished,
        duration_ms: recording.duration_ms,
        asr_ms: MOCK_ASR_MS,
        mode: "whole_take",
        ...(polished ? { refine_ms: MOCK_REFINE_MS } : {}),
        ...(refineFailure === undefined
          ? {}
          : { refine_failure: refineFailure, refine_error: MOCK_REFINE_FAILED }),
      });
      this.dwell(MOCK_DICTATION_DWELL_MS, session);
    };
    this.laterDictation(MOCK_ASR_MS, () => {
      if (this.state.dictation.session !== session) return;
      if (!refine) {
        finish();
        return;
      }
      this.takePreset = this.resolveTakePreset();
      this.emitPhase({
        phase: "processing",
        stage: "refining",
        started_at: startedAt,
        stage_started_at: this.now(),
      });
      this.laterDictation(MOCK_REFINE_MS, finish);
    });
  }

  // ---- phone as microphone (docs/dictation.md §20) ---------------------------------------------

  /** Desktop: a paired phone starts streaming a take (what the core does on `TakeStart`). */
  simulatePhoneTake(name = phonePeer().name) {
    this.startDictation("dictation");
    this.takeRemote = name;
    this.takeSource = undefined;
    this.emitPhase(this.state.dictation.phase);
  }

  /** A long take's recognition (docs/dictation.md §22): `done` of `total` segments, as the core
   *  reports it once the take is past its first two minutes. Only while a take runs. */
  simulateLongTakeProgress(done: number, total: number) {
    const phase = this.state.dictation.phase;
    if (phase.phase !== "listening" && phase.phase !== "processing") return;
    this.takeSegments = { done, total };
    this.emitPhase(phase);
  }

  /** Desktop: the phone let go (`TakeStop`). */
  simulatePhoneTakeStop() {
    this.stopDictation();
  }

  private emitPhoneTake(state: PhoneTakeState, opus?: boolean) {
    const current = this.state.phone_take;
    if (current === undefined) return;
    this.emit({
      type: "phone_take",
      take: { ...current, state, ...(opus === undefined ? {} : { opus }) },
    });
  }

  /** What the desktop shell reports after (re)registering both chords: the dictation chord, and the
   *  edit chord while it is on; nothing while the recorder holds the keyboard. */
  private hotkeyStatus(capturing: boolean): HotkeyStatus {
    const { hotkey, edit_hotkey, solo_key, copy_last_hotkey } = this.state.settings;
    return {
      ...(capturing ? {} : { registered: hotkey }),
      ...(capturing || edit_hotkey === null ? {} : { edit_registered: edit_hotkey }),
      ...(capturing || copy_last_hotkey == null ? {} : { copy_last_registered: copy_last_hotkey }),
      ...(capturing ? {} : mockSoloStatus(solo_key)),
      pressed: false,
      capturing,
      backend: MOCK_HOTKEY_BACKEND,
      capabilities: { ...MOCK_HOTKEY_CAPABILITIES },
      solo_pressed: false,
    };
  }

  /** The probe of a relay that is not connected: how its last attempt failed, the build's own
   *  relay without the words (as the core reports it). */
  private mockProbeFailure(): ProbeResult {
    const failure = this.state.relay.last_failure;
    if (failure === undefined || failure.kind === "timeout") return { result: "timeout" };
    if (failure.kind === "refused") return { result: "refused" };
    const reason = this.state.relay.source === "user" ? (failure.detail ?? "") : "";
    return { result: "failed", reason, kind: failure.kind };
  }

  /** A self-check against the mock's own state: the relay answers when its link is up, fails
   *  the way its last attempt did when it is not (`simulateRelay` with a `last_failure`), and an
   *  online device has a round trip. */
  private connectivityReport(): ConnectivityReport {
    const relayUp = this.state.relay.state === "connected";
    return {
      checked_at: this.now(),
      relay: {
        configured: this.state.relay.source !== "none",
        ...(this.state.relay.source === "none"
          ? {}
          : {
              result: relayUp ? { result: "ok" as const, ms: 48 } : this.mockProbeFailure(),
            }),
      },
      peers: this.state.devices.map((d) => {
        const online = d.connection.state === "online";
        return {
          public_key: d.device.public_key,
          name: d.device.name,
          online,
          ...(online ? { rtt_ms: 61 } : {}),
        };
      }),
    };
  }

  /** The text the foreground application has selected, as the next voice edit's copy will find
   *  it (docs/dictation.md §19); `null` (the default) = nothing selected. */
  setSelection(text: string | null) {
    this.selection = text;
  }

  /** A new session interrupts any dwell and starts listening at once — `ready: false` until the
   *  simulated device delivers its first samples (`MOCK_MIC_READY_MS`), then `ready: true` with a
   *  fresh `started_at`; with `live_preview_ready` the streaming partials follow (§11). */
  private startDictation(kind: TakeKind) {
    this.clearDictationTimers();
    this.pinned = undefined;
    this.clearLiveTimers();
    this.clearExtraStop();
    const session = this.state.dictation.session + 1;
    this.takeKind = kind;
    this.copiedSelection = undefined;
    // Like the core, the output mode is decided when the take starts (§12) — by the scene the
    // foreground app matches, if any (§18.4): its overrides apply to this take only. A voice edit
    // (§19) is always one whole take and matches no scene; it keeps the app as its context.
    this.takeMode = kind === "edit" ? "whole_take" : this.state.engines.effective_output_mode;
    this.injectedChars = 0;
    this.takeContext = undefined;
    this.takeScene = undefined;
    this.takeModeError = undefined;
    // §21: the preset of the start; switching or editing one mid-take applies to the next take.
    this.takePresetId = this.state.settings.engines.refine_preset;
    this.takePresets = this.state.presets;
    this.takePreset = undefined;
    // §22: a voice edit's instruction is spoken into the microphone.
    this.takeSource = kind === "edit" ? "microphone" : this.state.settings.recording.source;
    this.takeSegments = undefined;
    this.takeRecording = undefined;
    this.takeRetake = false;
    if (kind === "edit" && !this.state.engines.refine_ready) {
      // §19.4: no LLM, no edit — refused at the press, the microphone never opens.
      this.emitPhase(
        {
          phase: "failed",
          message: MOCK_EDIT_UNAVAILABLE,
          code: "edit_unavailable",
        },
        session,
      );
      this.dwell(MOCK_DICTATION_DWELL_MS, session);
      return;
    }
    // The core keeps the probe's answer sanitised (§18.2): a normalised id, one-line names.
    const app = this.foregroundApp === null ? undefined : sanitizeForegroundApp(this.foregroundApp);
    const host = hostOsOf(this.state.identity?.platform ?? "other");
    if (kind === "edit" && app !== undefined && isTerminalApp(host, app.id)) {
      // §19.2: the copy chord is a terminal's interrupt; refused once the probe answered — no
      // copy, no microphone, nothing recorded.
      this.emitPhase(
        {
          phase: "listening",
          started_at: this.now(),
          ready: false,
          locked: false,
        },
        session,
      );
      this.emitPhase({
        phase: "failed",
        message: MOCK_EDIT_IN_TERMINAL,
        code: "edit_in_terminal",
      });
      this.dwell(MOCK_DICTATION_DWELL_MS, session);
      return;
    }
    // The phone has no probe: its takes run with the scene the user picked
    // (`settings.pinned_scene`) while the list still has it, like the core (user decision
    // 2026-10-01).
    const scene =
      kind === "edit"
        ? undefined
        : app !== undefined
          ? matchScene(this.state.scenes, app)
          : this.role === "phone"
            ? this.state.scenes.find((s) => s.id === this.state.settings.pinned_scene)
            : undefined;
    this.takeScene = scene;
    if (app !== undefined)
      this.takeContext = {
        app: { id: app.id, name: app.name },
        ...(scene === undefined ? {} : { scene: sceneRefOf(scene) }),
      };
    const mode = scene?.overrides.output_mode;
    if (mode != null) {
      const serviceable = mode === "whole_take" || this.state.engines.live_preview_ready;
      this.takeMode = serviceable ? mode : "whole_take";
      if (!serviceable) this.takeModeError = MOCK_SCENE_MODE_NOT_READY;
    }
    this.emitPhase(
      {
        phase: "listening",
        started_at: this.now(),
        ready: false,
        locked: false,
      },
      session,
    );
    if (kind === "edit") this.copySelection(session);
    this.laterLive(MOCK_MIC_READY_MS, () => {
      if (!this.listeningIn(session)) return;
      const current = this.state.dictation.phase;
      const locked = current.phase === "listening" && current.locked;
      this.emitPhase({
        phase: "listening",
        started_at: this.now(),
        ready: true,
        locked,
      });
      if (!this.state.engines.live_preview_ready) return;
      const injecting = this.takeMode === "live_inject";
      const step = (n: number) => {
        const live = MOCK_LIVE_SCRIPT[n];
        const phase = this.state.dictation.phase;
        if (live === undefined || !this.listeningIn(session) || phase.phase !== "listening") return;
        // A degraded preview keeps what it showed and ignores later partials, like the core.
        if (phase.live?.degraded !== undefined) return;
        // `live_inject` pastes every committed sentence the moment it closes (§12): the mock's
        // paste is instant, so `injected` follows `committed` and the cancel count follows both.
        const injected = injecting ? live.committed.length : 0;
        if (injecting) this.injectedChars = injectedChars(live.committed, injected);
        this.emitPhase({
          ...phase,
          live: { ...structuredClone(live), injected },
        });
        this.laterLive(MOCK_LIVE_STEP_MS, () => {
          step(n + 1);
        });
      };
      this.laterLive(MOCK_LIVE_STEP_MS, () => {
        step(0);
      });
    });
  }

  /** The edit take's copy at the press (§19.2): nothing (or only blanks) selected and a selection
   *  over the limit end the take at once, before anything is uploaded. */
  private copySelection(session: number) {
    const selection = this.selection;
    this.laterDictation(MOCK_COPY_MS, () => {
      if (!this.listeningIn(session)) return;
      const refuse = (code: DictationFailureCode, message: string) => {
        this.clearLiveTimers();
        this.clearExtraStop();
        this.emitPhase({ phase: "failed", message, code });
        this.dwell(MOCK_DICTATION_DWELL_MS, session);
      };
      if (selection === null || selection.trim().length === 0) {
        refuse("no_selection", MOCK_NO_SELECTION);
        return;
      }
      const chars = Array.from(selection).length;
      if (chars > MAX_EDIT_SELECTION_CHARS) {
        refuse(
          "selection_too_long",
          `选中文本过长：${chars} 字（上限 ${MAX_EDIT_SELECTION_CHARS} 字）`,
        );
        return;
      }
      this.copiedSelection = selection;
    });
  }

  /** Whether `session` is still the one listening (a stale timer must not touch a later take). */
  private listeningIn(session: number): boolean {
    return (
      this.state.dictation.session === session && this.state.dictation.phase.phase === "listening"
    );
  }

  /** The core's test hook (docs/dictation.md §11 `StreamDegraded`): the streaming path failed
   *  (model load, tap overrun, decoder error). The text shown so far stays, later partials are
   *  ignored, and the recording continues to the whole-take result untouched. */
  simulateLiveDegraded(reason: string) {
    const phase = this.state.dictation.phase;
    if (phase.phase !== "listening") return;
    this.clearLiveTimers();
    const live: LiveText = {
      committed: [],
      current: "",
      injected: 0,
      ...phase.live,
      degraded: reason,
    };
    this.emitPhase({ ...phase, live });
  }

  /** One hotkey edge into the activation machine (docs/dictation.md §13), simplified to what the
   *  UI can observe: `hold` starts on press and stops on release; `toggle` flips on press and
   *  ignores release; `hold_or_toggle` stops on a release held at least `hold_threshold_ms`,
   *  locks the take on a shorter one (`listening.locked`) and stops on the next press. CLI edges
   *  are exempt from the mode: press toggles, release cancels. Debounce, the release grace and
   *  the pending press while processing are core details the mock does not simulate. */
  private hotkeyEdge(edge: HotkeyEdgeArgs) {
    const at = edge.atMs ?? this.now();
    const phase = this.state.dictation.phase;
    const listening = phase.phase === "listening";
    // docs/dictation.md §19: one take at a time — while a take of the other kind runs, this key's
    // edges are dropped (no stop, no pending start).
    const purpose = edge.purpose ?? "dictation";
    const busy = listening || phase.phase === "processing";
    if (busy && this.state.dictation.kind !== purpose) return;
    if (edge.source === "cli") {
      if (!edge.pressed) this.cancelDictation();
      else if (listening) this.stopDictation();
      else if (phase.phase !== "processing") this.startDictation(purpose);
      return;
    }
    const { activation, hold_threshold_ms } = this.state.settings;
    if (edge.pressed) {
      if (phase.phase === "listening") {
        // The second press ends a toggle or a locked take; under hold it is auto-repeat noise.
        if (activation === "toggle" || (activation === "hold_or_toggle" && phase.locked))
          this.stopDictation();
        return;
      }
      if (phase.phase === "processing") return;
      this.pressedAt = at;
      this.startDictation(purpose);
      return;
    }
    if (phase.phase !== "listening" || activation === "toggle") return;
    if (activation === "hold_or_toggle") {
      if (phase.locked) return;
      const held = at - (this.pressedAt ?? at);
      if (held < hold_threshold_ms) {
        this.emitPhase({ ...phase, locked: true });
        return;
      }
    }
    this.stopDictation();
  }

  /** Stop the take. With `extra_recording_ms > 0` (§13) the microphone stays open that much
   *  longer — a second stop closes it at once, a cancel in the window drops the recording. */
  private stopDictation() {
    if (this.state.dictation.phase.phase !== "listening") return;
    const extra = this.state.settings.extra_recording_ms;
    if (this.extraStop !== undefined) {
      this.clearExtraStop();
      this.finishTake();
      return;
    }
    if (extra <= 0) {
      this.finishTake();
      return;
    }
    this.extraStop = setTimeout(() => {
      this.extraStop = undefined;
      this.finishTake();
    }, extra);
  }

  private clearExtraStop() {
    if (this.extraStop !== undefined) clearTimeout(this.extraStop);
    this.extraStop = undefined;
  }

  /** Close the recorder and run the pipeline the take's output mode asks for (§12): the whole take
   *  goes transcribing → refining? → done; `streaming_final` goes finalizing → refining? → done with
   *  the committed sentences plus the tail as its text; `live_inject` goes finalizing → inserting →
   *  done without refinement. A streaming take that has no text, or whose preview degraded, falls
   *  back to the whole take with the reason in `live_error`. */
  private finishTake() {
    const current = this.state.dictation.phase;
    if (current.phase !== "listening") return;
    if (this.takeKind === "edit") {
      this.finishEdit(current.started_at);
      return;
    }
    this.clearLiveTimers();
    const startedAt = current.started_at;
    const session = this.state.dictation.session;
    const engines = this.state.engines;
    const stoppedAt = this.now();
    // The preview follows the take into processing until the final text replaces it (§11).
    const live = current.live;
    const preview = live === undefined ? "" : livePreviewText(live);
    const carried = preview.length > 0 ? { preview } : {};
    const requested = this.takeMode;
    const liveError =
      requested === "whole_take"
        ? undefined
        : preview.length === 0
          ? MOCK_EMPTY_STREAM_ERROR
          : live?.degraded;
    const mode: OutputMode = liveError === undefined ? requested : "whole_take";
    const streaming = mode !== "whole_take";
    const scene = this.takeScene;
    const context = this.takeContext;
    const modeError = this.takeModeError;
    const refine =
      (scene?.overrides.refine_enabled ?? engines.refine_enabled) && mode !== "live_inject";
    // docs/dictation.md §3.6: a clean-up that ran and was turned down keeps the raw text.
    const refineFailure = refine ? this.refineFailure : undefined;
    const polished = refine && refineFailure === undefined;
    const asrMs = streaming ? MOCK_FINALIZE_MS : MOCK_ASR_MS;
    const raw = streaming ? preview : MOCK_DICTATION_RAW;
    const durationMs = Math.max(0, stoppedAt - startedAt);
    const segments = streaming && live !== undefined ? streamSegments(live, durationMs) : undefined;
    // The status names the clean-up's preset from the start of processing (§21).
    const preset = refine ? this.resolveTakePreset() : undefined;
    this.takePreset = preset;
    // docs/dictation.md §26: the recording is kept before it is recognised.
    this.keepTakeRecording(durationMs);
    this.emitPhase({
      phase: "processing",
      stage: streaming ? "finalizing" : "transcribing",
      started_at: stoppedAt,
      stage_started_at: stoppedAt,
      ...carried,
    });
    const finish = () => {
      if (this.state.dictation.session !== session) return;
      // The phone's result goes to its clipboard (docs/dictation.md §20.7), whatever the setting;
      // another window in front copies instead of pasting (§14.3).
      const switched =
        this.role !== "phone" &&
        engines.inject === "paste" &&
        this.windowSwitched &&
        this.state.settings.engines.switched_window === "copy_only";
      const via =
        this.role === "phone" || engines.inject === "clipboard_only" || switched
          ? "clipboard"
          : "paste";
      // docs/dictation.md §16.3: the dictionary corrects the transcript, the (canned) LLM keeps the
      // glossary terms, the rules run last; an emptied text is no speech, nothing is inserted.
      const vocabulary = Vocabulary.compile(this.state.dictionary, this.state.rules);
      const corrected = vocabulary.correct(raw);
      const refined = polished ? vocabulary.correct(MOCK_DICTATION_TEXT).text : corrected.text;
      const ruled = vocabulary.applyRules(refined);
      const text = ruled.text;
      if (text.trim().length === 0) {
        this.emitPhase({
          phase: "failed",
          message: MOCK_NO_SPEECH,
          code: "no_speech",
        });
        this.settleRecording({ kind: "failed", code: "no_speech", message: MOCK_NO_SPEECH });
        this.dwell(MOCK_DICTATION_DWELL_MS, session);
        return;
      }
      const hits = { corrections: corrected.hits, rules: ruled.hits };
      // §28: only a paste that was delivered is checked, never a copy or `live_inject`.
      const check =
        via === "paste" && mode !== "live_inject" && this.state.settings.engines.insert_check
          ? this.insertCheck
          : undefined;
      const checked = check === undefined ? {} : { check };
      const failedOver = liveError ?? modeError;
      const extra = {
        ...(polished ? { refine_ms: MOCK_REFINE_MS } : {}),
        ...(segments === undefined ? {} : { segments }),
        ...(failedOver === undefined ? {} : { live_error: failedOver }),
      };
      const done: DictationPhase = {
        phase: "done",
        text,
        raw_text: raw,
        chars: Array.from(text).length,
        via,
        refined: polished,
        duration_ms: durationMs,
        asr_ms: asrMs,
        mode,
        ...extra,
        ...(refineFailure === undefined
          ? {}
          : { refine_failure: refineFailure, refine_error: MOCK_REFINE_FAILED }),
        ...(switched ? { clipboard_code: "target_changed" as const } : {}),
        ...checked,
      };
      const entry: HistoryEntry = {
        id: this.uuid(),
        at_ms: this.now(),
        raw_text: raw,
        text,
        refined: polished,
        asr_model: engines.asr_model,
        ...(polished ? { refine_model: engines.refine_model } : {}),
        ...(refineFailure === undefined ? {} : { refine_failure: refineFailure }),
        // As the core: a failed request keeps the service's words (§3.6).
        ...(refineFailure === "rate_limited" ||
        refineFailure === "quota" ||
        refineFailure === "failed"
          ? { refine_error: MOCK_REFINE_FAILED }
          : {}),
        duration_ms: durationMs,
        asr_ms: asrMs,
        outcome: switched
          ? { kind: "clipboard", reason: MOCK_WINDOW_CHANGED, code: "target_changed" }
          : { kind: "inserted", via, ...checked },
        starred: false,
        mode,
        ...extra,
        ...(hits.corrections.length + hits.rules.length > 0 ? { vocabulary: hits } : {}),
        ...(context === undefined ? {} : { app: context.app }),
        ...(scene === undefined ? {} : { scene: context?.scene ?? sceneRefOf(scene) }),
        ...(preset === undefined || !polished ? {} : { preset }),
        kind: "dictation",
      };
      this.recordHistory(entry);
      this.settleRecording({ kind: "recognized", entry: entry.id });
      if (this.role === "phone") this.phoneClipboard = text;
      this.emitPhase(done);
      const card = switched || check === "not_written";
      this.dwellOrPin(
        card ? MOCK_DICTATION_FAILED_DWELL_MS : MOCK_DICTATION_DWELL_MS,
        session,
        card,
      );
    };
    this.laterDictation(asrMs, () => {
      if (this.state.dictation.session !== session) return;
      if (mode === "live_inject") {
        // The tail is the last paste; nothing is refined (§12).
        this.emitPhase({
          phase: "processing",
          stage: "inserting",
          started_at: stoppedAt,
          stage_started_at: this.now(),
        });
        this.laterDictation(MOCK_FINALIZE_MS, finish);
        return;
      }
      if (!refine) {
        finish();
        return;
      }
      this.emitPhase({
        phase: "processing",
        stage: "refining",
        started_at: stoppedAt,
        stage_started_at: this.now(),
        ...carried,
      });
      this.laterDictation(MOCK_REFINE_MS, finish);
    });
  }

  /** A voice edit's pipeline (docs/dictation.md §19.3): transcribing (the instruction, dictionary
   *  corrections) → refining (`Refiner::edit`: the canned rewrite) → inserting → done, and a
   *  `kind: edit` history row with the instruction and the original selection. The replacement
   *  rules never run on the rewrite; an instruction corrected away is no speech. */
  private finishEdit(startedAt: number) {
    this.clearLiveTimers();
    const session = this.state.dictation.session;
    const engines = this.state.engines;
    const context = this.takeContext;
    const stoppedAt = this.now();
    const durationMs = Math.max(0, stoppedAt - startedAt);
    this.emitPhase({
      phase: "processing",
      stage: "transcribing",
      started_at: stoppedAt,
      stage_started_at: stoppedAt,
    });
    this.laterDictation(MOCK_ASR_MS, () => {
      if (this.state.dictation.session !== session) return;
      const selection = this.copiedSelection;
      if (selection === undefined) return; // refused by the copy
      const vocabulary = Vocabulary.compile(this.state.dictionary, this.state.rules);
      const instruction = vocabulary.correct(MOCK_EDIT_INSTRUCTION);
      if (instruction.text.trim().length === 0) {
        this.emitPhase({
          phase: "failed",
          message: MOCK_NO_SPEECH,
          code: "no_speech",
        });
        this.dwell(MOCK_DICTATION_DWELL_MS, session);
        return;
      }
      this.emitPhase({
        phase: "processing",
        stage: "refining",
        started_at: stoppedAt,
        stage_started_at: this.now(),
      });
      this.laterDictation(MOCK_REFINE_MS, () => {
        if (this.state.dictation.session !== session) return;
        this.emitPhase({
          phase: "processing",
          stage: "inserting",
          started_at: stoppedAt,
          stage_started_at: this.now(),
        });
        this.laterDictation(MOCK_FINALIZE_MS, () => {
          if (this.state.dictation.session !== session) return;
          const via = engines.inject === "clipboard_only" ? "clipboard" : "paste";
          const text = MOCK_EDIT_TEXT;
          // §28: a rewrite equal to the selection cannot be told from no paste; it is not checked.
          const check =
            via === "paste" && this.state.settings.engines.insert_check && text !== selection
              ? this.insertCheck
              : undefined;
          const checked = check === undefined ? {} : { check };
          const entry: HistoryEntry = {
            id: this.uuid(),
            at_ms: this.now(),
            raw_text: MOCK_EDIT_INSTRUCTION,
            text,
            refined: true,
            asr_model: engines.asr_model,
            refine_model: engines.refine_model,
            duration_ms: durationMs,
            asr_ms: MOCK_ASR_MS,
            refine_ms: MOCK_REFINE_MS,
            outcome: { kind: "inserted", via, ...checked },
            starred: false,
            mode: "whole_take",
            ...(instruction.hits.length > 0
              ? { vocabulary: { corrections: instruction.hits, rules: [] } }
              : {}),
            kind: "edit",
            edit: { instruction: instruction.text, selection },
            // The app it ran in, never a scene (§19 with §18.6).
            ...(context === undefined ? {} : { app: context.app }),
          };
          this.recordHistory(entry);
          this.emitPhase({
            phase: "done",
            text,
            raw_text: MOCK_EDIT_INSTRUCTION,
            chars: Array.from(text).length,
            via,
            refined: true,
            duration_ms: durationMs,
            asr_ms: MOCK_ASR_MS,
            refine_ms: MOCK_REFINE_MS,
            mode: "whole_take",
            ...checked,
          });
          const card = check === "not_written";
          this.dwellOrPin(
            card ? MOCK_DICTATION_FAILED_DWELL_MS : MOCK_DICTATION_DWELL_MS,
            session,
            card,
          );
        });
      });
    });
  }

  private cancelDictation() {
    const current = this.state.dictation.phase.phase;
    if (current !== "listening" && current !== "processing") return;
    this.clearDictationTimers();
    this.clearLiveTimers();
    this.clearExtraStop();
    // §26.3: a dictation take cancelled while it records keeps its copy once it is long.
    const phase = this.state.dictation.phase;
    const listenedMs = phase.phase === "listening" ? this.now() - phase.started_at : 0;
    if (this.takeKind === "dictation" && listenedMs > MOCK_IN_MEMORY_MS) {
      this.keepTakeRecording(listenedMs);
    }
    // `live_inject` does not take back what it pasted (§12); every other mode reports 0.
    this.emitPhase({ phase: "cancelled", injected_chars: this.injectedChars });
    // §26: a recording kept for the take reads cancelled.
    this.settleRecording({ kind: "cancelled" });
    this.dwell(MOCK_DICTATION_DWELL_MS, this.state.dictation.session);
  }

  /** The core's test hook: a failed run (ASR error, or injection failure with the text kept);
   *  `code` is the stage that stopped it, as the core reports it. A kept recording reads failed
   *  when no text came (§26). */
  simulateDictationFailed(message: string, text?: string, code?: DictationFailureCode) {
    this.clearDictationTimers();
    this.clearLiveTimers();
    const session = this.state.dictation.session;
    const coded = code === undefined ? {} : { code };
    this.emitPhase(
      text === undefined
        ? { phase: "failed", message, ...coded }
        : { phase: "failed", message, text, ...coded },
    );
    if (text === undefined)
      this.settleRecording({ kind: "failed", code: code ?? "unknown", message });
    // §28: a failure with text, or without text but with its recording kept, is a card.
    const card = text !== undefined || (this.takeRecording !== undefined && code !== "no_speech");
    this.dwellOrPin(
      text === undefined ? MOCK_DICTATION_DWELL_MS : MOCK_DICTATION_FAILED_DWELL_MS,
      session,
      card,
    );
  }

  /** What the next paste take's insert check finds (docs/dictation.md §28); `undefined` = none runs. */
  setInsertCheck(check: InsertCheck | undefined): void {
    this.insertCheck = check;
  }

  /** A terminal state's dwell, or none for a result card the shell shows while 识别结果卡片常驻 is
   *  on (docs/dictation.md §28): it stays until `dictation_dismiss` or the next take. */
  private dwellOrPin(ms: number, session: number, card: boolean) {
    if (card && this.resultCards && this.state.settings.engines.result_cards_stay) {
      this.pinned = { ms, session };
      return;
    }
    this.pinned = undefined;
    this.dwell(ms, session);
  }

  private dwell(ms: number, session: number) {
    this.laterDictation(ms, () => {
      if (this.state.dictation.session === session) this.emitPhase({ phase: "idle" });
    });
  }

  private laterDictation(ms: number, fn: () => void) {
    const handle = setTimeout(() => {
      this.dictationTimers.delete(handle);
      fn();
    }, ms);
    this.dictationTimers.add(handle);
  }

  private clearDictationTimers() {
    for (const t of this.dictationTimers) clearTimeout(t);
    this.dictationTimers.clear();
  }

  private laterLive(ms: number, fn: () => void) {
    const handle = setTimeout(() => {
      this.liveTimers.delete(handle);
      fn();
    }, ms);
    this.liveTimers.add(handle);
  }

  private clearLiveTimers() {
    for (const t of this.liveTimers) clearTimeout(t);
    this.liveTimers.clear();
  }

  /** RFC 4122-shaped id from the seeded generator, so history ids are stable in tests. */
  private uuid(): string {
    const hex = hexFromRandom(this.random, 16);
    return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-4${hex.slice(13, 16)}-8${hex.slice(17, 20)}-${hex.slice(20, 32)}`;
  }

  // ---- internals ---------------------------------------------------------------------------------

  /** Below the relay's minimum version the core refuses any pairing (docs/pairing.md 「最低版本」). */
  private refuseTooOld(): boolean {
    if (relayTooOld(this.state.relay) === undefined) return false;
    this.emit({ type: "error", message: PAIRING_TOO_OLD });
    return true;
  }

  private startPairing() {
    this.clearTimers();
    this.emitPairing({
      ...idleSnapshot(),
      state: { state: "creating_session" },
    });
    this.later(CREATE_SESSION_MS, () => {
      const sessionId = hexFromRandom(this.random, 8);
      const ticket = hexFromRandom(this.random, 16);
      const code = this.sixDigits();
      const expiresAt = Math.floor(this.now() / 1000) + this.ttlSecs;
      this.emitPairing({
        state: { state: "waiting_for_peer" },
        session_id: sessionId,
        code,
        ticket_uri: `voltip://pair?v=1&s=${sessionId}&t=${ticket}`,
        expires_at: expiresAt,
        remaining_secs: this.ttlSecs,
      });
      this.startCountdown();
      if (this.autoPeer) {
        const peer = this.autoPeer;
        this.later(peer.joinAfterMs, () => {
          this.simulatePeerJoined(peer.info, peer.scanned ?? false);
        });
      }
    });
  }

  private joinWithCode(code: string) {
    const digits = code.replace(/\s+/g, "");
    if (!/^\d{6}$/.test(digits)) {
      this.finish({
        state: "failed",
        reason: { kind: "relay", code: "invalid_code" },
      });
      return;
    }
    if (this.expectedCode !== undefined && this.expectedCode.replace(/\s+/g, "") !== digits) {
      this.finish({
        state: "failed",
        reason: { kind: "relay", code: "invalid_code" },
      });
      return;
    }
    // The relay hands out the six digits, so they prove nothing: both users confirm.
    this.joinSession(false);
  }

  private joinWithTicket(uri: string) {
    let parsed: URL | undefined;
    try {
      parsed = new URL(uri);
    } catch (_error) {
      parsed = undefined;
    }
    if (parsed?.protocol !== "voltip:" || parsed.searchParams.get("t") === null) {
      this.finish({ state: "failed", reason: { kind: "protocol" } });
      return;
    }
    this.joinSession(true);
  }

  private joinSession(scanned: boolean) {
    this.clearTimers();
    this.emitPairing({
      ...idleSnapshot(),
      state: { state: "creating_session" },
    });
    this.later(CREATE_SESSION_MS, () => {
      this.simulatePeerJoined(undefined, scanned);
      if (this.autoPeer) {
        const peer = this.autoPeer;
        this.later(KEY_EXCHANGE_MS + peer.confirmAfterMs, () => {
          this.simulatePeerConfirmed();
        });
      }
    });
  }

  private confirmLocal() {
    if (this.state.pairing.state.state !== "awaiting_verification") return;
    this.emitPairing({ local_confirmed: true });
    if (this.state.pairing.peer_confirmed) this.becomeTrusted();
  }

  private becomeTrusted() {
    this.clearTimers();
    const peer = this.state.pairing.peer ?? phonePeer();
    const publicKey =
      peer.platform === "android" || peer.platform === "ios"
        ? MOCK_PUBLIC_KEYS.phone
        : MOCK_PUBLIC_KEYS.desktop;
    const trustedAt = Math.floor(this.now() / 1000);
    const device: TrustedDevice = {
      device_id: peer.device_id,
      name: peer.name,
      platform: peer.platform,
      public_key: publicKey,
      fingerprint: this.state.pairing.safety_code?.fingerprint ?? fingerprintOf(this.random),
      trusted_at: trustedAt,
      last_seen: trustedAt,
      sync: true,
      sync_gen: 0,
    };
    this.emitPairing({ state: { state: "trusted" } });
    this.openNextIfAlwaysOn();
    this.emit({ type: "trusted", ...device });
    const others = this.state.devices.filter((d) => d.device.public_key !== publicKey);
    this.emit({
      type: "devices",
      devices: [...others, { device, connection: { state: "online" } }],
    });
  }

  private finish(state: Snapshot["state"]) {
    this.clearTimers();
    this.emitPairing({
      state,
      remaining_secs: state.state === "expired" ? 0 : undefined,
    });
    this.openNextIfAlwaysOn();
  }

  private alwaysOn(): boolean {
    return this.role !== "phone" && this.state.settings.pairing_always_on;
  }

  /** Always-on pairing: the next session after the outcome has been on screen for a moment. */
  private openNextIfAlwaysOn() {
    if (!this.alwaysOn()) return;
    this.later(MOCK_ALWAYS_ON_PAUSE_MS, () => {
      if (this.alwaysOn()) this.startPairing();
    });
  }

  private startCountdown() {
    this.stopCountdown();
    this.countdown = setInterval(() => {
      const remaining = Math.max(0, (this.state.pairing.remaining_secs ?? 0) - 1);
      if (this.alwaysOn() && remaining <= MOCK_ALWAYS_ON_RENEW_SECS) {
        this.startPairing();
        return;
      }
      if (remaining === 0) {
        this.finish({ state: "expired" });
        return;
      }
      this.emitPairing({ remaining_secs: remaining });
    }, 1000);
  }

  private stopCountdown() {
    if (this.countdown !== undefined) clearInterval(this.countdown);
    this.countdown = undefined;
  }

  private clearTimers() {
    this.stopCountdown();
    for (const t of this.pending) clearTimeout(t);
    this.pending.clear();
  }

  private later(ms: number, fn: () => void) {
    const handle = setTimeout(() => {
      this.pending.delete(handle);
      fn();
    }, ms);
    this.pending.add(handle);
  }

  private applyRelaySettings(settings: Settings) {
    if (!settings.relay_enabled || settings.relay_url === undefined) {
      this.emit({
        type: "relay",
        state: "disconnected",
        attempts: 0,
        source: "none",
      });
      return;
    }
    // A relay the user entered is named; the build's own relay never is (the mock has none).
    const endpoint = settings.relay_url;
    this.emit({
      type: "relay",
      endpoint,
      source: "user",
      state: "connecting",
      attempts: 0,
    });
    this.later(RELAY_CONNECT_MS, () => {
      this.emit({
        type: "relay",
        endpoint,
        source: "user",
        state: "connected",
        attempts: 0,
      });
    });
  }

  private sixDigits(): string {
    const n = Math.floor(this.random() * 1_000_000)
      .toString()
      .padStart(6, "0");
    return `${n.slice(0, 3)} ${n.slice(3)}`;
  }

  private safetyCode(): SafetyCode {
    const pick = () => SAFETY_WORDS[Math.floor(this.random() * SAFETY_WORDS.length)] ?? "amber";
    return {
      words: [pick(), pick(), pick(), pick()],
      fingerprint: fingerprintOf(this.random),
    };
  }

  /** What the desktop shell would report on its own (mirrors `Bridge::publish`): a hotkey
   *  registration result or a press. Folded into the state and broadcast like any core event. */
  publish(event: UiEvent): void {
    this.emit(event);
  }

  private emitPairing(patch: Partial<Snapshot>) {
    const next: Snapshot = { ...this.state.pairing, ...patch };
    if ("remaining_secs" in patch && patch.remaining_secs === undefined) {
      delete next.remaining_secs;
    }
    this.emit({ type: "pairing", ...next });
  }

  private emit(event: UiEvent) {
    this.state = applyEvent(this.state, event);
    this.log.push(event);
    for (const listener of this.listeners) listener(event);
  }
}

/** What an import job was asked for and how far it got (docs/dictation.md §31). */
interface MockImportRun {
  path: string;
  asr: RecognitionChoice;
  refine?: PolishModel;
  language?: string;
  /** The file was decoded: 重试 keeps it. */
  decoded: boolean;
  /** Segments recognised: a resumable 重试 goes on from the next one. */
  recognized: number;
}

/** The failures a file meets while it is decoded. */
const DECODE_FAILURES: ReadonlySet<ImportFailure["code"]> = new Set([
  "unreadable",
  "unsupported_format",
  "no_audio",
  "unsupported_codec",
  "corrupt",
  "storage",
]);

/** How many segments the preview's import cuts `duration_ms` into. */
function mockImportSegments(duration_ms: number): number {
  return Math.max(1, Math.ceil(duration_ms / MOCK_IMPORT_SEGMENT_MS));
}

/** `AudioCache::view`: the recordings and their size in all (docs/dictation.md §26). */
function cacheView(available: boolean, recordings: KeptRecording[]) {
  return { available, recordings, bytes: recordings.reduce((sum, r) => sum + r.bytes, 0) };
}

/** A paste that neither pasted nor copied (`paste_text`'s `failed { reason }`). */
function pasteFailed(reason: PasteFailure): Promise<PasteOutcome> {
  return Promise.resolve({ kind: "failed", reason });
}

/** The canned clean-up of 试一试: the examples of the built-in presets give their outputs, any other
 *  text comes back without its fillers and with a closing full stop. */
function mockPresetOutput(preset: PresetId | undefined, text: string): string {
  // An unsaved instruction (`preset` undefined) answers any example with its output.
  const sample =
    preset === undefined
      ? Object.values(MOCK_PRESET_SAMPLES).find((s) => s.input === text)
      : isBuiltinPreset(preset)
        ? MOCK_PRESET_SAMPLES[preset]
        : undefined;
  if (sample?.input === text) return sample.output;
  const cleaned = text.replace(/嗯|啊|呃|那个/g, "").trim();
  const body = cleaned.length > 0 ? cleaned : text;
  return /[。！？.!?]$/.test(body) ? body : `${body}。`;
}

/** `voltip_core::history::process::PROCESS_UNCONFIGURED`. */
export const MOCK_PROCESS_UNCONFIGURED = "尚未配置 AI 润色服务，无法用预设处理";
/** `voltip_core::history::process::PROCESS_ENTRY_GONE`. */
export const MOCK_PROCESS_ENTRY_GONE = "这条记录已删除";

/** How the status and the history name `scene` (`Scene::to_ref`). */
function sceneRefOf(scene: Scene): SceneRef {
  return {
    id: scene.id,
    name: scene.name,
    ...(scene.builtin === undefined ? {} : { builtin: scene.builtin }),
  };
}

/** The preview's fixed id of the built-in scene at `index` of `MOCK_BUILTIN_SCENES`. */
export function builtinSceneId(index: number): string {
  return `b0117e1e-5ce0-4000-8000-${String(index + 1).padStart(12, "0")}`;
}

function required<T>(args: T | undefined): T {
  if (args === undefined) throw new Error("missing command arguments");
  return args;
}

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** `voltip_core::vocabulary::MAX_UNDOS`: the accepted suggestions 撤销 can still take back. */
const MAX_SUGGESTION_UNDOS = 100;

/** An id argument as the bridge parses it: a UUID, or the command is refused. */
function uuidArg(id: string): string {
  if (!UUID.test(id)) throw new Error("id must be a UUID");
  return id;
}

/** `items` in the order of `ids` when `ids` is exactly a permutation of their ids. */
function permute<T extends { id: string }>(
  items: readonly T[],
  ids: readonly string[],
): T[] | undefined {
  if (ids.length !== items.length || new Set(ids).size !== ids.length) return undefined;
  const out: T[] = [];
  for (const id of ids) {
    const item = items.find((x) => x.id === id);
    if (item === undefined) return undefined;
    out.push(item);
  }
  return out;
}

/** Characters (code points) of the first `injected` committed sentences, joined like the paste. */
function injectedChars(committed: readonly LiveSegment[], injected: number): number {
  return Array.from(joinLiveText(committed.slice(0, injected).map((s) => s.text))).length;
}

/** `Done.segments` of a streaming take (§12): the committed sentences plus the tail as the last
 *  one, timed from the last endpoint to the end of the take. */
function streamSegments(live: LiveText, durationMs: number): LiveSegment[] {
  const tail = live.current.trim();
  if (tail.length === 0) return [...live.committed];
  const start = live.committed.at(-1)?.end_ms ?? 0;
  return [...live.committed, { text: tail, start_ms: start, end_ms: Math.max(start, durationMs) }];
}

/** Ready-made device rows for demos and page tests. */
export function sampleDevices(now = Math.floor(Date.now() / 1000)): DeviceView[] {
  const phone = phoneIdentity();
  const laptop: TrustedDevice = {
    device_id: "b08f44e7-5a91-4e2d-8c3f-071b5c3f071b",
    name: "MacBook Pro",
    platform: "macos",
    public_key: MOCK_PUBLIC_KEYS.laptop,
    fingerprint: "B0:8F:44:E7 · 5A:91:E2:D8",
    trusted_at: now - 86_400 * 12,
    last_seen: now - 86_400 * 2,
    sync: true,
    sync_gen: 0,
  };
  return [
    {
      device: {
        device_id: phone.device_id,
        name: phone.name,
        platform: phone.platform,
        public_key: phone.public_key,
        fingerprint: phone.fingerprint,
        trusted_at: now - 86_400 * 3,
        last_seen: now - 200,
        sync: true,
        sync_gen: 0,
      },
      connection: { state: "online" },
    },
    { device: laptop, connection: { state: "offline" } },
  ];
}

/** The built-in service's states the previews show (docs/dictation.md §27; Penpot 「用量与额度」
 *  board 11): under 80 %, from 80 % on, at the limit, the counting period, unknown. */
export type SampleUsageState = "ok" | "near" | "out" | "count" | "unknown";

/** A week of built-in use on this computer and today's report in `state`, for demos and screenshots:
 *  recognition in minutes and clean-ups, against 60 minutes and 100 a day. */
export function sampleUsage(
  state: SampleUsageState = "ok",
  nowMs = Date.now(),
): Partial<UsageView> {
  const dayMs = 86_400_000;
  const beijing = 8 * 3_600_000;
  const todayIndex = Math.floor((nowMs + beijing) / dayMs);
  const resetAt = ((todayIndex + 1) * dayMs - beijing) / 1000;
  const used = { ok: [23, 41], near: [52, 86], out: [60, 100], count: [23, 41], unknown: [23, 41] }[
    state
  ];
  const [asrMin = 0, refine = 0] = used;
  const week = [
    [6, 18],
    [19, 47],
    [14, 33],
    [60, 100],
    [27, 61],
    [3, 9],
  ];
  const days = [...week, [asrMin, refine]].map(([m = 0, r = 0], i) => {
    const date = new Date((todayIndex - 6 + i) * dayMs);
    return { day: date.toISOString().slice(0, 10), asr_seconds: m * 60, refine: r };
  });
  return {
    builtin: true,
    quota: {
      mode: state === "count" ? "count" : "limit",
      basis: "device",
      day: days[6]?.day ?? "",
      reset_at: resetAt,
      asr: { used_seconds: asrMin * 60, limit_seconds: state === "count" ? null : 3600 },
      refine: { used: refine, limit: state === "count" ? null : 100 },
    },
    reported_at: nowMs - 60_000,
    unavailable: state === "unknown",
    days,
    asr_origins: [
      { origin: { kind: "this_device" }, seconds: Math.max(0, asrMin - 5) * 60 },
      { origin: { kind: "phone", name: "Pixel 8" }, seconds: Math.min(asrMin, 5) * 60 },
    ],
    refine_purposes: [
      { purpose: "refine", count: Math.round(refine * 0.88) },
      { purpose: "edit", count: refine - Math.round(refine * 0.88) },
    ],
    others: [
      { provider: "local", service: "asr", model: "qwen3-asr-0.6b", amount: 7800 },
      { provider: "aliyun", service: "asr", model: "paraformer-realtime-v2", amount: 2280 },
      { provider: "groq", service: "llm", model: "qwen/qwen3.8-27b", amount: 312 },
    ],
    others_today: [{ provider: "local", service: "asr", model: "qwen3-asr-0.6b", amount: 420 }],
    dismissed: [],
  };
}

/** Local midnight of the day containing `ms`. */
export function startOfLocalDay(ms: number): number {
  const d = new Date(ms);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

const DAY_MS = 86_400_000;

function sampleRowDayOffset(row: SampleHistoryRow): number {
  if (row.day.startsWith("今天")) return 0;
  if (row.day.startsWith("昨天")) return 1;
  return 2;
}

function sampleRowOutcome(row: SampleHistoryRow): HistoryEntry["outcome"] {
  switch (row.outcome) {
    case "inserted":
      return { kind: "inserted", via: "paste" };
    case "lost":
      return { kind: "failed", reason: "目标窗口已丢失" };
    case "staged":
    case "returned":
    case "unfocused":
      return { kind: "clipboard", reason: "目标窗口没有焦点" };
  }
}

/** The ids of the sample backup's items, apart from the sample history's own. */
function sampleBackupId(n: number): string {
  return `00000000-0000-4000-9000-${String(n).padStart(12, "0")}`;
}

/** `voltip-auto-20261030-152000-weekly.voltip-backup`: an automatic backup's name as the core
 *  writes it (`voltip_core::backup::auto::auto_name`), in local time (docs/dictation.md §4.6.1). */
function autoBackupName(kind: AutoBackupKind, atMs: number): string {
  const d = new Date(atMs);
  const date = `${d.getFullYear()}${twoDigits(d.getMonth() + 1)}${twoDigits(d.getDate())}`;
  const time = `${twoDigits(d.getHours())}${twoDigits(d.getMinutes())}${twoDigits(d.getSeconds())}`;
  return `voltip-auto-${date}-${time}-${kind}.${BACKUP_EXTENSION}`;
}

/** `7` → `07`. */
function twoDigits(n: number): string {
  return String(n).padStart(2, "0");
}

/** Items per part of a backup's content. */
function backupCounts(content: Omit<MockBackupFile, "manifest">): BackupCounts {
  return {
    history: content.history.length,
    dictionary: content.dictionary.length,
    rules: content.rules.length,
    scenes: content.scenes.length,
    presets: content.presets.length,
  };
}

/** The backup the preview's open dialog finds before anything was exported (docs/dictation.md
 *  §4.6): another computer's, made a day before `nowMs`, with the sample history under ids of its
 *  own, a dictionary entry, a rule, a preset and a scene that uses it. */
export function sampleBackup(nowMs: number): MockBackupFile {
  const made = nowMs - DAY_MS;
  const id = sampleBackupId;
  const history = sampleHistory(made).map((entry, i) => ({ ...entry, id: id(100 + i) }));
  const dictionary = [
    dictionaryEntrySchema.parse({
      id: id(1),
      term: "Kubernetes",
      heard_as: ["酷伯奈提斯"],
      enabled: true,
      source: { kind: "manual" },
      created_at_ms: made,
      updated_at_ms: made,
    }),
  ];
  const rules = [
    replacementRuleSchema.parse({
      id: id(2),
      name: "句号",
      kind: "literal",
      pattern: "句号",
      replacement: "。",
      case_sensitive: true,
      enabled: true,
      created_at_ms: made,
      updated_at_ms: made,
    }),
  ];
  const presets: CustomPreset[] = [
    {
      id: id(3),
      name: "周报",
      prompt: "把口述整理成周报：本周完成、下周计划、风险。",
      created_at_ms: made,
      updated_at_ms: made,
    },
  ];
  const scenes: Scene[] = [
    {
      id: id(4),
      name: "Outlook",
      enabled: true,
      match: { apps: ["outlook"], title_contains: [] },
      overrides: { refine_preset: id(3) },
      created_at_ms: made,
      updated_at_ms: made,
    },
  ];
  const content = { history, dictionary, rules, scenes, presets };
  return {
    manifest: {
      format: "voltip-backup",
      version: 1,
      app_version: MOCK_CURRENT_VERSION,
      created_at_ms: made,
      device: "Office-PC",
      platform: "windows",
      counts: backupCounts(content),
    },
    ...content,
  };
}

/** The sample history rows as real `HistoryEntry` records, dated relative to `nowMs` (today /
 *  yesterday / two days ago by local calendar) so the home statistics have something to count.
 *  Rows that carried no text (the protected-field placeholder) are dropped: the core never stores
 *  those. */
export function sampleHistory(nowMs: number): HistoryEntry[] {
  const today = startOfLocalDay(nowMs);
  // The fixture is listed newest first; `previous` keeps it that way after clamping.
  let previous = Number.POSITIVE_INFINITY;
  return historyEntries
    .filter((row) => row.text.length > 0)
    .map((row) => {
      const [, h = "0", m = "0", sec = "0"] = /(\d+):(\d+):(\d+)$/.exec(row.timestamp) ?? [];
      const dayStart = today - sampleRowDayOffset(row) * DAY_MS;
      let at = dayStart + (Number(h) * 3600 + Number(m) * 60 + Number(sec)) * 1000;
      // Today's rows must not sit in the future, and every row stays older than the one before.
      if (at > nowMs) at = nowMs - 5 * 60_000;
      if (at >= previous) at = previous - 5 * 60_000;
      at = Math.max(dayStart, at);
      previous = at;
      const refined = row.timing.polish !== null;
      return {
        id: `00000000-0000-4000-8000-${String(row.id).padStart(12, "0")}`,
        at_ms: at,
        raw_text: row.rawText,
        text: row.text,
        refined,
        asr_model: MOCK_ENGINE_BUILTIN.asr_model,
        ...(refined
          ? {
              refine_model: MOCK_ENGINE_BUILTIN.refine_model,
              refine_ms: row.timing.polish ?? 0,
              preset: { id: "proofread" as const, name: "校对" },
            }
          : {}),
        duration_ms: Math.round(row.audioSecs * 1000),
        asr_ms: row.timing.asr,
        outcome: sampleRowOutcome(row),
        starred: row.starred,
        mode: "whole_take" as const,
        kind: "dictation" as const,
      };
    });
}
