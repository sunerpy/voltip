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
import type { Backend, EventListener, FrameListener, Unsubscribe } from "./backend";
import { type SampleHistoryRow, historyEntries } from "./fixtures/history";
import builtinPresetTexts from "./fixtures/ipc/presets-builtin.json";
import builtinSceneRows from "./fixtures/ipc/scenes-builtin.json";
import { joinLiveText, livePreviewText } from "./labels";
import { type BuiltInService, keyEntry, providerSpec, resolveEngineStatus } from "./providers";
import {
  type AppRef,
  type ArgsOf,
  type CommandArgs,
  type DeviceConnection,
  type DeviceIdentityPublic,
  type DeviceInfo,
  type DeviceView,
  type DictationPhase,
  type DictationFailureCode,
  type DictionaryEntry,
  type EntrySource,
  type ImportMode,
  type PreviewDraft,
  type ReplacementRule,
  type RuleDraft,
  type VocabularyPreview,
  type EngineSettings,
  type EngineStatus,
  type HistoryEntry,
  type HotkeyEdgeArgs,
  type InjectPreflight,
  type PasteFailure,
  type PasteOutcome,
  type LiveSegment,
  type LiveText,
  type ModelInstallState,
  type ModelState,
  type MutationCommand,
  type OutputMode,
  type Permission,
  type PermissionReport,
  type RelayStatus,
  type SafetyCode,
  type Scene,
  type ServeSettings,
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
  type ConnectivityReport,
  type HotkeyCapabilities,
  type HotkeyStatus,
  SOLO_KEYS,
  type SoloKey,
  MAX_PHONE_TEXT_CHARS,
  MAX_PASTE_TEXT_CHARS,
  type SentText,
  type NearbyDevice,
  applyEvent,
  DEFAULT_HOTKEY,
  defaultSettings,
  emptyEngineStatus,
  emptyHotkeyStatus,
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
import { historyHitsOf, historyPageOf, historyStatsOf } from "./history-queries";
import {
  Vocabulary,
  checkDictionary,
  checkRules,
  exportRulesToml,
  parseRulesToml,
  previewVocabulary,
  validateDictionaryDraft,
  validateRuleDraft,
} from "./vocabulary";

export interface MockPeer {
  /** Milliseconds after `waiting_for_peer` until the simulated phone joins. */
  joinAfterMs: number;
  /** Milliseconds after joining until the simulated phone confirms the safety code. */
  confirmAfterMs: number;
  info?: DeviceInfo;
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
  /** Overrides for the resolved engine status (tests that need a status the settings cannot
   *  produce). */
  engines?: Partial<EngineStatus>;
  /** The local service's status at start, instead of the one the settings give (tests that need
   *  a status the preview passes through only for a moment, such as `starting`). */
  serve?: ServeStatus;
  /** Which built-in services the preview pretends were compiled in (default: both, with keys);
   *  `{}` is a build without them. */
  builtIn?: { asr?: BuiltInService; llm?: BuiltInService };
  /** Providers whose key is already stored (`provider_key_set` adds / removes more). */
  providerKeys?: readonly { provider: ProviderId; kind: ServiceKind }[];
  /** Model ids `provider_probe` answers with; `false` makes every probe fail as `unreachable`. */
  probeModels?: Partial<Record<ProviderId, readonly string[]>> | false;
  /** Initial updater status (`disabled` for a build without an update source, `failed`, …). */
  update?: UpdateStatus;
  /** Install state per catalogue id (`{ "sense-voice-small": { kind: "installed", … } }`); every
   *  model starts `not_installed` otherwise. */
  models?: Partial<Record<string, ModelInstallState>>;
  /** Initial personal dictionary and replacement rules (docs/dictation.md §16); both start empty,
   *  like a first run of the core. Ignored by the phone role (the phone has neither). */
  dictionary?: DictionaryEntry[];
  rules?: ReplacementRule[];
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
/** `voltip_core::history::process::PART_MAX_CHARS`: the preview's 用 AI 预设处理 counts its parts
 *  the same way (docs/dictation.md §22). */
export const MOCK_PART_MAX_CHARS = 1_500;
/** Where the preview says an export was saved. */
export const MOCK_EXPORT_DIR = "/home/you/Documents";
export const MOCK_DICTATION_DWELL_MS = 2500;
export const MOCK_DICTATION_FAILED_DWELL_MS = 6000;
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

/** A catalogue row with its directory and its files' public addresses, as the core reports them. */
function withFiles(
  row: Omit<ModelState, "active" | "state" | "dir" | "files">,
): Omit<ModelState, "active" | "state"> {
  return {
    ...row,
    dir: `${MOCK_MODELS_ROOT}/${row.id}`,
    files: (MOCK_MODEL_FILES[row.id] ?? []).map(([name, size]) => ({
      name,
      size_bytes: size,
      urls: ["https://huggingface.co", "https://hf-mirror.com"].map(
        (base) => `${base}/${row.repo}/resolve/main/${name}`,
      ),
    })),
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
/** What the preview phone's LAN browse sees (docs/pairing.md 「局域网发现」). */
export const MOCK_NEARBY: readonly NearbyDevice[] = [
  {
    fingerprint: "A7C4198E3DF26109",
    name: "Studio",
    platform: "macos",
    pairing: true,
    trusted: false,
  },
];

/** Always-on pairing in the preview (docs/pairing.md 「常开配对」, the core's `RENEW_BEFORE_SECS`
 *  and `FINISHED_PAUSE`): a waiting session this close to its end is renewed, and a finished one
 *  is followed by the next after this pause. */
export const MOCK_ALWAYS_ON_RENEW_SECS = 10;
export const MOCK_ALWAYS_ON_PAUSE_MS = 4000;
/** The core's refusal on a phone. */
export const ALWAYS_ON_DESKTOP_ONLY = "pairing: 常开配对只在电脑上可用";

/** How long the preview's desktop takes to insert a phone's text (docs/dictation.md §20.6). */
export const MOCK_TEXT_MS = 200;
/** Ports the preview's local speech service cannot bind (another program has them). */
export const MOCK_TAKEN_PORTS: readonly number[] = [47999];

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
  /** Every `providerConsoleOpen` made, in order (tests). */
  readonly consolesOpened: ProviderId[] = [];
  /** `project_link_open` calls, for tests. */
  readonly linksOpened: ProjectLink[] = [];
  /** `guide_open` calls (page and language), for tests. */
  readonly guidesOpened: { page: GuidePage; locale: string }[] = [];
  /** `model_folder_open` calls (model ids), for tests. */
  readonly modelFoldersOpened: string[] = [];
  /** What the phone's `update_install` opened: `store` for the listing, else the release's version. */
  readonly updatePagesOpened: string[] = [];
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
  /** `history_process` requests running (docs/dictation.md §22). */
  private readonly processing = new Map<
    number,
    { id: string; timer: ReturnType<typeof setTimeout> }
  >();
  /** Every event emitted, oldest first; handy for asserting ordering in tests. */
  readonly log: UiEvent[] = [];
  /** The whole history, newest first: what the core keeps in `history.sqlite3`. The state holds
   *  its newest `HISTORY_RECENT` and the total, as `UiState` does (docs/dictation.md §4.4). */
  private history: HistoryEntry[] = [];

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
      llm: { model: MOCK_ENGINE_BUILTIN.refine_model, key: true },
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
        backend: MOCK_HOTKEY_BACKEND,
        ...(this.role === "phone" ? {} : { capabilities: { ...MOCK_HOTKEY_CAPABILITIES } }),
      },
      dictation: idleDictation(),
      sent_texts: [],
      // A desktop on the LAN waiting for a pairing (docs/pairing.md 「局域网发现」): the phone lists it.
      nearby: this.role === "phone" ? [...MOCK_NEARBY] : [],
      history_recent: this.history.slice(0, HISTORY_RECENT),
      history_total: this.history.length,
      engines: emptyEngineStatus(),
      update: options.update ?? idleUpdate(),
      models,
      dictionary: [...(options.dictionary ?? [])],
      rules: [...(options.rules ?? [])],
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
    };
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
  async historyExport(id: string, format: ExportFormat, fileName: string): Promise<ExportOutcome> {
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
    this.exports.push({ id, format, fileName });
    if (this.role === "phone") return { kind: "shared" };
    return { kind: "saved", path: `${MOCK_EXPORT_DIR}/${fileName}.${format}` };
  }

  /** Every `historyExport` that saved or was shared, in order (tests). */
  readonly exports: { id: string; format: ExportFormat; fileName: string }[] = [];

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
      this.startPairing();
    },
    pairing_join_code: (args) => {
      this.joinWithCode(required(args).code);
    },
    pairing_join_ticket: (args) => {
      this.joinWithTicket(required(args).uri);
    },
    pairing_join_nearby: (args) => {
      // Mirrors `PairingJoinNearby`: only a nearby device that waits for a pairing can be joined.
      const { fingerprint } = required(args);
      const device = this.state.nearby.find((d) => d.fingerprint === fingerprint);
      if (device === undefined) {
        this.emit({ type: "error", message: "pairing: 附近没有找到此设备" });
        return;
      }
      if (!device.pairing) {
        this.emit({
          type: "error",
          message: "pairing: 此设备当前没有等待配对",
        });
        return;
      }
      this.joinSession();
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
    settings_set_lan_discovery: (args) => {
      const { enabled } = required(args);
      this.emit({
        type: "settings",
        ...this.state.settings,
        lan_discovery: enabled,
      });
      this.emit({
        type: "nearby",
        devices: enabled && this.role === "phone" ? [...MOCK_NEARBY] : [],
      });
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
      }
      this.emit({
        type: "settings",
        ...this.state.settings,
        edit_hotkey: hotkey,
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
      const settings: Settings = {
        ...this.state.settings,
        engines: { ...engines },
      };
      this.emit({ type: "settings", ...settings });
      this.emit({ type: "engines", ...this.resolveEngines(settings.engines) });
      this.emit({ type: "models", models: this.modelsFor(settings.engines) });
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
      this.emit({
        type: "engines",
        ...this.resolveEngines(this.state.settings.engines),
      });
    },
    provider_probe: (args) => {
      const { provider, kind, baseUrl, key } = required(args);
      this.probe(provider, kind, baseUrl ?? undefined, key ?? undefined);
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
    },
    history_clear: () => {
      this.setHistory([]);
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
      if (this.history.length > keep) this.setHistory(this.history.slice(0, keep));
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
      this.commitRules(
        this.state.rules.map((r) => (r.id === id ? this.ruleFrom(draft, id, r.created_at_ms) : r)),
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
  };

  /** How often 复制令牌 was pressed (tests; the token itself never reaches the interface). */
  serveTokenCopies = 0;
  /** How often 重新生成 was pressed (tests). */
  serveTokenRotations = 0;

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

  private ruleFrom(draft: RuleDraft, id: string, createdAtMs: number): ReplacementRule {
    return {
      id,
      ...draft,
      created_at_ms: createdAtMs,
      updated_at_ms: this.now(),
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
      else rules[at] = this.ruleFrom(draft, existing.id, existing.created_at_ms);
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

  /** The other device joined the session: Noise XX runs, then both sides see the safety code. */
  simulatePeerJoined(peer: DeviceInfo = this.role === "phone" ? desktopPeer() : phonePeer()) {
    const phase = this.state.pairing.state.state;
    if (phase !== "waiting_for_peer" && phase !== "creating_session") return;
    this.stopCountdown();
    this.emitPairing({ state: { state: "key_exchange" } });
    this.later(KEY_EXCHANGE_MS, () => {
      this.emitPairing({
        state: { state: "awaiting_verification" },
        safety_code: this.safetyCode(),
        peer,
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
                ? {
                    ...d.device,
                    last_seen: Math.floor(this.now() / 1000),
                    last_connection: connection.via,
                  }
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
    for (const t of this.modelTimers.values()) clearTimeout(t);
    this.modelTimers.clear();
    this.listeners.clear();
  }

  // ---- local model simulation (docs/dictation.md §10) -------------------------------------------

  /** The core's test hook: a download that failed mid-way (network, checksum). The `.part` files
   *  stay, so `model_download` on the same id is the retry. */
  simulateModelFailed(id: string, message: string) {
    if (!this.catalogueRow(id)) return;
    this.stopModelTimer(id);
    this.setModelState(id, { kind: "failed", message });
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
        file: MOCK_MODEL_FILE,
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
      file: MOCK_MODEL_FILE,
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
    const current = this.modelState(id);
    if (current === undefined || current.kind === "not_installed") return;
    this.stopModelTimer(id);
    this.setModelState(id, { kind: "not_installed" });
  }

  /** `model_import` (docs/dictation.md §10): a verifying beat, then installed, or what
   *  {@link simulateImportProblems} said the directory lacks. */
  private importModel(id: string) {
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

  /** `Effect::Record`: nothing when history is off, the newest `keep` otherwise. */
  private recordHistory(entry: HistoryEntry) {
    const { enabled, keep } = this.state.settings.history;
    if (!enabled) return;
    this.setHistory([entry, ...this.history].slice(0, keep));
  }

  // ---- providers (docs/dictation.md §3.3) ----------------------------------------------------------

  providerConsoleOpen(provider: ProviderId): Promise<void> {
    if (!providerSpec(provider).console)
      return Promise.reject(new Error(`${provider}: no key page`));
    this.consolesOpened.push(provider);
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
    const url = this.state.models.find((m) => m.id === id)?.files.find((f) => f.name === file)
      ?.urls[source];
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
    if (provider !== "builtin") {
      const saved = this.state.settings.engines.providers?.[provider];
      const url =
        baseUrl?.trim() ||
        (kind === "asr" ? saved?.asr_url : saved?.llm_url) ||
        (preset.baseUrl.length > 0 ? preset.baseUrl : undefined);
      if (url === undefined || !/^https?:\/\/[^/]+/.test(url)) return failed("invalid_url");
      const entry = keyEntry(provider, kind);
      const hasKey =
        (key?.trim().length ?? 0) > 0 || (entry !== undefined && this.userKeys.has(entry));
      if (spec.key === "required" && !hasKey) return failed("key_missing");
    }
    const handle = setTimeout(() => {
      this.probeTimers.delete(handle);
      if (this.probeModels === false) return failed("unreachable");
      const models = [
        ...(this.probeModels[provider] ??
          (provider === "builtin" ? [this.builtIn[kind]?.model ?? ""] : preset.models)),
      ];
      models.sort();
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
    }
    if (phase.phase !== "listening" && phase.phase !== "processing") this.takeSegments = undefined;
    const context = this.takeContext === undefined ? {} : { context: this.takeContext };
    const remote = this.takeRemote === undefined ? {} : { remote: this.takeRemote };
    const preset = this.takePreset === undefined ? {} : { preset: this.takePreset };
    const source = this.takeSource === undefined ? {} : { source: this.takeSource };
    const segments = this.takeSegments === undefined ? {} : { segments: this.takeSegments };
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
    const { hotkey, edit_hotkey, solo_key } = this.state.settings;
    return {
      ...(capturing ? {} : { registered: hotkey }),
      ...(capturing || edit_hotkey === null ? {} : { edit_registered: edit_hotkey }),
      ...(capturing ? {} : mockSoloStatus(solo_key)),
      pressed: false,
      capturing,
      backend: MOCK_HOTKEY_BACKEND,
      capabilities: { ...MOCK_HOTKEY_CAPABILITIES },
      solo_pressed: false,
    };
  }

  /** A self-check against the mock's own state: the relay answers when its link is up, an online
   *  device has a round trip, and its LAN addresses answer only when it is connected directly. */
  private connectivityReport(): ConnectivityReport {
    const relayUp = this.state.relay.state === "connected";
    return {
      checked_at: this.now(),
      lan: {
        listening: true,
        addresses: [this.role === "phone" ? "192.168.1.52:47831" : "192.168.1.30:47831"],
      },
      relay: {
        configured: this.state.relay.source !== "none",
        ...(this.state.relay.source === "none"
          ? {}
          : {
              result: relayUp ? { result: "ok" as const, ms: 48 } : { result: "timeout" as const },
            }),
      },
      peers: this.state.devices.map((d) => {
        const online = d.connection.state === "online" ? d.connection.via : undefined;
        return {
          public_key: d.device.public_key,
          name: d.device.name,
          ...(online === undefined ? {} : { via: online, rtt_ms: online === "direct" ? 6 : 61 }),
          addresses: (d.device.direct_hints ?? []).map((address) => ({
            address,
            same_subnet: address.startsWith("192.168.1."),
            result:
              online === "direct"
                ? { result: "ok" as const, ms: 5 }
                : { result: "timeout" as const },
          })),
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
    const asrMs = streaming ? MOCK_FINALIZE_MS : MOCK_ASR_MS;
    const raw = streaming ? preview : MOCK_DICTATION_RAW;
    const durationMs = Math.max(0, stoppedAt - startedAt);
    const segments = streaming && live !== undefined ? streamSegments(live, durationMs) : undefined;
    // The status names the clean-up's preset from the start of processing (§21).
    const preset = refine ? this.resolveTakePreset() : undefined;
    this.takePreset = preset;
    this.emitPhase({
      phase: "processing",
      stage: streaming ? "finalizing" : "transcribing",
      started_at: stoppedAt,
      stage_started_at: stoppedAt,
      ...carried,
    });
    const finish = () => {
      if (this.state.dictation.session !== session) return;
      // The phone's result goes to its clipboard (docs/dictation.md §20.7), whatever the setting.
      const via =
        this.role === "phone" || engines.inject === "clipboard_only" ? "clipboard" : "paste";
      // docs/dictation.md §16.3: the dictionary corrects the transcript, the (canned) LLM keeps the
      // glossary terms, the rules run last; an emptied text is no speech, nothing is inserted.
      const vocabulary = Vocabulary.compile(this.state.dictionary, this.state.rules);
      const corrected = vocabulary.correct(raw);
      const refined = refine ? vocabulary.correct(MOCK_DICTATION_TEXT).text : corrected.text;
      const ruled = vocabulary.applyRules(refined);
      const text = ruled.text;
      if (text.trim().length === 0) {
        this.emitPhase({
          phase: "failed",
          message: MOCK_NO_SPEECH,
          code: "no_speech",
        });
        this.dwell(MOCK_DICTATION_DWELL_MS, session);
        return;
      }
      const hits = { corrections: corrected.hits, rules: ruled.hits };
      const failedOver = liveError ?? modeError;
      const extra = {
        ...(refine ? { refine_ms: MOCK_REFINE_MS } : {}),
        ...(segments === undefined ? {} : { segments }),
        ...(failedOver === undefined ? {} : { live_error: failedOver }),
      };
      const done: DictationPhase = {
        phase: "done",
        text,
        raw_text: raw,
        chars: Array.from(text).length,
        via,
        refined: refine,
        duration_ms: durationMs,
        asr_ms: asrMs,
        mode,
        ...extra,
      };
      const entry: HistoryEntry = {
        id: this.uuid(),
        at_ms: this.now(),
        raw_text: raw,
        text,
        refined: refine,
        asr_model: engines.asr_model,
        ...(refine ? { refine_model: engines.refine_model } : {}),
        duration_ms: durationMs,
        asr_ms: asrMs,
        outcome: { kind: "inserted", via },
        starred: false,
        mode,
        ...extra,
        ...(hits.corrections.length + hits.rules.length > 0 ? { vocabulary: hits } : {}),
        ...(context === undefined ? {} : { app: context.app }),
        ...(scene === undefined ? {} : { scene: context?.scene ?? sceneRefOf(scene) }),
        ...(preset === undefined ? {} : { preset }),
        kind: "dictation",
      };
      this.recordHistory(entry);
      if (this.role === "phone") this.phoneClipboard = text;
      this.emitPhase(done);
      this.dwell(MOCK_DICTATION_DWELL_MS, session);
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
            outcome: { kind: "inserted", via },
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
          });
          this.dwell(MOCK_DICTATION_DWELL_MS, session);
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
    // `live_inject` does not take back what it pasted (§12); every other mode reports 0.
    this.emitPhase({ phase: "cancelled", injected_chars: this.injectedChars });
    this.dwell(MOCK_DICTATION_DWELL_MS, this.state.dictation.session);
  }

  /** The core's test hook: a failed run (ASR error, or injection failure with the text kept). */
  simulateDictationFailed(message: string, text?: string) {
    this.clearDictationTimers();
    this.clearLiveTimers();
    const session = this.state.dictation.session;
    this.emitPhase(
      text === undefined ? { phase: "failed", message } : { phase: "failed", message, text },
    );
    this.dwell(
      text === undefined ? MOCK_DICTATION_DWELL_MS : MOCK_DICTATION_FAILED_DWELL_MS,
      session,
    );
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
          this.simulatePeerJoined(peer.info);
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
    this.joinSession();
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
    this.joinSession();
  }

  private joinSession() {
    this.clearTimers();
    this.emitPairing({
      ...idleSnapshot(),
      state: { state: "creating_session" },
    });
    this.later(CREATE_SESSION_MS, () => {
      this.simulatePeerJoined();
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
      last_connection: "direct",
      sync: true,
      sync_gen: 0,
    };
    this.emitPairing({ state: { state: "trusted" } });
    this.openNextIfAlwaysOn();
    this.emit({ type: "trusted", ...device });
    const others = this.state.devices.filter((d) => d.device.public_key !== publicKey);
    this.emit({
      type: "devices",
      devices: [...others, { device, connection: { state: "online", via: "direct" } }],
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
    last_connection: "relay",
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
        last_connection: "direct",
        direct_hints: ["192.168.1.37:47831"],
        sync: true,
        sync_gen: 0,
      },
      connection: { state: "online", via: "direct" },
    },
    { device: laptop, connection: { state: "offline" } },
  ];
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
