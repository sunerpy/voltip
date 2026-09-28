// IPC contract between the Rust core (`voltip-core::ui`) and the webviews. Every shape mirrors the
// serde output documented in docs/frontend.md §3; events that fail validation are
// dropped by the backends rather than half-applied.
import { z } from "zod";

export const THEME_IDS = ["light", "dark", "warm", "graphite"] as const;
export const themeIdSchema = z.enum(THEME_IDS);
export type ThemeId = z.infer<typeof themeIdSchema>;

export const platformSchema = z.enum(["windows", "macos", "linux", "android", "ios", "other"]);
export type Platform = z.infer<typeof platformSchema>;

export const connectionStateSchema = z.enum([
  "disconnected",
  "connecting",
  "authenticating",
  "connected",
  "reconnecting",
  "closed",
]);
export type ConnectionState = z.infer<typeof connectionStateSchema>;

export const connectionKindSchema = z.enum(["direct", "relay"]);
export type ConnectionKind = z.infer<typeof connectionKindSchema>;

/** 64 lower-case hex characters: an X25519 public key. */
export const hexKeySchema = z.string().regex(/^[0-9a-f]{64}$/, "expected 64 lower-case hex chars");

/** How the core hands the finished text to the front application (`voltip_core::engines::InjectMode`). */
export const injectModeSchema = z.enum(["paste", "clipboard_only"]);
export type InjectMode = z.infer<typeof injectModeSchema>;

/** Who serves recognition or clean-up (`voltip_core::providers::ProviderId`, docs/dictation.md
 *  §3): the build's own service, the on-device models, a few OpenAI-compatible vendors, Ollama on
 *  this machine and a custom endpoint. Display order. */
export const PROVIDER_IDS = [
  "builtin",
  "local",
  "openai",
  "groq",
  "siliconflow",
  "deepseek",
  "ollama",
  "custom",
] as const;
export const providerIdSchema = z.enum(PROVIDER_IDS);
export type ProviderId = z.infer<typeof providerIdSchema>;

/** The two services a provider may offer: speech recognition and text clean-up (LLM). */
export const SERVICE_KINDS = ["asr", "llm"] as const;
export const serviceKindSchema = z.enum(SERVICE_KINDS);
export type ServiceKind = z.infer<typeof serviceKindSchema>;

/** Where a provider's credential comes from (`voltip_core::providers::KeyPolicy`). */
export const keyPolicySchema = z.enum(["builtin", "required", "optional", "none"]);
export type KeyPolicy = z.infer<typeof keyPolicySchema>;

/** Where on-device models run (`voltip_core::engines::LocalDevice`, docs/dictation.md §10.4). */
export const LOCAL_DEVICES = ["auto", "cpu", "gpu"] as const;
export const localDeviceSchema = z.enum(LOCAL_DEVICES);
export type LocalDevice = z.infer<typeof localDeviceSchema>;
/** `voltip_core::engines::MAX_LOCAL_THREADS`. */
export const MAX_LOCAL_THREADS = 256;

/** Why a service cannot run (`voltip_core::engines::EngineIssue`). */
export const ENGINE_ISSUES = [
  "unavailable",
  "key_missing",
  "url_missing",
  "model_missing",
  "model_not_installed",
  "no_provider",
] as const;
export const engineIssueSchema = z.enum(ENGINE_ISSUES);
export type EngineIssue = z.infer<typeof engineIssueSchema>;

/** How the final text is produced and delivered (`voltip_core::engines::OutputMode`,
 *  docs/dictation.md §12): the whole take recognised after release (default), the streaming
 *  recogniser's final text (no second pass), or every sentence pasted the moment it closes. */
export const OUTPUT_MODES = ["whole_take", "streaming_final", "live_inject"] as const;
export const outputModeSchema = z.enum(OUTPUT_MODES);
export type OutputMode = z.infer<typeof outputModeSchema>;

/** The two modes that ride on the streaming recogniser (they need `live_preview_ready`). */
export function isStreamingOutputMode(mode: OutputMode): boolean {
  return mode !== "whole_take";
}

/** Which script Chinese text is brought to right after recognition, before the dictionary
 *  (`voltip_core::engines::ChineseScript`, docs/dictation.md §17): Simplified (default),
 *  Traditional, or left as the recogniser wrote it. */
export const CHINESE_SCRIPTS = ["simplified", "traditional", "as_is"] as const;
export const chineseScriptSchema = z.enum(CHINESE_SCRIPTS);
export type ChineseScript = z.infer<typeof chineseScriptSchema>;

/** How far the LLM clean-up may rewrite (`voltip_core::engines::RefineStyle`, docs/dictation.md
 *  §18.1): only a scene overrides it; the global setting is always `default`. */
export const REFINE_STYLES = ["default", "punctuation", "formal"] as const;
export const refineStyleSchema = z.enum(REFINE_STYLES);
export type RefineStyle = z.infer<typeof refineStyleSchema>;

/** One provider's choices (`voltip_core::engines::ProviderSettings`): absent = the preset. */
export const providerSettingsSchema = z.object({
  asr_model: z.string().optional(),
  asr_url: z.string().optional(),
  llm_model: z.string().optional(),
  llm_url: z.string().optional(),
});
export type ProviderSettings = z.infer<typeof providerSettingsSchema>;

/** `Settings.engines` (`voltip_core::engines::EngineSettings`, `#[serde(default)]`): one provider
 *  per service, per-provider model / endpoint choices, the on-device runtime and the pipeline
 *  switches. Keys never live here: they go through `provider_key_set` into the system keychain. */
export const engineSettingsSchema = z.object({
  /** Recognition provider; `builtin` falls back to `local` in a build without the built-in. */
  asr_provider: providerIdSchema.default("builtin"),
  /** Clean-up provider; `builtin` means none in a build without the built-in. */
  llm_provider: providerIdSchema.default("builtin"),
  refine_enabled: z.boolean().default(true),
  /** Per-provider choices; Rust omits the map when empty. */
  providers: z.partialRecord(providerIdSchema, providerSettingsSchema).optional(),
  /** Catalogue id of the local model (`qwen3-asr-0.6b`); absent / `null` = the catalogue default. */
  local_model: z.string().nullable().optional(),
  /** Where local models run; `auto` picks a GPU when the build has a backend for it. */
  local_device: localDeviceSchema.default("auto"),
  /** The GPU `local_device = gpu` asks for (a device name from `UiState.hardware`). */
  local_gpu: z.string().nullable().optional(),
  /** Inference threads for local models; absent = decided per engine. */
  local_threads: z.number().int().min(1).max(MAX_LOCAL_THREADS).nullable().optional(),
  /** ISO language hint (`zh`, `en`); absent = auto-detect. */
  language: z.string().optional(),
  /** Live preview while listening (docs/dictation.md §11): on by default; only takes effect once
   *  the streaming model (`zipformer-stream-zh-en`) is installed, whatever the provider. */
  live_preview: z.boolean().default(true),
  /** Output mode (docs/dictation.md §12); the streaming modes fall back to `whole_take` while the
   *  streaming model is missing (`EngineStatus.effective_output_mode` says which one runs). */
  output_mode: outputModeSchema.default("whole_take"),
  /** Trim leading / trailing silence with Silero VAD before local recognition (§12). */
  vad_trim: z.boolean().default(false),
  /** Script of every recogniser's Chinese text (§17); always serialised by the core. */
  chinese_script: chineseScriptSchema.default("simplified"),
  inject: injectModeSchema.default("paste"),
});
export type EngineSettings = z.infer<typeof engineSettingsSchema>;

/** Defaults mirroring `EngineSettings::default()`: the built-in service for both, no local model
 *  picked, automatic device, live preview on, whole-take output without VAD, Simplified Chinese,
 *  refine on, paste injection. */
export function defaultEngineSettings(): EngineSettings {
  return {
    asr_provider: "builtin",
    llm_provider: "builtin",
    refine_enabled: true,
    local_device: "auto",
    live_preview: true,
    output_mode: "whole_take",
    vad_trim: false,
    chinese_script: "simplified",
    inject: "paste",
  };
}

/** How a press / release of the hotkey drives a take (`voltip_core::dictation::activation::Activation`,
 *  docs/dictation.md §13): hold to talk (default), press to start and press again to stop, or hold
 *  with a short press locking the take. */
export const ACTIVATIONS = ["hold", "toggle", "hold_or_toggle"] as const;
export const activationSchema = z.enum(ACTIVATIONS);
export type Activation = z.infer<typeof activationSchema>;

/** `voltip_core::dictation::activation::DEFAULT_HOLD_THRESHOLD_MS`: a `hold_or_toggle` release
 *  before this many milliseconds locks the take instead of stopping it. */
export const DEFAULT_HOLD_THRESHOLD_MS = 300;
/** `voltip_core::MAX_ACTIVATION_MS`: the core refuses larger threshold / extra-recording values. */
export const MAX_ACTIVATION_MS = 5000;

/** Which parts of a take's context may go to the LLM clean-up (`voltip_core::scenes::ContextSharing`,
 *  docs/dictation.md §18.5): the app name (on by default) and the window title (off by default).
 *  `#[serde(default)]` on both levels, always serialised. */
export const contextSharingSchema = z.object({
  app_name: z.boolean().default(true),
  window_title: z.boolean().default(false),
});
export type ContextSharing = z.infer<typeof contextSharingSchema>;

/** Mirrors `ContextSharing::default()`. */
export function defaultContextSharing(): ContextSharing {
  return { app_name: true, window_title: false };
}

/** UI language as persisted by the core: `system` follows the OS / webview language. */
export const LOCALE_SETTINGS = ["system", "zh-cn", "en"] as const;
export const localeSettingSchema = z.enum(LOCALE_SETTINGS);
export type LocaleSetting = z.infer<typeof localeSettingSchema>;

/** The license Voltip is published under (the root `package.json` names the same; a test keeps
 *  them equal). */
/** Project pages the shell opens (`voltip_core::ui::ProjectLink`): the repository and its
 *  new-issue page. */
export const PROJECT_LINKS = ["source", "feedback", "releases"] as const;
export const projectLinkSchema = z.enum(PROJECT_LINKS);
export type ProjectLink = z.infer<typeof projectLinkSchema>;

// ---- in-app feedback (docs/feedback.md; the shell's src/feedback.rs) -------------------------

export const FEEDBACK_KINDS = ["bug", "idea", "other"] as const;
export const feedbackKindSchema = z.enum(FEEDBACK_KINDS);
export type FeedbackKind = z.infer<typeof feedbackKindSchema>;
/** The endpoint's limits, in UTF-16 code units like `maxLength`. */
export const FEEDBACK_MESSAGE_MAX = 5000;
export const FEEDBACK_CONTACT_MAX = 200;

/** What a report carries besides the user's words: versions, platform, which provider kind is in
 *  use. Never a host, a key or a dictation. */
export const feedbackDiagnosticsSchema = z.object({
  app_version: z.string(),
  os: z.string(),
  arch: z.string(),
  session: z.string().optional(),
  locale: z.string(),
  asr_provider: z.string(),
  local_model: z.string().optional(),
  compute: z.string().optional(),
  llm_provider: z.string().optional(),
  output_mode: z.string(),
});
export type FeedbackDiagnostics = z.infer<typeof feedbackDiagnosticsSchema>;

/** `feedback_diagnostics`: whether this build has an endpoint, and what a report would carry. */
export const feedbackInfoSchema = z.object({
  configured: z.boolean(),
  diagnostics: feedbackDiagnosticsSchema,
});
export type FeedbackInfo = z.infer<typeof feedbackInfoSchema>;

/** `feedback_submit`'s answer. */
export const feedbackReceiptSchema = z.object({ id: z.string() });
export type FeedbackReceipt = z.infer<typeof feedbackReceiptSchema>;

/** Why a report did not go out (the shell's `SendError` wire names). */
export const FEEDBACK_ERRORS = [
  "not_configured",
  "invalid",
  "rate_limited",
  "unauthorized",
  "network",
  "timeout",
  "server",
] as const;
export type FeedbackError = (typeof FEEDBACK_ERRORS)[number];

/** The dialog's report; `locale` is the language the webview resolved. */
export interface FeedbackDraft {
  kind: FeedbackKind;
  message: string;
  contact: string | null;
  locale: string;
}

export const APP_LICENSE = "Apache-2.0";

/** `voltip_core::DEFAULT_EDIT_HOTKEY`: the voice-edit chord (docs/dictation.md §19). */
export const DEFAULT_EDIT_HOTKEY = "Ctrl+Alt+E";

/** `voltip_core::history::MIN_KEEP`: the fewest entries the retention may ask for. */
export const HISTORY_MIN_KEEP = 10;
/** History recording and retention (`voltip_core::HistorySettings`, docs/dictation.md §4). */
export const historySettingsSchema = z.object({
  /** Record takes; off keeps nothing new. */
  enabled: z.boolean().default(true),
  /** Newest entries kept (10–500). */
  keep: z.number().int().min(10).max(500).default(500),
});
export type HistorySettings = z.infer<typeof historySettingsSchema>;

/** Where the dictation pill appears (`voltip_core::OverlayPlacement`); the desktop shell places the
 *  window, `off` shows none. */
export const OVERLAY_PLACEMENTS = ["bottom", "top", "off"] as const;
export const overlayPlacementSchema = z.enum(OVERLAY_PLACEMENTS);
export type OverlayPlacement = z.infer<typeof overlayPlacementSchema>;

/** The lone-key trigger's keys (`voltip_platform::solo_key::SoloKey`, docs/dictation.md §13.1), in
 *  the order the settings page lists them. `fn` exists only on macOS. */
export const SOLO_KEYS = [
  "right_ctrl",
  "right_alt",
  "right_shift",
  "right_meta",
  "fn",
  "mouse_middle",
  "mouse_back",
  "mouse_forward",
] as const;
export const soloKeySchema = z.enum(SOLO_KEYS);
export type SoloKey = z.infer<typeof soloKeySchema>;

export const settingsSchema = z.object({
  schema: z.literal(1),
  theme: themeIdSchema,
  follow_system_theme: z.boolean(),
  relay_url: z.string().optional(),
  relay_enabled: z.boolean(),
  /** Global dictation hotkey in display form (`Ctrl+Alt+Space`), validated by the core. */
  hotkey: z.string(),
  engines: engineSettingsSchema.default(defaultEngineSettings),
  locale: localeSettingSchema.default("system"),
  /** Check for updates on launch (and install silently when the build has an update source). */
  auto_update: z.boolean().default(false),
  /** Activation (docs/dictation.md §13). All three are `#[serde(default)]` on the Rust side and
   *  always serialised, so an older `settings.json` or core reads as hold / 300 / 0. */
  activation: activationSchema.default("hold"),
  /** `hold_or_toggle`: a release before this many ms of holding locks the take. */
  hold_threshold_ms: z.number().int().nonnegative().default(DEFAULT_HOLD_THRESHOLD_MS),
  /** Keep recording this long after the stop edge (0 = stop at once). */
  extra_recording_ms: z.number().int().nonnegative().default(0),
  /** docs/dictation.md §18.5; an older `settings.json` or core reads as app name on, title off. */
  context_sharing: contextSharingSchema.default(defaultContextSharing),
  /** Voice edit (docs/dictation.md §19): the chord that rewrites the selected text by a spoken
   *  instruction; `null` = off. Always serialised; an older `settings.json` reads as the default. */
  edit_hotkey: z.string().nullable().default(DEFAULT_EDIT_HOTKEY),
  /** The lone-key trigger (docs/dictation.md §13.1), next to `hotkey`; `null` = off. Always
   *  serialised; an older `settings.json` or core reads as off. */
  solo_key: soloKeySchema.nullable().default(null),
  history: historySettingsSchema.default(() => ({ enabled: true, keep: 500 })),
  overlay: overlayPlacementSchema.default("bottom"),
});
export type Settings = z.infer<typeof settingsSchema>;

/** `voltip_core::DEFAULT_HOTKEY`. */
export const DEFAULT_HOTKEY = "Ctrl+Alt+Space";

// ---- dictation pipeline (docs/dictation.md §2–§4) ------------------------------------------

/** Where the text went: `Ctrl+V` into the front app, or only onto the clipboard. */
export const viaSchema = z.enum(["paste", "clipboard"]);
export type Via = z.infer<typeof viaSchema>;

/** `finalizing` (docs/dictation.md §12): a streaming mode waiting for the recogniser's final text. */
export const PROCESSING_STAGES = ["transcribing", "finalizing", "refining", "inserting"] as const;
export const processingStageSchema = z.enum(PROCESSING_STAGES);
export type ProcessingStage = z.infer<typeof processingStageSchema>;

/** `voltip_core::dictation::FailureCode`: which stage of the pipeline failed. The five voice-edit
 *  codes (docs/dictation.md §19.4): nothing selected, a selection over `MAX_EDIT_SELECTION_CHARS`,
 *  the selection could not be read, no LLM configured, a terminal in front (the copy chord would
 *  interrupt it, §19.2). */
export const DICTATION_FAILURE_CODES = [
  "no_speech",
  "audio",
  "asr",
  "refine",
  "inject",
  "no_selection",
  "selection_too_long",
  "selection",
  "edit_unavailable",
  "edit_in_terminal",
  "unknown",
] as const;
export const dictationFailureCodeSchema = z.enum(DICTATION_FAILURE_CODES);
export type DictationFailureCode = z.infer<typeof dictationFailureCodeSchema>;

/** One sentence the streaming recogniser closed at an endpoint (`voltip_core::dictation::Segment`),
 *  with stream timestamps in milliseconds. */
export const liveSegmentSchema = z.object({
  text: z.string(),
  start_ms: z.number().nonnegative(),
  end_ms: z.number().nonnegative(),
});
export type LiveSegment = z.infer<typeof liveSegmentSchema>;

/** The live preview while listening (`voltip_core::dictation::LiveText`, docs/dictation.md §11):
 *  `committed` sentences in the normal colour, `current` dimmed; `degraded` says the preview
 *  stopped following the audio (the final text is unaffected). Every field is `#[serde(default)]`. */
export const liveTextSchema = z.object({
  committed: z.array(liveSegmentSchema).default(() => []),
  current: z.string().default(""),
  degraded: z.string().optional(),
  /** `live_inject` (§12): how many committed sentences were already pasted into the front app. */
  injected: z.number().int().nonnegative().default(0),
});
export type LiveText = z.infer<typeof liveTextSchema>;

/** `voltip_core::dictation::DictationPhase` (`#[serde(tag = "phase")]`). `listening.ready` and
 *  `listening.locked` are `#[serde(default)]` (false = the device has not delivered samples yet /
 *  the take is not locked) and always on the wire; `live` and `processing.preview` are skip-if-none,
 *  so they are simply absent when there is no preview. `done.mode` and `cancelled.injected_chars`
 *  are `#[serde(default)]` too (a core from before §12 reads as a whole take with nothing pasted). */
export const dictationPhaseSchema = z.discriminatedUnion("phase", [
  z.object({ phase: z.literal("idle") }),
  z.object({
    phase: z.literal("listening"),
    /** Re-taken when the device delivered its first samples, so the timer excludes the start-up. */
    started_at: z.number(),
    ready: z.boolean().default(false),
    live: liveTextSchema.optional(),
    /** `hold_or_toggle` (docs/dictation.md §13): a short press locked the take; the next press stops it. */
    locked: z.boolean().default(false),
  }),
  z.object({
    phase: z.literal("processing"),
    stage: processingStageSchema,
    started_at: z.number(),
    /** `committed + current` carried over from listening, shown until the final text arrives. */
    preview: z.string().optional(),
  }),
  z.object({
    phase: z.literal("done"),
    text: z.string(),
    raw_text: z.string(),
    chars: z.number().int().nonnegative(),
    via: viaSchema,
    refined: z.boolean(),
    duration_ms: z.number().nonnegative(),
    asr_ms: z.number().nonnegative(),
    refine_ms: z.number().nonnegative().optional(),
    refine_error: z.string().optional(),
    /** Where the final text came from (§12); a streaming take that degraded reads `whole_take`
     *  here with the reason in `live_error`. */
    mode: outputModeSchema.default("whole_take"),
    /** The streaming sentences (the tail as the last one) in the streaming modes. */
    segments: z.array(liveSegmentSchema).optional(),
    /** Why the streaming path was abandoned or cut short, when a streaming mode was asked for. */
    live_error: z.string().optional(),
  }),
  z.object({
    phase: z.literal("failed"),
    message: z.string(),
    /** Which stage failed; the UI localises the code and falls back to `message`. */
    code: dictationFailureCodeSchema.optional(),
    text: z.string().optional(),
  }),
  z.object({
    phase: z.literal("cancelled"),
    /** `live_inject` (§12): characters already pasted before the cancel; they are not taken back. */
    injected_chars: z.number().int().nonnegative().default(0),
  }),
]);
export type DictationPhase = z.infer<typeof dictationPhaseSchema>;
export type DictationPhaseName = DictationPhase["phase"];

/** What a take is for (`voltip_core::TakeKind`, docs/dictation.md §19): dictating text, or
 *  rewriting the text selected in the foreground application by a spoken instruction. */
export const TAKE_KINDS = ["dictation", "edit"] as const;
export const takeKindSchema = z.enum(TAKE_KINDS);
export type TakeKind = z.infer<typeof takeKindSchema>;

/** `voltip_core::MAX_EDIT_SELECTION_CHARS`: a longer selection is refused (`selection_too_long`). */
export const MAX_EDIT_SELECTION_CHARS = 2000;

// ---- scenes and context (docs/dictation.md §18) -----------------------------------------------

/** `voltip_core::scenes` limits: the core refuses a draft or a list past these. */
export const MAX_SCENES = 50;
export const MAX_SCENE_NAME_CHARS = 32;
export const MAX_SCENE_APPS = 20;
export const MAX_APP_ID_CHARS = 128;
export const MAX_TITLE_KEYWORDS = 10;
export const MAX_TITLE_KEYWORD_CHARS = 64;
export const MAX_SCENE_PROMPT_CHARS = 500;
export const MAX_LANGUAGE_CHARS = 16;
export const MAX_RECENT_APPS = 20;
/** The probe's app name and window title are cut to these (`…`) before anything uses them. */
export const MAX_CONTEXT_NAME_CHARS = 64;
export const MAX_CONTEXT_TITLE_CHARS = 200;
/** `SceneOverrides.language` meaning "no language hint for this take" (auto-detect). */
export const LANGUAGE_AUTO = "auto";

/** Which applications (normalised ids) and, optionally, which window titles a scene applies to. */
export const sceneMatchSchema = z.object({
  apps: z.array(z.string()),
  title_contains: z.array(z.string()).default(() => []),
});
export type SceneMatch = z.infer<typeof sceneMatchSchema>;

/** What a scene changes for one take (`SceneOverrides`); absent / `null` follows the global
 *  setting. The core omits unset ones (`skip_serializing_if`); drafts may send `null`. */
export const sceneOverridesSchema = z.object({
  refine_enabled: z.boolean().nullable().optional(),
  refine_style: refineStyleSchema.nullable().optional(),
  output_mode: outputModeSchema.nullable().optional(),
  /** `auto` (no hint) or a language code (`zh`, `en`, `yue`). */
  language: z.string().nullable().optional(),
  chinese_script: chineseScriptSchema.nullable().optional(),
  /** Extra instruction for the LLM (≤ `MAX_SCENE_PROMPT_CHARS`, newlines allowed). */
  prompt: z.string().nullable().optional(),
});
export type SceneOverrides = z.infer<typeof sceneOverridesSchema>;

/** One scene (`voltip_core::scenes::Scene`, `UiState.scenes`, in matching order). */
export const sceneSchema = z.object({
  id: z.string(),
  name: z.string(),
  enabled: z.boolean(),
  match: sceneMatchSchema,
  overrides: sceneOverridesSchema.default(() => ({})),
  created_at_ms: z.number().nonnegative(),
  updated_at_ms: z.number().nonnegative(),
});
export type Scene = z.infer<typeof sceneSchema>;

/** What `scenes_add` / `scenes_update` send (`SceneDraft`, snake_case inside, `match` on the wire). */
export const sceneDraftSchema = z.object({
  name: z.string(),
  enabled: z.boolean(),
  match: sceneMatchSchema,
  overrides: sceneOverridesSchema,
});
export type SceneDraft = z.infer<typeof sceneDraftSchema>;

/** An application as the status, the history and `recent_apps` name it (`AppRef`). */
export const appRefSchema = z.object({ id: z.string(), name: z.string() });
export type AppRef = z.infer<typeof appRefSchema>;

/** A scene as the status and the history name it (`SceneRef`; its name at the time). */
export const sceneRefSchema = z.object({ id: z.string(), name: z.string() });
export type SceneRef = z.infer<typeof sceneRefSchema>;

/** The take's context (`TakeContext`): the app in front when it started and the matched scene. */
export const takeContextSchema = z.object({
  app: appRefSchema,
  scene: sceneRefSchema.optional(),
});
export type TakeContext = z.infer<typeof takeContextSchema>;

/** `voltip_core::dictation::DictationStatus`: the phase plus a session counter the UI uses to
 *  drop notifications that belong to an earlier run, the take's context once the foreground probe
 *  answered (§18.6; absent on shells without a probe), and the kind of the current (or last) take
 *  (§19; always serialised, a core from before voice edit reads as `dictation`). */
export const dictationStatusSchema = z.object({
  session: z.number().int().nonnegative(),
  phase: dictationPhaseSchema,
  context: takeContextSchema.optional(),
  kind: takeKindSchema.default("dictation"),
  /** The paired phone the take's audio comes from (docs/dictation.md §20), by name. */
  remote: z.string().optional(),
});
export type DictationStatus = z.infer<typeof dictationStatusSchema>;

/** The core's resting dictation state (also what a state without the field parses to). */
export function idleDictation(): DictationStatus {
  return { session: 0, phase: { phase: "idle" }, kind: "dictation" };
}

/** `voltip_core::history::Outcome`. */
export const historyOutcomeSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("inserted"), via: viaSchema }),
  z.object({ kind: z.literal("clipboard"), reason: z.string() }),
  z.object({ kind: z.literal("failed"), reason: z.string() }),
]);
export type HistoryOutcome = z.infer<typeof historyOutcomeSchema>;

// ---- personal dictionary and replacement rules (docs/dictation.md §16) -------------------------

/** `voltip_core::vocabulary` limits: the core refuses a draft, a list or a text past these. */
export const MAX_DICTIONARY_ENTRIES = 500;
export const MAX_TERM_CHARS = 64;
export const MAX_HEARD_AS = 10;
export const MAX_RULES = 200;
export const MAX_RULE_NAME_CHARS = 64;
export const MAX_PATTERN_CHARS = 256;
export const MAX_REPLACEMENT_CHARS = 256;
/** Texts over this many UTF-8 bytes are passed through unchanged (and a preview is refused). */
export const MAX_TEXT_BYTES = 64 * 1024;
/** `rules_import` refuses a TOML text past this many bytes. */
export const MAX_TOML_BYTES = 256 * 1024;

/** Where a dictionary entry came from (`voltip_core::vocabulary::EntrySource`, `tag = "kind"`). */
export const entrySourceSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("manual") }),
  /** Added from a history row (「加入词典」); the row may have been deleted since. */
  z.object({ kind: z.literal("history"), history_id: z.string() }),
]);
export type EntrySource = z.infer<typeof entrySourceSchema>;

/** One dictionary entry (`DictionaryEntry`, `UiState.dictionary`, in dictionary order): the right
 *  spelling and the mis-hearings replaced by it right after recognition. */
export const dictionaryEntrySchema = z.object({
  id: z.string(),
  term: z.string(),
  heard_as: z.array(z.string()).default(() => []),
  /** Off: neither corrected nor sent as a glossary term. */
  enabled: z.boolean(),
  source: entrySourceSchema,
  created_at_ms: z.number().nonnegative(),
  updated_at_ms: z.number().nonnegative(),
});
export type DictionaryEntry = z.infer<typeof dictionaryEntrySchema>;

/** What `dictionary_add` / `dictionary_update` send (`DictionaryDraft`, snake_case inside). */
export const dictionaryDraftSchema = z.object({
  term: z.string(),
  heard_as: z.array(z.string()),
  enabled: z.boolean(),
});
export type DictionaryDraft = z.infer<typeof dictionaryDraftSchema>;

/** `literal` (plain text, word boundaries for spaced scripts) or `regex` (the Rust `regex`
 *  dialect; the replacement may use `$1` / `${name}` / `$$`). */
export const RULE_KINDS = ["literal", "regex"] as const;
export const ruleKindSchema = z.enum(RULE_KINDS);
export type RuleKind = z.infer<typeof ruleKindSchema>;

/** One replacement rule (`ReplacementRule`, `UiState.rules`, in execution order). */
export const replacementRuleSchema = z.object({
  id: z.string(),
  name: z.string(),
  kind: ruleKindSchema,
  pattern: z.string(),
  replacement: z.string().default(""),
  /** Off: ASCII letters match regardless of case. */
  case_sensitive: z.boolean(),
  enabled: z.boolean(),
  created_at_ms: z.number().nonnegative(),
  updated_at_ms: z.number().nonnegative(),
});
export type ReplacementRule = z.infer<typeof replacementRuleSchema>;

/** What `rules_add` / `rules_update` send, and one `[[rule]]` table of the TOML format. */
export const ruleDraftSchema = z.object({
  name: z.string(),
  kind: ruleKindSchema,
  pattern: z.string(),
  replacement: z.string(),
  case_sensitive: z.boolean(),
  enabled: z.boolean(),
});
export type RuleDraft = z.infer<typeof ruleDraftSchema>;

/** `rules_import`: `replace` makes the file the whole list; `merge` updates same-name rules in
 *  place and appends the rest. */
export const IMPORT_MODES = ["replace", "merge"] as const;
export const importModeSchema = z.enum(IMPORT_MODES);
export type ImportMode = z.infer<typeof importModeSchema>;

/** How often one entry / rule fired (`VocabularyHit`); a preview's unsaved draft rule is the nil id. */
export const vocabularyHitSchema = z.object({
  id: z.string(),
  count: z.number().int().nonnegative(),
});
export type VocabularyHit = z.infer<typeof vocabularyHitSchema>;

/** Which corrections and rules fired in one take (`HistoryEntry.vocabulary`). */
export const vocabularyHitsSchema = z.object({
  corrections: z.array(vocabularyHitSchema).default(() => []),
  rules: z.array(vocabularyHitSchema).default(() => []),
});
export type VocabularyHits = z.infer<typeof vocabularyHitsSchema>;

/** `vocabulary_preview`'s stand-in rule: replaces the rule with that `id`, or is appended when
 *  `id` is absent / `null` (a new rule). */
export const previewDraftSchema = z.object({
  id: z.string().nullable().optional(),
  rule: ruleDraftSchema,
});
export type PreviewDraft = z.infer<typeof previewDraftSchema>;

/** What `vocabulary_preview` answers: the text after the dictionary, then after the rules (no
 *  LLM); `error` says a step fell back to its input, as the pipeline would. */
export const vocabularyPreviewSchema = z.object({
  corrected: z.string(),
  output: z.string(),
  corrections: z.array(vocabularyHitSchema),
  rules: z.array(vocabularyHitSchema),
  error: z.string().optional(),
});
export type VocabularyPreview = z.infer<typeof vocabularyPreviewSchema>;

/** The nil UUID: the id of a preview's unsaved draft rule in `VocabularyPreview.rules`. */
export const NIL_ID = "00000000-0000-0000-0000-000000000000";

/** What a voice edit worked on (`voltip_core::EditRecord`, docs/dictation.md §19.5): the
 *  instruction as sent to the LLM (after the dictionary corrections) and the selection it rewrote.
 *  The result is the entry's `text`, the instruction as recognised its `raw_text`. */
export const editRecordSchema = z.object({
  instruction: z.string(),
  selection: z.string(),
});
export type EditRecord = z.infer<typeof editRecordSchema>;

/** One finished dictation as persisted in `history.json` (`voltip_core::history::HistoryEntry`). */
/** What a phone sent (`voltip_core::OriginKind`, docs/dictation.md §20). */
export const ORIGIN_KINDS = ["take", "typed", "clipboard"] as const;
/** Which phone a history entry came from, and how (`voltip_core::EntryOrigin`). */
export const entryOriginSchema = z.object({ device: z.string(), kind: z.enum(ORIGIN_KINDS) });
export type EntryOrigin = z.infer<typeof entryOriginSchema>;

export const historyEntrySchema = z.object({
  id: z.string(),
  at_ms: z.number().nonnegative(),
  raw_text: z.string(),
  text: z.string(),
  refined: z.boolean(),
  asr_model: z.string(),
  refine_model: z.string().optional(),
  duration_ms: z.number().nonnegative(),
  asr_ms: z.number().nonnegative(),
  refine_ms: z.number().nonnegative().optional(),
  outcome: historyOutcomeSchema,
  starred: z.boolean(),
  /** Same meaning as `done.mode` / `segments` / `live_error` (§12); an older `history.json` reads
   *  every row as a whole take. */
  mode: outputModeSchema.default("whole_take"),
  segments: z.array(liveSegmentSchema).optional(),
  live_error: z.string().optional(),
  /** Dictionary corrections and rules that fired (§16); absent when none did. */
  vocabulary: vocabularyHitsSchema.optional(),
  /** Dictation or voice edit (§19.5; always serialised, an older `history.json` reads as
   *  `dictation`); `edit` only on a voice edit. */
  kind: takeKindSchema.default("dictation"),
  edit: editRecordSchema.optional(),
  /** The app in front when the take started and the scene it ran with (§18.6); absent when the
   *  probe did not answer (or on the phone), and in rows written before scenes. */
  app: appRefSchema.optional(),
  scene: sceneRefSchema.optional(),
  /** A phone's take or text rather than this device's own (docs/dictation.md §20.6). */
  origin: entryOriginSchema.optional(),
});
export type HistoryEntry = z.infer<typeof historyEntrySchema>;

/** `voltip_core::history::MAX_ENTRIES`: older rows are dropped past this. */
export const HISTORY_LIMIT = 500;

/** What the UI may know about a stored secret: whether one is set and who set it. The value
 *  itself never crosses the IPC boundary. */
export const secretStateSchema = z.object({
  set: z.boolean(),
  source: z.enum(["builtin", "user", "none"]),
});
export type SecretState = z.infer<typeof secretStateSchema>;

/** One service of a provider as the engines pane shows it (`voltip_core::engines::ServiceStatus`). */
export const serviceStatusSchema = z.object({
  /** Model in effect (`""` when none is chosen); the catalogue id for the on-device provider. */
  model: z.string(),
  /** Suggested models, default first. */
  presets: z.array(z.string()),
  /** The endpoint requests go to; absent for the built-in and on-device providers. */
  base_url: z.string().optional(),
  /** The vendor's public endpoint (the URL field's placeholder); absent for the custom one. */
  default_base_url: z.string().optional(),
  key: secretStateSchema,
  /** Why it cannot run; absent = ready. */
  issue: engineIssueSchema.optional(),
  /** The provider in use for this service. */
  active: z.boolean(),
});
export type ServiceStatus = z.infer<typeof serviceStatusSchema>;

/** One provider card (`voltip_core::engines::ProviderStatus`). */
export const providerStatusSchema = z.object({
  id: providerIdSchema,
  key: keyPolicySchema,
  on_device: z.boolean(),
  /** The desktop can open the vendor's key page (`provider_console_open`). */
  console: z.boolean(),
  asr: serviceStatusSchema.optional(),
  llm: serviceStatusSchema.optional(),
});
export type ProviderStatus = z.infer<typeof providerStatusSchema>;

/** `voltip_core::engines::EngineStatus` (`UiState.engines`): the resolved configuration. Provider
 *  ids, models, the hosts of endpoints the user chose and key presence — never a key, and never
 *  the built-in service's host (`asr_host` / `refine_host` are `""` for it and on-device). */
export const engineStatusSchema = z.object({
  asr_provider: providerIdSchema.default("builtin"),
  /** Recognition can run; the home button is disabled until it can. */
  asr_ready: z.boolean().default(false),
  asr_issue: engineIssueSchema.optional(),
  /** Recognition model (the local model's display name on-device). */
  asr_model: z.string().default(""),
  asr_host: z.string().default(""),
  /** Catalogue id of the resolved local model; absent for remote recognition. */
  local_model: z.string().nullable().optional(),
  /** On-device and every model file present and checksummed. */
  local_ready: z.boolean().default(false),
  /** `live_preview` is on and the streaming model is installed (independent of the provider). */
  live_preview_ready: z.boolean().default(false),
  /** The mode the next `dictation_start` really runs (§12): `output_mode`, or `whole_take` when a
   *  streaming mode was asked for but the streaming model is not ready. */
  effective_output_mode: outputModeSchema.default("whole_take"),
  language: z.string().optional(),
  refine_enabled: z.boolean().default(true),
  /** The clean-up provider chosen; absent in a build without the built-in one until the user picks. */
  llm_provider: providerIdSchema.optional(),
  /** The clean-up can run (whatever `refine_enabled` says). */
  refine_ready: z.boolean().default(false),
  refine_issue: engineIssueSchema.optional(),
  refine_model: z.string().default(""),
  refine_host: z.string().default(""),
  inject: injectModeSchema.default("paste"),
  /** Every provider this build offers, in display order. */
  providers: z.array(providerStatusSchema).default([]),
});
export type EngineStatus = z.infer<typeof engineStatusSchema>;

/** Before the core reported its engines (browser / mock mode, or the first milliseconds): nothing
 *  resolved, nothing set. Mirrors `EngineStatus::default()` on the Rust side. */
export function emptyEngineStatus(): EngineStatus {
  return {
    asr_provider: "builtin",
    asr_ready: false,
    asr_model: "",
    asr_host: "",
    local_ready: false,
    live_preview_ready: false,
    effective_output_mode: "whole_take",
    refine_enabled: true,
    refine_ready: false,
    refine_model: "",
    refine_host: "",
    inject: "paste",
    providers: [],
  };
}

/** Whether dictation can start with the resolved recognition (the one readiness rule the home
 *  page, the title-bar readout and the engines pane share). */
export function engineReady(status: EngineStatus): boolean {
  return status.asr_ready;
}

/** Whether the core has reported its engines yet (the status starts empty). */
export function enginesReported(status: EngineStatus): boolean {
  return status.providers.length > 0;
}

/** The provider card of `id`, if this build offers it. */
export function providerStatus(status: EngineStatus, id: ProviderId): ProviderStatus | undefined {
  return status.providers.find((p) => p.id === id);
}

/** Why a provider probe failed (`voltip_core::providers::ProbeFailure`). */
export const PROBE_FAILURES = [
  "unsupported",
  "invalid_url",
  "key_missing",
  "unauthorized",
  "unreachable",
  "timeout",
  "http_status",
  "bad_response",
] as const;
export const probeFailureSchema = z.enum(PROBE_FAILURES);
export type ProbeFailure = z.infer<typeof probeFailureSchema>;

/** `UiEvent::ProviderProbe` without its tag: the answer to one `provider_probe`. Flat on the wire
 *  (`result: ok` with `models` + `latency_ms`, or `result: failed` with `reason` [+ `status`]). */
export const probeReportSchema = z.object({
  provider: providerIdSchema,
  kind: serviceKindSchema,
  result: z.enum(["ok", "failed"]),
  models: z.array(z.string()).optional(),
  latency_ms: z.number().int().nonnegative().optional(),
  reason: probeFailureSchema.optional(),
  status: z.number().int().optional(),
});
export type ProbeReport = z.infer<typeof probeReportSchema>;

// ---- local models (docs/dictation.md §10) -------------------------------------------------

/** `voltip_asr_local::catalogue` engines: which recogniser a model runs on — transcribe.cpp for
 *  the Qwen3-ASR GGUF tiers, sherpa-onnx for the light tiers and the streaming preview; `silero_vad`
 *  is the auxiliary voice-activity model behind `vad_trim` (§12). */
export const MODEL_ENGINES = [
  "transcribe_cpp",
  "sense_voice",
  "paraformer",
  "zipformer_streaming",
  "silero_vad",
] as const;
export const modelEngineSchema = z.enum(MODEL_ENGINES);
export type ModelEngine = z.infer<typeof modelEngineSchema>;

/** Product tiers the settings dialog groups by (docs/dictation.md §10), in display order; the
 *  streaming tier is the live-preview model, not a recognition choice. `auxiliary` is the hidden
 *  `silero-vad` entry (§12): the core keeps it out of `UiState.models`, the wire still names it. */
export const MODEL_TIERS = ["balanced", "accurate", "light", "streaming", "auxiliary"] as const;
export const modelTierSchema = z.enum(MODEL_TIERS);
export type ModelTier = z.infer<typeof modelTierSchema>;

/** What a model can do: whole-take recognition (`offline`), partial results (`streaming`), or
 *  voice-activity detection (`vad`, the auxiliary model that never shows up as a card). */
export const MODEL_CAPABILITIES = ["offline", "streaming", "vad"] as const;
export const modelCapabilitySchema = z.enum(MODEL_CAPABILITIES);
export type ModelCapability = z.infer<typeof modelCapabilitySchema>;

/** `voltip_core::models::ModelInstallState` (`#[serde(tag = "kind")]`). */
export const modelInstallStateSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("not_installed") }),
  z.object({
    kind: z.literal("downloading"),
    received: z.number().nonnegative(),
    total: z.number().nonnegative(),
    /** The file currently streaming (`model.int8.onnx`). */
    file: z.string(),
  }),
  z.object({ kind: z.literal("verifying") }),
  z.object({
    kind: z.literal("installed"),
    path: z.string(),
    /** Unix seconds. */
    installed_at: z.number().nonnegative(),
  }),
  /** The `.part` files stay on disk; `model_download` resumes. */
  z.object({ kind: z.literal("failed"), message: z.string() }),
]);
export type ModelInstallState = z.infer<typeof modelInstallStateSchema>;
export type ModelInstallKind = ModelInstallState["kind"];

/** One catalogue row plus its install state (`voltip_core::models::ModelState`, `UiState.models`). */
export const modelStateSchema = z.object({
  id: z.string(),
  /** Display name in the core's words (`均衡`); the UI prefers its dictionary by id under `en`. */
  name: z.string(),
  engine: modelEngineSchema,
  tier: modelTierSchema,
  /** `#[serde(default)]` on the Rust side: an empty list when a core did not report any. */
  capabilities: z.array(modelCapabilitySchema).default(() => []),
  languages: z.array(z.string()),
  size_bytes: z.number().nonnegative(),
  description: z.string(),
  recommended: z.boolean(),
  /** Where the files come from (`owner/name` on Hugging Face). */
  repo: z.string().default(""),
  /** The model `Settings.engines` currently selects. */
  active: z.boolean(),
  state: modelInstallStateSchema,
});
export type ModelState = z.infer<typeof modelStateSchema>;

/** A model the user can pick for recognition (the streaming preview model is not one). */
export function isRecognitionModel(model: Pick<ModelState, "capabilities">): boolean {
  return model.capabilities.includes("offline");
}

/** The model that feeds the live preview (`capabilities: ["streaming"]`). */
export function isStreamingModel(model: Pick<ModelState, "capabilities">): boolean {
  return model.capabilities.includes("streaming");
}

export const deviceIdentityPublicSchema = z.object({
  device_id: z.string(),
  name: z.string(),
  platform: platformSchema,
  public_key: hexKeySchema,
  fingerprint: z.string(),
});
export type DeviceIdentityPublic = z.infer<typeof deviceIdentityPublicSchema>;

/** Where the relay in use comes from (`voltip_core::RelaySource`). */
export const relaySourceSchema = z.enum(["none", "builtin", "user"]);
export type RelaySource = z.infer<typeof relaySourceSchema>;

export const relayStatusSchema = z.object({
  /** The relay the user entered; absent for the build's own relay (never named) and for none. */
  endpoint: z.string().optional(),
  source: relaySourceSchema.default("none"),
  state: connectionStateSchema,
  attempts: z.number().int().nonnegative(),
});
export type RelayStatus = z.infer<typeof relayStatusSchema>;

export const failureReasonSchema = z.union([
  z.object({
    kind: z.enum([
      "timeout",
      "replay",
      "handshake",
      "protocol",
      "cancelled",
      "peer_left",
      "identity_changed",
    ]),
  }),
  z.object({ kind: z.literal("relay"), code: z.string() }),
]);
export type FailureReason = z.infer<typeof failureReasonSchema>;

export const PAIRING_PHASES = [
  "idle",
  "creating_session",
  "waiting_for_peer",
  "key_exchange",
  "awaiting_verification",
  "trusted",
  "expired",
  "rejected",
] as const;

export const pairingStateSchema = z.union([
  z.object({ state: z.enum(PAIRING_PHASES) }),
  z.object({ state: z.literal("failed"), reason: failureReasonSchema }),
]);
export type PairingState = z.infer<typeof pairingStateSchema>;

export const safetyCodeSchema = z.object({
  words: z.tuple([z.string(), z.string(), z.string(), z.string()]),
  fingerprint: z.string(),
});
export type SafetyCode = z.infer<typeof safetyCodeSchema>;

export const deviceInfoSchema = z.object({
  device_id: z.string(),
  name: z.string(),
  platform: platformSchema,
});
export type DeviceInfo = z.infer<typeof deviceInfoSchema>;

export const snapshotSchema = z.object({
  state: pairingStateSchema,
  session_id: z.string().optional(),
  code: z.string().optional(),
  ticket_uri: z.string().optional(),
  expires_at: z.number().optional(),
  remaining_secs: z.number().optional(),
  safety_code: safetyCodeSchema.optional(),
  peer: deviceInfoSchema.optional(),
  local_confirmed: z.boolean(),
  peer_confirmed: z.boolean(),
});
export type Snapshot = z.infer<typeof snapshotSchema>;

export const trustedDeviceSchema = z.object({
  device_id: z.string(),
  name: z.string(),
  platform: platformSchema,
  public_key: hexKeySchema,
  fingerprint: z.string(),
  trusted_at: z.number(),
  last_seen: z.number().optional(),
  last_connection: connectionKindSchema.optional(),
  /** Last known `ip:port` endpoints of the peer's LAN host; omitted by the core when empty. */
  direct_hints: z.array(z.string()).optional(),
});
export type TrustedDevice = z.infer<typeof trustedDeviceSchema>;

export const deviceConnectionSchema = z.union([
  z.object({ state: z.enum(["offline", "connecting"]) }),
  z.object({ state: z.literal("online"), via: connectionKindSchema }),
  z.object({ state: z.literal("identity_changed"), presented_fingerprint: z.string() }),
]);
export type DeviceConnection = z.infer<typeof deviceConnectionSchema>;

export const deviceViewSchema = z.object({
  device: trustedDeviceSchema,
  connection: deviceConnectionSchema,
});
export type DeviceView = z.infer<typeof deviceViewSchema>;

/** What the hotkey can do in the session the desktop runs in (`voltip_core::ui::HotkeyCapabilities`). */
export const hotkeyCapabilitiesSchema = z.object({
  /** The shell can register a global chord here (not on a pure Wayland session). */
  global: z.boolean(),
  /** The chord fires whichever window has the focus (under XWayland only X11 windows). */
  everywhere: z.boolean(),
  /** Presses and releases both arrive, so press-and-hold activation works. */
  hold: z.boolean(),
  /** What a desktop or compositor shortcut runs to start and stop a take. */
  toggle_command: z.string(),
  /** The same for voice edit. */
  edit_toggle_command: z.string(),
  /** The lone keys the shell can watch here (§13.1); empty without an input hook (pure Wayland). */
  solo_keys: z.array(soloKeySchema).default(() => []),
});
export type HotkeyCapabilities = z.infer<typeof hotkeyCapabilitiesSchema>;

/** What the desktop shell reports about the OS-level hotkey registration (`voltip_core::ui::HotkeyStatus`). */
export const hotkeyStatusSchema = z.object({
  registered: z.string().optional(),
  error: z.string().optional(),
  pressed: z.boolean(),
  /** The recorder is open: the shell suspended the OS registration until it closes. */
  capturing: z.boolean().optional(),
  backend: z.string(),
  /** The voice-edit chord (docs/dictation.md §19): registered, or why not (a conflict, pure
   *  Wayland, the same chord as dictation). Both absent while it is off. */
  edit_registered: z.string().optional(),
  edit_error: z.string().optional(),
  /** Rust always sends it (all `false` and empty until the shell reports); absent in old payloads. */
  capabilities: hotkeyCapabilitiesSchema.optional(),
  /** The lone-key trigger (docs/dictation.md §13.1): the key the input hook watches, or why the
   *  chosen one is not watched; both absent while it is off. */
  solo_registered: soloKeySchema.optional(),
  solo_error: z.string().optional(),
  /** The lone key is held down on its own (Rust always sends it; absent in old payloads). */
  solo_pressed: z.boolean().optional(),
});
export type HotkeyStatus = z.infer<typeof hotkeyStatusSchema>;

/** Before the shell has reported anything (browser / mock mode, or the first milliseconds). */
export function emptyHotkeyStatus(): HotkeyStatus {
  return { pressed: false, capturing: false, backend: "" };
}

// ---- auto-update (docs/frontend.md §7) --------------------------------------------------

// ---- platform queries (docs/dictation.md §15; `voltip_platform`) ----

/** `voltip_platform::HostOs`. */
export const hostOsSchema = z.enum(["macos", "windows", "linux", "other"]);
export type HostOs = z.infer<typeof hostOsSchema>;

/** `voltip_platform::Permission`. */
export const PERMISSIONS = ["microphone", "accessibility"] as const;
export const permissionSchema = z.enum(PERMISSIONS);
export type Permission = z.infer<typeof permissionSchema>;

/** `voltip_platform::PermissionState`. */
export const PERMISSION_STATES = ["granted", "denied", "not_determined", "not_applicable"] as const;
export const permissionStateSchema = z.enum(PERMISSION_STATES);
export type PermissionState = z.infer<typeof permissionStateSchema>;

/** `voltip_platform::PermissionReport`: the `permissions_status` answer. */
export const permissionReportSchema = z.object({
  platform: hostOsSchema,
  microphone: permissionStateSchema,
  accessibility: permissionStateSchema,
});
export type PermissionReport = z.infer<typeof permissionReportSchema>;

/** Every permission `not_applicable`: what Linux and the phone answer. */
export function notApplicablePermissions(platform: HostOs): PermissionReport {
  return {
    platform,
    microphone: "not_applicable",
    accessibility: "not_applicable",
  };
}

/** `true` when the platform gates nothing (the onboarding step says so and moves on). */
export function nothingToGrant(report: PermissionReport): boolean {
  return PERMISSIONS.every((p) => report[p] === "not_applicable");
}

/** Mirror of `voltip_platform::onboarding_gate`: the permissions that block "continue".
 *  Microphone blocks only when denied (the OS prompts on first use otherwise); Accessibility
 *  blocks when denied or never asked (macOS never prompts by itself); Input Monitoring never
 *  blocks (it only unlocks the "hold a modifier alone" trigger). */
export function onboardingGate(report: PermissionReport): Permission[] {
  return PERMISSIONS.filter((p) => {
    const state = report[p];
    if (state === "granted" || state === "not_applicable") return false;
    if (p === "microphone") return state === "denied";
    if (p === "accessibility") return true;
    return false;
  });
}

/** Mirror of `voltip_platform::PollPlan::DEFAULT`: re-read every second, stop after three
 *  consecutive failed queries. */
export const PERMISSION_POLL_INTERVAL_MS = 1000;
export const PERMISSION_POLL_MAX_ERRORS = 3;

/** `voltip_platform::IntegrityLevel` (Windows token integrity, ordered low → high). */
export const integrityLevelSchema = z.enum([
  "untrusted",
  "low",
  "medium",
  "medium_plus",
  "high",
  "system",
  "protected_process",
]);
export type IntegrityLevel = z.infer<typeof integrityLevelSchema>;

/** `voltip_platform::InjectDecision`. */
export const INJECT_DECISIONS = [
  "proceed",
  "elevated_target",
  "secure_desktop",
  "unknown",
] as const;
export const injectDecisionSchema = z.enum(INJECT_DECISIONS);
export type InjectDecision = z.infer<typeof injectDecisionSchema>;

/** `voltip_platform::InjectPreflight`: the `inject_preflight` answer. `checked` is `false` on
 *  hosts without UIPI (macOS, Linux), where the decision is always `proceed`. */
export const injectPreflightSchema = z.object({
  platform: hostOsSchema,
  checked: z.boolean(),
  decision: injectDecisionSchema,
  target_process: z.string().nullable(),
  self_level: integrityLevelSchema.nullable(),
  target_level: integrityLevelSchema.nullable(),
});
export type InjectPreflight = z.infer<typeof injectPreflightSchema>;

/** The unchecked `proceed` answer of a host without UIPI. */
export function uncheckedPreflight(platform: HostOs): InjectPreflight {
  return {
    platform,
    checked: false,
    decision: "proceed",
    target_process: null,
    self_level: null,
    target_level: null,
  };
}

/** `voltip_core::update::UpdateStatus` (`#[serde(tag = "state")]`). */
export const updateStatusSchema = z.discriminatedUnion("state", [
  z.object({ state: z.literal("idle") }),
  z.object({ state: z.literal("checking") }),
  z.object({ state: z.literal("up_to_date"), version: z.string(), checked_at: z.number() }),
  z.object({
    state: z.literal("available"),
    version: z.string(),
    current: z.string(),
    notes: z.string().optional(),
    date: z.string().optional(),
  }),
  z.object({
    state: z.literal("downloading"),
    version: z.string(),
    received: z.number().nonnegative(),
    total: z.number().nonnegative().optional(),
  }),
  z.object({ state: z.literal("ready"), version: z.string() }),
  z.object({ state: z.literal("installing"), version: z.string() }),
  z.object({ state: z.literal("failed"), message: z.string() }),
  /** This build was packaged without an update endpoint / public key. */
  z.object({ state: z.literal("disabled") }),
]);
export type UpdateStatus = z.infer<typeof updateStatusSchema>;
export type UpdateState = UpdateStatus["state"];

/** Before the core reported anything about updates (and what a state without the field parses to). */
export function idleUpdate(): UpdateStatus {
  return { state: "idle" };
}

/** `UiEvent { type: "update", ...UpdateStatus }`: the status flattened next to `type`, one object
 *  per variant so both discriminators (`type`, `state`) stay literal. */
const UPDATE_TYPE = z.literal("update");
const updateEventSchema = z.discriminatedUnion("state", [
  updateStatusSchema.options[0].extend({ type: UPDATE_TYPE }),
  updateStatusSchema.options[1].extend({ type: UPDATE_TYPE }),
  updateStatusSchema.options[2].extend({ type: UPDATE_TYPE }),
  updateStatusSchema.options[3].extend({ type: UPDATE_TYPE }),
  updateStatusSchema.options[4].extend({ type: UPDATE_TYPE }),
  updateStatusSchema.options[5].extend({ type: UPDATE_TYPE }),
  updateStatusSchema.options[6].extend({ type: UPDATE_TYPE }),
  updateStatusSchema.options[7].extend({ type: UPDATE_TYPE }),
  updateStatusSchema.options[8].extend({ type: UPDATE_TYPE }),
]);

/** Why a phone's take ended without delivering (`voltip_core::phone::PhoneTakeFailure`): the
 *  first four come from the desktop, `microphone` / `offline` are the phone's own. */
export const PHONE_TAKE_FAILURES = [
  "busy",
  "unavailable",
  "no_speech",
  "failed",
  "microphone",
  "offline",
] as const;
export const phoneTakeFailureSchema = z.enum(PHONE_TAKE_FAILURES);
export type PhoneTakeFailure = z.infer<typeof phoneTakeFailureSchema>;

/** Where the phone's take is (`voltip_core::phone::PhoneTakeState`, docs/dictation.md §20). */
export const phoneTakeStateSchema = z.discriminatedUnion("state", [
  z.object({ state: z.literal("starting") }),
  z.object({ state: z.literal("listening") }),
  z.object({ state: z.literal("processing") }),
  z.object({ state: z.literal("done"), text: z.string(), pasted: z.boolean() }),
  z.object({ state: z.literal("failed"), code: phoneTakeFailureSchema, message: z.string() }),
  z.object({ state: z.literal("cancelled") }),
]);
export type PhoneTakeState = z.infer<typeof phoneTakeStateSchema>;

/** The phone's current or last take to a desktop (`UiState.phone_take`). */
export const phoneTakeViewSchema = z.object({
  /** The desktop (hex public key, as `DeviceView.device.public_key`). */
  device: z.string(),
  take: z.number().int().nonnegative(),
  /** When the phone started it (Unix ms). */
  started_at: z.number().int().nonnegative(),
  state: phoneTakeStateSchema,
  /** The audio goes out as Opus (docs/dictation.md §20.1); absent before it = PCM. */
  opus: z.boolean().optional(),
});
export type PhoneTakeView = z.infer<typeof phoneTakeViewSchema>;

/** Where a phone's text came from (`voltip_protocol::app::PhoneTextSource`, §20.6). */
export const PHONE_TEXT_SOURCES = ["typed", "clipboard"] as const;
export type PhoneTextSource = (typeof PHONE_TEXT_SOURCES)[number];
/** `voltip_core::phone::MAX_PHONE_TEXT_CHARS`. */
export const MAX_PHONE_TEXT_CHARS = 10_000;
/** Why a sent text did not arrive (`SentTextFailure`); the last two are the phone's own. */
export const SENT_TEXT_FAILURES = [
  "busy",
  "unavailable",
  "failed",
  "offline",
  "no_answer",
] as const;
export const sentTextStateSchema = z.discriminatedUnion("state", [
  z.object({ state: z.literal("sending") }),
  z.object({ state: z.literal("queued") }),
  z.object({ state: z.literal("delivered"), pasted: z.boolean() }),
  z.object({ state: z.literal("failed"), code: z.enum(SENT_TEXT_FAILURES), message: z.string() }),
]);
export type SentTextState = z.infer<typeof sentTextStateSchema>;
/** A text the phone sent to a desktop (`voltip_core::phone::SentText`, docs/dictation.md §20.6). */
export const sentTextSchema = z.object({
  id: z.number().int().nonnegative(),
  /** The desktop (hex public key). */
  device: z.string(),
  device_name: z.string(),
  body: z.string(),
  source: z.enum(PHONE_TEXT_SOURCES),
  /** Unix ms. */
  sent_at: z.number().int().nonnegative(),
  state: sentTextStateSchema,
});
export type SentText = z.infer<typeof sentTextSchema>;

/** No further answer is expected for this text. */
export function sentTextFinal(state: SentTextState): boolean {
  return state.state === "delivered" || state.state === "failed";
}

/** The take is over (delivered, refused, cancelled). */
export function phoneTakeFinal(state: PhoneTakeState): boolean {
  return state.state === "done" || state.state === "failed" || state.state === "cancelled";
}

/** A GPU the local engines can run on (`voltip_core::ui::GpuDevice`, docs/dictation.md §10.6). */
export const gpuDeviceSchema = z.object({
  /** Backend device name (`Metal`, `Vulkan0`): what `EngineSettings.local_gpu` stores. */
  name: z.string(),
  description: z.string(),
  kind: z.string(),
  memory_mb: z.number().int().nonnegative().default(0),
  integrated: z.boolean().default(false),
});
export type GpuDevice = z.infer<typeof gpuDeviceSchema>;

/** What the local models can run on (`voltip_core::ui::HardwareStatus`); shell-owned, empty until
 *  the desktop reports and always on the phone. */
export const hardwareStatusSchema = z.object({
  cpu_threads: z.number().int().nonnegative().default(0),
  gpus: z.array(gpuDeviceSchema).default(() => []),
});
export type HardwareStatus = z.infer<typeof hardwareStatusSchema>;

// ---- connectivity self-check (docs/pairing.md; `voltip_core::connectivity`) -------------

/** One probe: a Voltip relay or LAN host answered `hello`, or why not. */
export const probeResultSchema = z.discriminatedUnion("result", [
  z.object({ result: z.literal("ok"), ms: z.number().int().nonnegative() }),
  z.object({ result: z.literal("timeout") }),
  z.object({ result: z.literal("refused") }),
  z.object({ result: z.literal("failed"), reason: z.string() }),
]);
export type ProbeResult = z.infer<typeof probeResultSchema>;

export const addressCheckSchema = z.object({
  address: z.string(),
  /** On this device's own IPv4 /24: a failure points at a firewall or Wi-Fi client isolation. */
  same_subnet: z.boolean(),
  result: probeResultSchema,
});
export type AddressCheck = z.infer<typeof addressCheckSchema>;

export const peerCheckSchema = z.object({
  public_key: hexKeySchema,
  name: z.string(),
  /** How the live channel runs; absent while the device is offline. */
  via: connectionKindSchema.optional(),
  /** Round trip of an encrypted ping on that channel. */
  rtt_ms: z.number().int().nonnegative().optional(),
  addresses: z.array(addressCheckSchema),
});
export type PeerCheck = z.infer<typeof peerCheckSchema>;

export const connectivityReportSchema = z.object({
  /** Unix milliseconds. */
  checked_at: z.number(),
  lan: z.object({ listening: z.boolean(), addresses: z.array(z.string()) }),
  relay: z.object({ configured: z.boolean(), result: probeResultSchema.optional() }),
  peers: z.array(peerCheckSchema),
});
export type ConnectivityReport = z.infer<typeof connectivityReportSchema>;

export const connectivityStatusSchema = z.object({
  running: z.boolean(),
  report: connectivityReportSchema.optional(),
});
export type ConnectivityStatus = z.infer<typeof connectivityStatusSchema>;

export const uiStateSchema = z.object({
  identity: deviceIdentityPublicSchema.nullable(),
  settings: settingsSchema,
  secret_backend: z.string(),
  /** The app version (`""` until the core is ready). */
  app_version: z.string().default(""),
  relay: relayStatusSchema,
  pairing: snapshotSchema,
  devices: z.array(deviceViewSchema),
  hotkey: hotkeyStatusSchema.default(emptyHotkeyStatus),
  dictation: dictationStatusSchema.default(idleDictation),
  history: z.array(historyEntrySchema).default(() => []),
  engines: engineStatusSchema.default(emptyEngineStatus),
  update: updateStatusSchema.default(idleUpdate),
  /** The local model catalogue with install states; `[]` on the phone (no local models there). */
  models: z.array(modelStateSchema).default(() => []),
  /** The personal dictionary and the replacement rules (§16), in order; `[]` on the phone. */
  dictionary: z.array(dictionaryEntrySchema).default(() => []),
  rules: z.array(replacementRuleSchema).default(() => []),
  /** The scenes (§18), in matching order; `[]` on the phone. */
  scenes: z.array(sceneSchema).default(() => []),
  /** The phone's current or last take streamed to a desktop (§20); absent on the desktop. */
  phone_take: phoneTakeViewSchema.optional(),
  /** The texts this phone sent (§20.6), newest first; always empty on the desktop. */
  sent_texts: z.array(sentTextSchema).default(() => []),
  /** What the local models can run on (§10.6); empty until the desktop shell reports. */
  hardware: hardwareStatusSchema.default(() => ({ cpu_threads: 0, gpus: [] })),
  /** The connectivity self-check: running, and the last report. */
  connectivity: connectivityStatusSchema.default(() => ({ running: false })),
});
export type UiState = z.infer<typeof uiStateSchema>;

// serde internally-tagged newtype variants flatten struct payloads next to `type`; the one
// array payload (`devices`) is wrapped by the Rust side as `{ type: "devices", devices: [...] }`.
export const uiEventSchema = z.discriminatedUnion("type", [
  uiStateSchema.extend({ type: z.literal("state") }),
  deviceIdentityPublicSchema.extend({ type: z.literal("identity") }),
  settingsSchema.extend({ type: z.literal("settings") }),
  relayStatusSchema.extend({ type: z.literal("relay") }),
  snapshotSchema.extend({ type: z.literal("pairing") }),
  z.object({ type: z.literal("devices"), devices: z.array(deviceViewSchema) }),
  trustedDeviceSchema.extend({ type: z.literal("trusted") }),
  /** A trusted device unpaired this one; the next `devices` event no longer lists it. */
  trustedDeviceSchema.extend({ type: z.literal("unpaired") }),
  /** The connectivity self-check started (`running`) or finished (`report`). */
  connectivityStatusSchema.extend({ type: z.literal("connectivity") }),
  z.object({
    type: z.literal("identity_changed"),
    previous: trustedDeviceSchema,
    presented_fingerprint: z.string(),
  }),
  z.object({ type: z.literal("message"), from: z.string(), body: z.string() }),
  z.object({ type: z.literal("error"), message: z.string() }),
  hotkeyStatusSchema.extend({ type: z.literal("hotkey") }),
  dictationStatusSchema.extend({ type: z.literal("dictation") }),
  z.object({ type: z.literal("history"), entries: z.array(historyEntrySchema) }),
  engineStatusSchema.extend({ type: z.literal("engines") }),
  probeReportSchema.extend({ type: z.literal("provider_probe") }),
  updateEventSchema,
  /** Full catalogue push; download progress arrives folded into the list (≥ 250 ms / 1 MiB apart). */
  z.object({ type: z.literal("models"), models: z.array(modelStateSchema) }),
  /** The whole dictionary / rule list after every change (§16). */
  z.object({ type: z.literal("dictionary"), entries: z.array(dictionaryEntrySchema) }),
  z.object({ type: z.literal("rules"), rules: z.array(replacementRuleSchema) }),
  /** The whole scene list after every change (§18). */
  z.object({ type: z.literal("scenes"), scenes: z.array(sceneSchema) }),
  /** The phone's take to a desktop moved (§20); `null` before the first. */
  z.object({ type: z.literal("phone_take"), take: phoneTakeViewSchema.nullable() }),
  /** The phone's list of sent texts, whole (§20.6). */
  z.object({ type: z.literal("sent_texts"), texts: z.array(sentTextSchema) }),
  /** What the local models can run on (§10.6), reported once by the desktop shell. */
  hardwareStatusSchema.extend({ type: z.literal("hardware") }),
]);
export type UiEvent = z.infer<typeof uiEventSchema>;
export type UiEventType = UiEvent["type"];

/** Tauri event channel name (`voltip_core::ui::UI_EVENT_NAME`). */
export const UI_EVENT_NAME = "voltip://event";

/** A microphone as `voltip-audio` enumerates it (cpal: WASAPI / CoreAudio / ALSA). */
export const audioDeviceSchema = z.object({
  /** Stable handle (the backend's device name). */
  id: z.string(),
  name: z.string(),
  is_default: z.boolean(),
  sample_rate_hz: z.number().int().positive().optional(),
  channels: z.number().int().positive().optional(),
});
export type AudioDevice = z.infer<typeof audioDeviceSchema>;

/** One level-meter frame streamed from Rust through a Tauri `Channel` (≈ 30 Hz). */
export const levelFrameSchema = z.object({
  rms_dbfs: z.number(),
  peak_dbfs: z.number(),
  clipping: z.boolean(),
  sample_rate_hz: z.number().int().nonnegative(),
  channels: z.number().int().nonnegative(),
  seq: z.number().int().nonnegative(),
});
export type LevelFrame = z.infer<typeof levelFrameSchema>;

/** Who produced a hotkey edge (`voltip_core::dictation::activation::EdgeSource`). */
export const EDGE_SOURCES = ["hotkey", "cli", "ui"] as const;
export type EdgeSource = (typeof EDGE_SOURCES)[number];

/** `hotkey_edge` arguments: `atMs` is Unix milliseconds (`voltip_core::now_ms()`); the optional
 *  fields default on the Rust side (now / `ui` / `dictation` / not chorded); `purpose: "edit"` is
 *  the voice-edit key (docs/dictation.md §19); `chorded` is the desktop input hook's report that
 *  another key joined the held lone-key trigger (§13.1). Declared apart from `CommandArgs` so the
 *  Rust contract test, which reads that interface line by line, sees one key per line. */
export type HotkeyEdgeArgs = {
  pressed: boolean;
  atMs?: number;
  source?: EdgeSource;
  purpose?: TakeKind;
  chorded?: boolean;
};

/** `settings_set_activation` arguments (docs/dictation.md §13). */
export type SetActivationArgs = {
  activation: Activation;
  holdThresholdMs: number;
  extraRecordingMs: number;
};

/** `dictionary_add` arguments: `historyId` names the history row the entry comes from. */
export type DictionaryAddArgs = { entry: DictionaryDraft; historyId?: string | null };

/** `vocabulary_preview` arguments: the text, and optionally one rule draft standing in. */
export type VocabularyPreviewArgs = { text: string; draft?: PreviewDraft | null };

/** `scenes_update` arguments (docs/dictation.md §18.6). */
export type ScenesUpdateArgs = { id: string; scene: SceneDraft };

/** `settings_set_context_sharing` arguments: both switches together. */
export type SetContextSharingArgs = { appName: boolean; windowTitle: boolean };

/** Tauri command name → argument object (camelCase; Tauri maps to snake_case Rust params). */
export interface CommandArgs {
  core_state: undefined;
  pairing_start: undefined;
  pairing_join_code: { code: string };
  pairing_join_ticket: { uri: string };
  pairing_confirm: undefined;
  pairing_reject: undefined;
  pairing_cancel: undefined;
  pairing_reset: undefined;
  device_forget: { publicKey: string };
  device_rename: { name: string };
  send_text: { publicKey: string; body: string };
  /** Phone: stream a take to this paired desktop, which records, recognises and delivers it. */
  phone_take_start: { publicKey: string };
  /** Phone: the speaker let go; the desktop delivers. */
  phone_take_stop: undefined;
  /** Phone: discard the take on both ends. */
  phone_take_cancel: undefined;
  settings_set_relay: { url: string | null; enabled: boolean };
  settings_set_theme: { theme: ThemeId; followSystem: boolean };
  settings_set_hotkey: { hotkey: string };
  /** Voice edit (docs/dictation.md §19): the edit chord, or `null` to switch the key off. */
  settings_set_edit_hotkey: { hotkey: string | null };
  /** The lone-key trigger (docs/dictation.md §13.1), or `null` to switch it off. */
  settings_set_solo_key: { key: SoloKey | null };
  /** Recorder open (`true`): the shell suspends the OS hotkey so the chord reaches the webview. */
  hotkey_capture: { active: boolean };
  devices_refresh: undefined;
  connectivity_check: undefined;
  /** Query: microphones known to the native audio backend (`Backend.audioDevices`). */
  audio_devices: undefined;
  /** Stream: subscribe to the native level meter; frames arrive on the `onFrame` Channel, the
   *  command returns the subscription id (`Backend.meter`). */
  audio_meter_start: { deviceId: string | null };
  /** Remove one meter subscription (the id `audio_meter_start` returned). */
  audio_meter_stop: { id: number };
  /** Query: the pill state the shell wants shown right now (`useOverlayWindowState` pulls it on mount). */
  overlay_state: undefined;
  /** Dictation (docs/dictation.md §5): start recording, stop and run the pipeline, or discard. */
  dictation_start: undefined;
  dictation_stop: undefined;
  dictation_cancel: undefined;
  /** The whole `Settings.engines` block; the core re-resolves and re-emits `engines`. */
  settings_set_engines: { engines: EngineSettings };
  /** Store (`value`) or delete (`null`) the user's key for a provider's service (a vendor's two
   *  services share one key); the value is never read back. */
  provider_key_set: { provider: ProviderId; kind: ServiceKind; value: string | null };
  /** List a provider's models with the form's values (`null` = the saved ones); answered by a
   *  `provider_probe` event. The key is used for this request only. */
  provider_probe: {
    provider: ProviderId;
    kind: ServiceKind;
    baseUrl?: string | null;
    key?: string | null;
  };
  /** Open the vendor's API-key page in the browser (catalogue URLs only). */
  provider_console_open: { provider: ProviderId };
  /** Open a project page in the browser; the shell builds the URL from its repository. */
  project_link_open: { link: ProjectLink };
  /** Query (docs/feedback.md): what a report would carry, and whether the build can send one. */
  feedback_diagnostics: { locale: string };
  /** Query: post the 反馈 dialog's report; rejects with a `FeedbackError` wire name. */
  feedback_submit: FeedbackDraft;
  /** Phone (docs/dictation.md §20.6): send text for the desktop to insert at its cursor. */
  phone_text_send: { publicKey: string; body: string; source: PhoneTextSource };
  /** Phone: forget the list of sent texts. */
  sent_texts_clear: undefined;
  /** Query (phone): the phone's clipboard text, `null` when it holds none. */
  phone_clipboard_read: undefined;
  history_delete: { id: string };
  history_clear: undefined;
  history_star: { id: string; starred: boolean };
  /** UI language; the core persists it and re-emits `settings` so every window follows. */
  settings_set_locale: { locale: LocaleSetting };
  settings_set_auto_update: { enabled: boolean };
  /** History recording and retention; a smaller `keep` trims the list at once. */
  settings_set_history: { enabled: boolean; keep: number };
  /** Where the dictation pill appears; the core persists it, the desktop shell follows. */
  settings_set_overlay: { placement: OverlayPlacement };
  /** One press / release edge into the activation machine (docs/dictation.md §13). The UI sends
   *  `pressed` only; the shell fills `atMs` (now) and `source` (`ui`) in. */
  hotkey_edge: HotkeyEdgeArgs;
  /** Activation mode plus its two timings (all three together; the core persists and re-emits
   *  `settings`, or answers with an `error` event above `MAX_ACTIVATION_MS`). */
  settings_set_activation: SetActivationArgs;
  /** Auto-update (docs/frontend.md §7): ask the updater, then download + install. */
  update_check: undefined;
  update_install: undefined;
  /** Query: the updater's current status (`Backend.updateStatus`). */
  update_status: undefined;
  /** Local models (docs/dictation.md §10): fetch / verify a catalogue model, abort the download,
   *  or delete its directory. Activation is `settings_set_engines { asr_provider: "local", local_model }`;
   *  the live preview switch is `settings_set_engines { live_preview }` (§11). */
  model_download: { id: string };
  model_cancel: { id: string };
  model_remove: { id: string };
  /** Personal dictionary (docs/dictation.md §16.4): append, replace, delete, reorder (every id in
   *  the new order). A draft that is wrong on its own rejects the call with the core's message;
   *  a clash with another entry comes back as an `error` event and the list stays as it was. */
  dictionary_add: DictionaryAddArgs;
  dictionary_update: { id: string; entry: DictionaryDraft };
  dictionary_remove: { id: string };
  dictionary_reorder: { ids: string[] };
  /** Replacement rules (§16.4 / §16.5): the same verbs, plus a TOML import (validated whole). */
  rules_add: { rule: RuleDraft };
  rules_update: { id: string; rule: RuleDraft };
  rules_remove: { id: string };
  rules_reorder: { ids: string[] };
  rules_import: { toml: string; mode: ImportMode };
  /** Query: the rules as TOML text (`Backend.rulesExport`). */
  rules_export: undefined;
  /** Query: a text through the dictionary and the rules (`Backend.vocabularyPreview`). */
  vocabulary_preview: VocabularyPreviewArgs;
  /** Scenes (docs/dictation.md §18.6): append, replace, delete, reorder (every id in the new
   *  order). A draft wrong on its own rejects the call with the core's message; a clash with the
   *  list (duplicate name, cap, unknown id) comes back as an `error` event. */
  scenes_add: { scene: SceneDraft };
  scenes_update: ScenesUpdateArgs;
  scenes_remove: { id: string };
  scenes_reorder: { ids: string[] };
  /** Which parts of a take's context may go to the LLM (§18.5); the core re-emits `settings`. */
  settings_set_context_sharing: SetContextSharingArgs;
  /** Query: the apps the history saw, newest first (`Backend.recentApps`, the scene editor). */
  recent_apps: undefined;
  /** Query (docs/dictation.md §15.1): what the OS grants right now (`Backend.permissionsStatus`);
   *  the onboarding step polls it every second while on screen. */
  permissions_status: undefined;
  /** Ask the OS for one permission (`Backend.permissionsRequest`): the macOS prompt / System
   *  Settings pane; an accepted no-op on hosts that gate nothing. Not a `UiCommand`: nothing in the
   *  core changes, so it travels through a `Backend` method like the other queries. */
  permissions_request: { permission: Permission };
  /** Query (§15.3): would an injection into the foreground window land (`Backend.injectPreflight`). */
  inject_preflight: undefined;
}
export type CommandName = keyof CommandArgs;
/** Queries and streams: called through dedicated `Backend` methods, never through `invoke`. */
export type QueryCommand =
  | "core_state"
  | "audio_devices"
  | "audio_meter_start"
  | "audio_meter_stop"
  | "overlay_state"
  | "update_status"
  | "rules_export"
  | "vocabulary_preview"
  | "recent_apps"
  | "permissions_status"
  | "permissions_request"
  | "inject_preflight"
  | "provider_console_open"
  | "project_link_open"
  | "feedback_diagnostics"
  | "feedback_submit"
  | "phone_clipboard_read";
export const QUERY_COMMANDS: readonly QueryCommand[] = [
  "core_state",
  "audio_devices",
  "audio_meter_start",
  "audio_meter_stop",
  "overlay_state",
  "update_status",
  "rules_export",
  "vocabulary_preview",
  "recent_apps",
  "permissions_status",
  "permissions_request",
  "inject_preflight",
  "provider_console_open",
  "project_link_open",
  "feedback_diagnostics",
  "feedback_submit",
  "phone_clipboard_read",
];
/** Commands the UI dispatches through `Backend.invoke` (everything except the queries / streams). */
export type MutationCommand = Exclude<CommandName, QueryCommand>;
export type ArgsOf<C extends CommandName> = CommandArgs[C] extends undefined
  ? []
  : [args: CommandArgs[C]];

/** The idle pairing snapshot the core starts with. */
export function idleSnapshot(): Snapshot {
  return { state: { state: "idle" }, local_confirmed: false, peer_confirmed: false };
}

/** Defaults mirroring `voltip_core::Settings::default()`. */
export function defaultSettings(): Settings {
  return {
    schema: 1,
    theme: "light",
    follow_system_theme: false,
    relay_enabled: true,
    hotkey: DEFAULT_HOTKEY,
    engines: defaultEngineSettings(),
    locale: "system",
    auto_update: false,
    activation: "hold",
    hold_threshold_ms: DEFAULT_HOLD_THRESHOLD_MS,
    extra_recording_ms: 0,
    context_sharing: defaultContextSharing(),
    edit_hotkey: DEFAULT_EDIT_HOTKEY,
    solo_key: null,
    history: { enabled: true, keep: 500 },
    overlay: "bottom",
  };
}

/** Fold a validated event into a cached state, exactly as `UiState::apply` does on the Rust side. */
export function applyEvent(state: UiState, event: UiEvent): UiState {
  switch (event.type) {
    case "state": {
      const { type: _type, ...rest } = event;
      return rest;
    }
    case "identity": {
      const { type: _type, ...identity } = event;
      return { ...state, identity };
    }
    case "settings": {
      const { type: _type, ...settings } = event;
      return { ...state, settings };
    }
    case "relay": {
      const { type: _type, ...relay } = event;
      return { ...state, relay };
    }
    case "pairing": {
      const { type: _type, ...pairing } = event;
      return { ...state, pairing };
    }
    case "devices":
      return { ...state, devices: event.devices };
    case "hotkey": {
      const { type: _type, ...hotkey } = event;
      return { ...state, hotkey };
    }
    case "dictation": {
      const { type: _type, ...dictation } = event;
      return { ...state, dictation };
    }
    case "history":
      return { ...state, history: event.entries };
    case "engines": {
      const { type: _type, ...engines } = event;
      return { ...state, engines };
    }
    case "update": {
      const { type: _type, ...update } = event;
      return { ...state, update };
    }
    case "models":
      return { ...state, models: event.models };
    case "dictionary":
      return { ...state, dictionary: event.entries };
    case "rules":
      return { ...state, rules: event.rules };
    case "scenes":
      return { ...state, scenes: event.scenes };
    case "hardware": {
      const { type: _type, ...hardware } = event;
      return { ...state, hardware };
    }
    case "connectivity": {
      const { type: _type, ...connectivity } = event;
      return { ...state, connectivity };
    }
    case "phone_take": {
      if (event.take === null) {
        const { phone_take: _gone, ...rest } = state;
        return rest;
      }
      return { ...state, phone_take: event.take };
    }
    case "sent_texts":
      return { ...state, sent_texts: event.texts };
    case "trusted":
    case "unpaired":
    case "identity_changed":
    case "message":
    case "error":
    case "provider_probe":
      return state;
  }
}
