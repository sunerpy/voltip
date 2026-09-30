// Human-readable labels (R-UI-5): toolbar readouts and tables show aliases; model ids stay
// secondary mono text. Everything here is pure; the words come from the i18n dictionaries and every
// helper takes the locale last, defaulting to Simplified Chinese so callers without a provider (and
// the existing tests) keep their wording.
import { type Locale, DEFAULT_LOCALE, type MessageKey, translate } from "./i18n";
import { isBuiltinPreset } from "./schema";
import type {
  Activation,
  BuiltinPreset,
  BuiltinScene,
  ConnectionState,
  CustomPreset,
  DeviceConnection,
  DictationPhase,
  DictationStatus,
  FailureReason,
  HistoryOutcome,
  LiveText,
  ModelTier,
  OutputMode,
  PairingState,
  Platform,
  PresetId,
  PresetRef,
  ProcessingStage,
  RecordingSource,
  RelayStatus,
  SecretState,
  SegmentProgress,
  SystemAudio,
  TakeKind,
  ThemeId,
  Via,
} from "./schema";

export const THEME_NAMES: Readonly<Record<ThemeId, string>> = {
  light: "明亮",
  dark: "暗黑",
  warm: "暖纸",
  graphite: "石墨",
};

export const THEME_SUBTITLES: Readonly<Record<ThemeId, string>> = {
  light: "白瓷",
  dark: "夜灯",
  warm: "手稿",
  graphite: "仪表",
};

export function themeName(theme: ThemeId, locale: Locale = DEFAULT_LOCALE): string {
  return translate(locale, `theme.name.${theme}`);
}

export function themeSubtitle(theme: ThemeId, locale: Locale = DEFAULT_LOCALE): string {
  return translate(locale, `theme.subtitle.${theme}`);
}

export const PLATFORM_LABELS: Readonly<Record<Platform, string>> = {
  windows: "Windows",
  macos: "macOS",
  linux: "Linux",
  android: "Android",
  ios: "iOS",
  other: "其他",
};

export function platformLabel(platform: Platform, locale: Locale = DEFAULT_LOCALE): string {
  return platform === "other" ? translate(locale, "platform.other") : PLATFORM_LABELS[platform];
}

export type Tone = "ok" | "danger" | "warn" | "accent" | "neutral" | "idle";

export interface Labelled {
  text: string;
  tone: Tone;
}

export function connectionLabel(
  connection: DeviceConnection,
  locale: Locale = DEFAULT_LOCALE,
): Labelled {
  const t = (key: MessageKey) => translate(locale, key);
  switch (connection.state) {
    case "online":
      return {
        text:
          connection.via === "direct" ? t("connection.onlineDirect") : t("connection.onlineRelay"),
        tone: "ok",
      };
    case "connecting":
      return { text: t("connection.connecting"), tone: "accent" };
    case "offline":
      return { text: t("connection.offline"), tone: "idle" };
    case "identity_changed":
      return { text: t("connection.identityChanged"), tone: "danger" };
  }
}

export function connectionKindLabel(
  kind: "direct" | "relay" | undefined,
  locale: Locale = DEFAULT_LOCALE,
): string {
  if (kind === "direct") return translate(locale, "connection.direct");
  if (kind === "relay") return translate(locale, "connection.relay");
  return "—";
}

function connectionStateText(state: ConnectionState, locale: Locale): string {
  return translate(locale, `relay.${state}`);
}

/** Toolbar readout for the relay link, e.g. `Relay ● 已连接` or `重连中 · 第 3 次`. */
export function relayLabel(relay: RelayStatus, locale: Locale = DEFAULT_LOCALE): Labelled {
  const base = connectionStateText(relay.state, locale);
  switch (relay.state) {
    case "connected":
      return { text: base, tone: "ok" };
    case "connecting":
    case "authenticating":
      return { text: base, tone: "accent" };
    case "reconnecting":
      return {
        text:
          relay.attempts > 0
            ? translate(locale, "relay.attempt", { base, n: relay.attempts })
            : base,
        tone: "warn",
      };
    case "disconnected":
      return {
        text: relay.source === "none" ? translate(locale, "relay.unconfigured") : base,
        tone: "idle",
      };
    case "closed":
      return { text: base, tone: "danger" };
  }
}

const RELAY_CODES = new Set([
  "invalid_code",
  "session_expired",
  "session_full",
  "rate_limited",
  "invalid_channel",
  "channel_full",
  "unsupported_version",
  "not_joined",
]);

function isRelayCode(
  code: string,
): code is
  | "invalid_code"
  | "session_expired"
  | "session_full"
  | "rate_limited"
  | "invalid_channel"
  | "channel_full"
  | "unsupported_version"
  | "not_joined" {
  return RELAY_CODES.has(code);
}

export function failureLabel(reason: FailureReason, locale: Locale = DEFAULT_LOCALE): string {
  if (reason.kind === "relay") {
    return isRelayCode(reason.code)
      ? translate(locale, `failure.relayCode.${reason.code}`)
      : translate(locale, "failure.relayRejected", { code: reason.code });
  }
  return translate(locale, `failure.${reason.kind}`);
}

export function pairingStateLabel(state: PairingState, locale: Locale = DEFAULT_LOCALE): Labelled {
  switch (state.state) {
    case "idle":
      return { text: translate(locale, "pairingState.idle"), tone: "idle" };
    case "creating_session":
      return { text: translate(locale, "pairingState.creating_session"), tone: "accent" };
    case "waiting_for_peer":
      return { text: translate(locale, "pairingState.waiting_for_peer"), tone: "ok" };
    case "key_exchange":
      return { text: translate(locale, "pairingState.key_exchange"), tone: "accent" };
    case "awaiting_verification":
      return { text: translate(locale, "pairingState.awaiting_verification"), tone: "accent" };
    case "trusted":
      return { text: translate(locale, "pairingState.trusted"), tone: "ok" };
    case "expired":
      return { text: translate(locale, "pairingState.expired"), tone: "danger" };
    case "rejected":
      return { text: translate(locale, "pairingState.rejected"), tone: "danger" };
    case "failed":
      return {
        text: translate(locale, "pairingState.failed", {
          reason: failureLabel(state.reason, locale),
        }),
        tone: "danger",
      };
  }
}

/** `107` → `01:47`. */
export function formatRemaining(secs: number): string {
  const s = Math.max(0, Math.floor(secs));
  const mm = Math.floor(s / 60)
    .toString()
    .padStart(2, "0");
  const ss = (s % 60).toString().padStart(2, "0");
  return `${mm}:${ss}`;
}

/** `483921` or `483 921` → `483 921`. */
export function formatCode(code: string): string {
  const digits = code.replace(/\D/g, "").slice(0, 6);
  return digits.length > 3 ? `${digits.slice(0, 3)} ${digits.slice(3)}` : digits;
}

/** `A7:C4:19:8E · 3D:F2:61:09` → `A7C4 … 6109`. */
export function shortFingerprint(fingerprint: string): string {
  const hex = fingerprint.replace(/[^0-9A-Fa-f]/g, "").toUpperCase();
  if (hex.length < 8) return fingerprint;
  return `${hex.slice(0, 4)} … ${hex.slice(-4)}`;
}

/** `9f0c2b1e…` → `9f0c…d5e6`. */
export function shortKey(publicKey: string): string {
  return publicKey.length > 12 ? `${publicKey.slice(0, 4)}…${publicKey.slice(-4)}` : publicKey;
}

/** Relative time; `now` and `then` are unix seconds. */
export function relativeTime(
  then: number | undefined,
  now: number,
  locale: Locale = DEFAULT_LOCALE,
): string {
  if (then === undefined) return translate(locale, "time.never");
  const delta = Math.max(0, now - then);
  if (delta < 60) return translate(locale, "time.justNow");
  if (delta < 3600) return translate(locale, "time.minutesAgo", { n: Math.floor(delta / 60) });
  if (delta < 86_400) return translate(locale, "time.hoursAgo", { n: Math.floor(delta / 3600) });
  if (delta < 86_400 * 30)
    return translate(locale, "time.daysAgo", { n: Math.floor(delta / 86_400) });
  return formatDate(then);
}

/** `2026-09-12` in UTC to stay deterministic across machines. */
export function formatDate(unixSecs: number): string {
  const d = new Date(unixSecs * 1000);
  const y = d.getUTCFullYear();
  const m = (d.getUTCMonth() + 1).toString().padStart(2, "0");
  const day = d.getUTCDate().toString().padStart(2, "0");
  return `${y}-${m}-${day}`;
}

/** `1842` → `1,842`. */
export function formatCount(n: number): string {
  return n.toLocaleString("en-US");
}

// ---------------------------------------------------------------------------------------------
// Dictation pipeline labels (docs/dictation.md): the phase line on the home page, the pill and
// the history list all read the same words.

/** `paste` → `粘贴`, `clipboard` → `剪贴板`. */
export function viaLabel(via: Via, locale: Locale = DEFAULT_LOCALE): string {
  return translate(locale, `via.${via}`);
}

/** `12_345` → `00:12` (elapsed listening time); past an hour `3_723_000` → `1:02:03`
 *  (docs/dictation.md §22: a take may run for two hours). */
export function formatElapsed(ms: number): string {
  const secs = Math.floor(Math.max(0, ms) / 1000);
  if (secs < 3600) return formatRemaining(secs);
  const hours = Math.floor(secs / 3600);
  return `${hours}:${formatRemaining(secs % 3600)}`;
}

/** `1384` → `1,384 ms`; `undefined` → `—`. */
export function formatMs(ms: number | undefined): string {
  return ms === undefined ? "—" : `${formatCount(Math.round(ms))} ms`;
}

/** `6800` → `6.8 s`. */
export function formatSeconds(ms: number): string {
  return `${(Math.max(0, ms) / 1000).toFixed(1)} s`;
}

/** A span of time for the statistics, from seconds up to thousands of hours, as numbers and
 *  their units, largest first: seconds alone under a minute, no seconds from an hour on, and a
 *  part that is zero left out (`3 分 47 秒`, `1 小时`, `63 小时 12 分`). */
export function durationParts(
  ms: number,
  locale: Locale = DEFAULT_LOCALE,
): { value: string; unit: string }[] {
  const total = Math.round(Math.max(0, ms) / 1000);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const part = (value: number, unit: "hours" | "minutes" | "seconds") => ({
    value: formatCount(value),
    unit: translate(locale, `time.duration.${unit}`),
  });
  if (h > 0) return m > 0 ? [part(h, "hours"), part(m, "minutes")] : [part(h, "hours")];
  if (m > 0) return s > 0 ? [part(m, "minutes"), part(s, "seconds")] : [part(m, "minutes")];
  return [part(s, "seconds")];
}

/** [`durationParts`] as text: `21 秒`, `3 分 47 秒`, `63 小时 12 分` (`21 s`, `3 min 47 s`,
 *  `63 h 12 min`). */
export function formatDuration(ms: number, locale: Locale = DEFAULT_LOCALE): string {
  return durationParts(ms, locale)
    .map((p) => `${p.value} ${p.unit}`)
    .join(" ");
}

/** A take's length: `6.8 s` under a minute, `11 分` / `1 小时 2 分` from then on (docs/dictation.md
 *  §22: a take may run for two hours). */
export function formatTakeLength(ms: number, locale: Locale = DEFAULT_LOCALE): string {
  return ms < 60_000 ? formatSeconds(ms) : formatDuration(ms, locale);
}

type FailedPhase = Extract<DictationPhase, { phase: "failed" }>;

/** The reason line of a failed dictation: the localized failure `code` when the core sent one,
 *  otherwise (or for `unknown`) the core's own `message`. `selection` (voice edit, docs/dictation.md
 *  §19) carries the core's reason, without the `selection: ` prefix of the error text. */
export function dictationFailureText(phase: FailedPhase, locale: Locale = DEFAULT_LOCALE): string {
  return failureText(phase, "dictation", locale);
}

/** [`dictationFailureText`] for a take of either kind: a failed rewrite says the selection was left
 *  alone (docs/dictation.md §19.4). */
export function takeFailureText(
  status: { phase: FailedPhase; kind: TakeKind; source?: RecordingSource },
  locale: Locale = DEFAULT_LOCALE,
): string {
  return failureText(status.phase, status.kind, locale, status.source);
}

function failureText(
  phase: FailedPhase,
  kind: TakeKind,
  locale: Locale,
  source?: RecordingSource,
): string {
  const code = phase.code;
  if (code === undefined || code === "unknown") return phase.message;
  if (kind === "edit" && code === "refine") return translate(locale, "dictation.edit.refineFailed");
  // docs/dictation.md §22: what failed to record is not always the microphone.
  if (code === "audio" && (source === "system" || source === "mixed"))
    return translate(locale, `dictation.audioFailure.${source}`);
  const prefix = `${code}: `;
  const reason = phase.message.startsWith(prefix)
    ? phase.message.slice(prefix.length)
    : phase.message;
  return translate(locale, `dictation.failureCode.${code}`, { reason });
}

/** One line for the current dictation phase; `now` (ms) drives the elapsed readout. */
export function dictationPhaseLabel(
  phase: DictationPhase,
  now: number,
  locale: Locale = DEFAULT_LOCALE,
): Labelled {
  return phaseLabel(phase, "dictation", now, locale);
}

/** [`dictationPhaseLabel`] for the core's status (`state.dictation`): a voice edit
 *  (docs/dictation.md §19, `kind: "edit"`) listens for an instruction, rewrites instead of
 *  polishing and reports the replaced text. */
export function takePhaseLabel(
  status: Pick<DictationStatus, "phase" | "kind" | "segments" | "source">,
  now: number,
  locale: Locale = DEFAULT_LOCALE,
): Labelled {
  return phaseLabel(status.phase, status.kind, now, locale, status.segments, status.source);
}

/** A long take's recognition while it records (`已识别 12 段`, docs/dictation.md §22). */
export function segmentsDoneLabel(
  segments: SegmentProgress,
  locale: Locale = DEFAULT_LOCALE,
): string {
  return translate(locale, "dictation.segments.listening", { n: segments.done });
}

/** Why the computer's sound cannot be recorded here (docs/dictation.md §22); `undefined` when it can. */
export function systemAudioNote(
  systemAudio: SystemAudio,
  locale: Locale = DEFAULT_LOCALE,
): string | undefined {
  switch (systemAudio.state) {
    case "available":
      return undefined;
    case "macos_too_old":
      return translate(locale, "settings.microphone.unavailable.macos_too_old", {
        version: systemAudio.version,
      });
    case "no_sound_server":
    case "unsupported":
      return translate(locale, `settings.microphone.unavailable.${systemAudio.state}`);
  }
}

/** `mixed` → 混合 / Mixed: what a take records (docs/dictation.md §22). */
export function recordingSourceLabel(
  source: RecordingSource,
  locale: Locale = DEFAULT_LOCALE,
): string {
  return translate(locale, `recordingSource.name.${source}`);
}

function phaseLabel(
  phase: DictationPhase,
  kind: TakeKind,
  now: number,
  locale: Locale,
  segments?: SegmentProgress,
  source?: RecordingSource,
): Labelled {
  const edit = kind === "edit";
  switch (phase.phase) {
    case "idle":
      return { text: translate(locale, "dictation.idle"), tone: "idle" };
    case "listening": {
      const line = translate(locale, edit ? "dictation.edit.listening" : "dictation.listening", {
        elapsed: formatElapsed(now - phase.started_at),
      });
      return {
        text: segments === undefined ? line : `${line} · ${segmentsDoneLabel(segments, locale)}`,
        tone: "accent",
      };
    }
    case "processing":
      return {
        text:
          edit && phase.stage === "refining"
            ? translate(locale, "dictation.edit.refining")
            : phase.stage === "transcribing" && segments !== undefined
              ? translate(locale, "dictation.segments.processing", {
                  done: segments.done,
                  total: segments.total,
                })
              : processingStageLabel(phase.stage, locale),
        tone: "accent",
      };
    case "done":
      if (edit)
        return {
          text: translate(
            locale,
            phase.via === "paste" ? "dictation.edit.donePaste" : "dictation.edit.doneClipboard",
            { n: phase.chars, via: viaLabel(phase.via, locale) },
          ),
          tone: "ok",
        };
      return {
        text:
          translate(
            locale,
            phase.via === "paste" ? "dictation.donePaste" : "dictation.doneClipboard",
            {
              n: phase.chars,
              via: viaLabel(phase.via, locale),
            },
          ) + (phase.refined ? translate(locale, "dictation.refinedSuffix") : ""),
        tone: "ok",
      };
    case "failed":
      return {
        text: translate(
          locale,
          phase.text === undefined ? "dictation.failed" : "dictation.notInserted",
          { reason: failureText(phase, kind, locale, source) },
        ),
        tone: "danger",
      };
    case "cancelled":
      return { text: translate(locale, "dictation.cancelled"), tone: "idle" };
  }
}

// ---- output modes and activation (docs/dictation.md §12–§13) ------------------------------

/** `whole_take` → 整段输出 / Whole take. */
export function outputModeLabel(mode: OutputMode, locale: Locale = DEFAULT_LOCALE): string {
  return translate(locale, `outputMode.name.${mode}`);
}

/** One sentence on what the mode does (the settings card body). */
export function outputModeDescription(mode: OutputMode, locale: Locale = DEFAULT_LOCALE): string {
  return translate(locale, `outputMode.description.${mode}`);
}

// ---- AI presets (docs/dictation.md §21) ----------------------------------------------------------

/** A preset's name: a built-in one in the interface's language, a custom one by its own name, and a
 *  custom one that was deleted since as 已删除的预设 (its takes use 校对). */
export function presetLabel(
  id: PresetId,
  presets: readonly CustomPreset[],
  locale: Locale = DEFAULT_LOCALE,
): string {
  if (isBuiltinPreset(id)) return translate(locale, `presets.${id}.name`);
  return presets.find((p) => p.id === id)?.name ?? translate(locale, "presets.missing");
}

/** The preset a status or a history row names: a built-in one in the interface's language, a
 *  custom one by the name it had then. */
export function presetRefLabel(ref: PresetRef, locale: Locale = DEFAULT_LOCALE): string {
  return isBuiltinPreset(ref.id) ? translate(locale, `presets.${ref.id}.name`) : ref.name;
}

/** A scene's name (a `Scene` or a `SceneRef`): a built-in one in the interface's language
 *  (docs/dictation.md §18.10), the user's by its own name. */
export function sceneLabel(
  scene: { name: string; builtin?: BuiltinScene },
  locale: Locale = DEFAULT_LOCALE,
): string {
  return scene.builtin === undefined
    ? scene.name
    : translate(locale, `builtinScenes.${scene.builtin}.name`);
}

/** One sentence on what a built-in preset does. */
export function presetDescription(id: BuiltinPreset, locale: Locale = DEFAULT_LOCALE): string {
  return translate(locale, `presets.${id}.description`);
}

/** `hold` → 按住说话 / Hold to talk (the settings card title). */
export function activationLabel(activation: Activation, locale: Locale = DEFAULT_LOCALE): string {
  return translate(locale, `activation.name.${activation}`);
}

/** One sentence on how the mode drives a take (the settings card body). */
export function activationDescription(
  activation: Activation,
  locale: Locale = DEFAULT_LOCALE,
): string {
  return translate(locale, `activation.description.${activation}`);
}

/** The home readiness chip: 按住说话 / 按一下开始 · 再按结束 / 按住说话 · 短按锁定. */
export function activationChip(activation: Activation, locale: Locale = DEFAULT_LOCALE): string {
  return translate(locale, `activation.chip.${activation}`);
}

/** The footer shortcut caption next to the chord: 按住听写 / 按一下听写 / 按住或按一下听写. */
export function activationShortcut(
  activation: Activation,
  locale: Locale = DEFAULT_LOCALE,
): string {
  return translate(locale, `activation.shortcut.${activation}`);
}

/** The one-sentence how-to with the chord spelled out (`Ctrl Alt Space`), e.g. the empty states:
 *  `按住 Ctrl Alt Space 说一句，松开即插入` / `按一下 … 开始，再按一下结束` / `按住 … 说话，短按锁定`. */
export function activationHint(
  activation: Activation,
  hotkey: string,
  locale: Locale = DEFAULT_LOCALE,
): string {
  return translate(locale, `activation.hint.${activation}`, {
    hotkey: hotkey.replaceAll("+", " "),
  });
}

// ---- live preview (docs/dictation.md §11) --------------------------------------------------

/** The CJK ranges `voltip_core::dictation::is_cjk` joins without a space. */
const CJK_CHAR =
  /[\u2E80-\u9FFF\uAC00-\uD7AF\uF900-\uFAFF\uFE30-\uFE4F\uFF00-\uFFEF\u{20000}-\u{3134F}]/u;

/** What goes between two spoken pieces: a space when both boundary characters are Latin (and the
 *  left one is not already whitespace), nothing at a CJK boundary or next to an empty side. */
export function liveTextGap(left: string, right: string): "" | " " {
  const last = Array.from(left).at(-1);
  const first = Array.from(right)[0];
  if (last === undefined || first === undefined) return "";
  return !/\s/.test(last) && !CJK_CHAR.test(last) && !CJK_CHAR.test(first) ? " " : "";
}

/** Join spoken pieces the way `LiveText::preview()` does: trimmed, Latin neighbours separated by a
 *  space, CJK neighbours (including full-width punctuation) joined directly. */
export function joinLiveText(pieces: readonly string[]): string {
  let out = "";
  for (const raw of pieces) {
    const piece = raw.trim();
    if (piece.length === 0) continue;
    out += liveTextGap(out, piece) + piece;
  }
  return out;
}

/** `committed + current` as one string — what `Processing.preview` carries. */
export function livePreviewText(live: Pick<LiveText, "committed" | "current">): string {
  return joinLiveText([...live.committed.map((s) => s.text), live.current]);
}

/** The two tones of the pill's live caption: every committed sentence joined, and the current one. */
export function liveCaptionParts(live: Pick<LiveText, "committed" | "current">): {
  committed: string;
  current: string;
} {
  return {
    committed: joinLiveText(live.committed.map((s) => s.text)),
    current: live.current.trim(),
  };
}

// ---- local models (docs/dictation.md §10) --------------------------------------------------

/** Catalogue id → dictionary sub-key (`model.name.<key>` / `model.description.<key>`): the two
 *  GGUF ids carry a dot, which the dot-joined message paths cannot hold, so they map to `0_6b`. */
const MODEL_KEYS = {
  "qwen3-asr-0.6b": "qwen3-asr-0_6b",
  "qwen3-asr-1.7b": "qwen3-asr-1_7b",
  "sense-voice-small": "sense-voice-small",
  "paraformer-zh": "paraformer-zh",
  "zipformer-stream-zh-en": "zipformer-stream-zh-en",
} as const;
export type ModelDictionaryKey = (typeof MODEL_KEYS)[keyof typeof MODEL_KEYS];

function isCatalogueId(id: string): id is keyof typeof MODEL_KEYS {
  return Object.hasOwn(MODEL_KEYS, id);
}

/** The dictionary key of a catalogue id, `undefined` for a model the dictionary does not know. */
export function modelDictionaryKey(id: string): ModelDictionaryKey | undefined {
  return isCatalogueId(id) ? MODEL_KEYS[id] : undefined;
}

/** A model's display name for the locale: the core sends the Chinese tier names (`均衡`), so
 *  under `zh-CN` the core's `name` is shown as is; under `en` the dictionary's name by id wins,
 *  falling back to the core's name for an id the dictionary does not know. */
export function modelDisplayName(
  id: string,
  name: string,
  locale: Locale = DEFAULT_LOCALE,
): string {
  const key = modelDictionaryKey(id);
  if (locale === "zh-CN" || key === undefined) return name;
  return translate(locale, `model.name.${key}`);
}

/** The core's own catalogue text, localised by id when the dictionary knows the model. */
export function modelDescription(
  id: string,
  description: string,
  locale: Locale = DEFAULT_LOCALE,
): string {
  const key = modelDictionaryKey(id);
  return key === undefined ? description : translate(locale, `model.description.${key}`);
}

/** `balanced` → 均衡 / Balanced. */
export function modelTierLabel(tier: ModelTier, locale: Locale = DEFAULT_LOCALE): string {
  return translate(locale, `model.tier.${tier}`);
}

export function processingStageLabel(
  stage: ProcessingStage,
  locale: Locale = DEFAULT_LOCALE,
): string {
  return translate(locale, `dictation.stage.${stage}`);
}

/** A machine prefix the core and the shell put in front of a refusal (`scenes: `, `phone text: `,
 *  `history.keep: `, `openai.asr_url: `): it names the field for the log, not for a reader. */
const MACHINE_PREFIX = /^[a-z][a-z_]*(?:\.[a-z_]+)*(?: [a-z]+)?: /;

/** A core or shell message as the interface shows it: without its machine prefix
 *  (docs/frontend.md §8). */
export function coreMessageText(message: string): string {
  return message.replace(MACHINE_PREFIX, "");
}

/** The system calls a shortcut backend goes through: implementation, never shown. */
const HOTKEY_SYSTEM_CALLS: ReadonlySet<string> = new Set(["RegisterHotKey", "Carbon"]);

/** The shell's shortcut backend (`global-shortcut · Windows · RegisterHotKey`) in the words a reader
 *  needs: the system and, on Linux, the session (`Windows`, `Linux · Wayland`), which decide what a
 *  shortcut can do. The library and the system call are implementation. Any other text (the browser
 *  preview's mock) is shown as it is. */
export function hotkeyMethodText(backend: string): string {
  const [library, ...rest] = backend.split(" · ");
  if (library !== "global-shortcut") return backend;
  const shown = rest.filter((part) => !HOTKEY_SYSTEM_CALLS.has(part));
  return shown.length > 0 ? shown.join(" · ") : backend;
}

/** History row outcome: `已插入 · 粘贴`, `仅剪贴板 · <reason>`, `失败 · <reason>`. */
export function outcomeLabel(outcome: HistoryOutcome, locale: Locale = DEFAULT_LOCALE): Labelled {
  switch (outcome.kind) {
    case "inserted":
      return {
        text: translate(locale, "outcome.inserted", { via: viaLabel(outcome.via, locale) }),
        tone: "ok",
      };
    case "clipboard":
      // The reason is explained under the entry (docs/dictation.md §4.2), never in the label.
      return { text: translate(locale, "outcome.clipboard"), tone: "warn" };
    case "failed":
      return {
        text: translate(locale, "outcome.failed", { reason: outcome.reason }),
        tone: "danger",
      };
  }
}

/** `{ set: true, source: "builtin" }` → `已内置`; user → `已设置`; none → `未设置`. */
export function secretStateLabel(state: SecretState, locale: Locale = DEFAULT_LOCALE): Labelled {
  if (!state.set) return { text: translate(locale, "secret.none"), tone: "danger" };
  return state.source === "user"
    ? { text: translate(locale, "secret.user"), tone: "ok" }
    : { text: translate(locale, "secret.builtin"), tone: "ok" };
}

/** `https://api.example.com/openai/v1` → `api.example.com`; a bare host or garbage is returned as is. */
export function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url.replace(/^[a-z]+:\/\//i, "").split("/")[0] ?? url;
  }
}
