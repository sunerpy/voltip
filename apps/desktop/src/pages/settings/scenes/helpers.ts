// Pure helpers of the 场景 settings group (docs/dictation.md §18): the editor's form state and its
// conversion to the wire `SceneDraft`, the instant localised checks shown while typing, the select
// options (every override starts at 跟随全局), and the one-line summary on a scene card. The core
// validates the draft again (its refusal comes back as the command's rejection) and checks the
// list (a clash is an `error` event).
import {
  BUILTIN_PRESETS,
  CHINESE_SCRIPTS,
  type ChineseScript,
  type CustomPreset,
  LANGUAGE_AUTO,
  type Locale,
  MAX_SCENE_APPS,
  MAX_SCENE_PROMPT_CHARS,
  MAX_TITLE_KEYWORDS,
  OUTPUT_MODES,
  type OutputMode,
  type PresetId,
  type Scene,
  type SceneDraft,
  type SceneOverrides,
  type TFunction,
  normalizeAppId,
  outputModeLabel,
  presetLabel,
  zhT,
} from "@voltip/shared";
import { LANGUAGE_CODES } from "../engines/helpers";

/** An override that is either unset (`""`, 跟随全局) or switched on / off. */
export type Switch = "" | "on" | "off";

/** The editor's form: the overrides as select values, `""` meaning 跟随全局 (unset). */
export interface EditorDraft {
  name: string;
  enabled: boolean;
  /** Normalised app ids, in the order they were added. */
  apps: string[];
  keywords: string[];
  refine: Switch;
  /** `""` = follow, otherwise a built-in preset's name or a custom preset's id (possibly deleted
   *  since). */
  preset: PresetId;
  outputMode: "" | OutputMode;
  /** `""` = follow, `auto` = auto-detect, otherwise a language code. */
  language: string;
  script: "" | ChineseScript;
  prompt: string;
}

/** The form for `scene`, or an empty one (enabled, every override following the globals). */
export function editorDraftFrom(scene?: Scene): EditorDraft {
  const o: SceneOverrides = scene?.overrides ?? {};
  return {
    name: scene?.name ?? "",
    enabled: scene?.enabled ?? true,
    apps: [...(scene?.match.apps ?? [])],
    keywords: [...(scene?.match.title_contains ?? [])],
    refine: o.refine_enabled == null ? "" : o.refine_enabled ? "on" : "off",
    preset: o.refine_preset ?? "",
    outputMode: o.output_mode ?? "",
    language: o.language ?? "",
    script: o.chinese_script ?? "",
    prompt: o.prompt ?? "",
  };
}

/** The wire draft: only the overrides that are set (unset keys absent, as the core stores them). */
export function sceneDraftOf(d: EditorDraft): SceneDraft {
  const overrides: SceneOverrides = {};
  if (d.refine !== "") overrides.refine_enabled = d.refine === "on";
  if (d.preset !== "") overrides.refine_preset = d.preset;
  if (d.outputMode !== "") overrides.output_mode = d.outputMode;
  if (d.language.trim().length > 0) overrides.language = d.language.trim();
  if (d.script !== "") overrides.chinese_script = d.script;
  if (d.prompt.trim().length > 0) overrides.prompt = d.prompt;
  return {
    name: d.name.trim(),
    enabled: d.enabled,
    match: { apps: [...d.apps], title_contains: [...d.keywords] },
    overrides,
  };
}

/** The draft for `scene` with only `enabled` changed (the card's switch). */
export function withEnabled(scene: Scene, enabled: boolean): SceneDraft {
  return { ...sceneDraftOf(editorDraftFrom(scene)), enabled };
}

/** `list` plus `raw` normalised as an app id; the same list when it is empty or already there. */
export function addApp(list: readonly string[], raw: string): string[] {
  const id = normalizeAppId(raw);
  return id.length === 0 || list.includes(id) ? [...list] : [...list, id];
}

/** `list` plus the trimmed keyword; the same list when it is empty or already there (any case). */
export function addKeyword(list: readonly string[], raw: string): string[] {
  const keyword = raw.trim();
  const lower = keyword.toLowerCase();
  return keyword.length === 0 || list.some((k) => k.toLowerCase() === lower)
    ? [...list]
    : [...list, keyword];
}

/** The prompt's length as the core counts it: characters after the line-ending clean-up. */
export function promptChars(prompt: string): number {
  return Array.from(prompt.replaceAll("\r\n", "\n").trim()).length;
}

function asciiFold(text: string): string {
  return text.replace(/[A-Z]/g, (c) => c.toLowerCase());
}

/** What is wrong with the form, per field (localised). `missing` problems (no name, no app) are
 *  shown after the first save attempt; the others as soon as they appear. */
export interface EditorProblems {
  name?: { text: string; missing: boolean };
  apps?: { text: string; missing: boolean };
  keywords?: { text: string; missing: boolean };
  prompt?: { text: string; missing: boolean };
}

export function editorProblems(
  d: EditorDraft,
  others: readonly Scene[],
  t: TFunction = zhT.t,
): EditorProblems {
  const out: EditorProblems = {};
  const name = d.name.trim();
  const clash = others.find((s) => asciiFold(s.name) === asciiFold(name));
  if (name.length === 0) out.name = { text: t("sceneEditor.error.name"), missing: true };
  else if (clash !== undefined)
    out.name = { text: t("sceneEditor.error.duplicate", { name: clash.name }), missing: false };
  if (d.apps.length === 0) out.apps = { text: t("sceneEditor.error.noApps"), missing: true };
  else if (d.apps.length > MAX_SCENE_APPS)
    out.apps = {
      text: t("sceneEditor.error.tooManyApps", { max: MAX_SCENE_APPS }),
      missing: false,
    };
  if (d.keywords.length > MAX_TITLE_KEYWORDS)
    out.keywords = {
      text: t("sceneEditor.error.tooManyKeywords", { max: MAX_TITLE_KEYWORDS }),
      missing: false,
    };
  if (promptChars(d.prompt) > MAX_SCENE_PROMPT_CHARS)
    out.prompt = {
      text: t("sceneEditor.error.promptTooLong", { max: MAX_SCENE_PROMPT_CHARS }),
      missing: false,
    };
  return out;
}

export function hasProblems(problems: EditorProblems): boolean {
  return Object.values(problems).some((p) => p !== undefined);
}

const SCRIPT_KEYS = {
  simplified: "engines.chineseScript.simplified",
  traditional: "engines.chineseScript.traditional",
  as_is: "engines.chineseScript.asIs",
} as const;

export interface Choice<V extends string> {
  value: V;
  label: string;
}

export function refineChoices(t: TFunction = zhT.t): Choice<Switch>[] {
  return [
    { value: "", label: t("sceneEditor.follow") },
    { value: "on", label: t("sceneEditor.refineOn") },
    { value: "off", label: t("sceneEditor.refineOff") },
  ];
}

/** 跟随全局, the built-in presets, the custom ones, and `current` when it names a custom preset that
 *  was deleted since (it stays selectable, labelled as such). */
export function presetChoices(
  presets: readonly CustomPreset[],
  current: string,
  t: TFunction = zhT.t,
): Choice<string>[] {
  const out: Choice<string>[] = [
    { value: "", label: t("sceneEditor.follow") },
    ...BUILTIN_PRESETS.map((id) => ({ value: id, label: t(`presets.${id}.name`) })),
    ...presets.map((p) => ({ value: p.id, label: p.name })),
  ];
  if (current !== "" && !out.some((o) => o.value === current))
    out.push({ value: current, label: t("presets.missing") });
  return out;
}

export function outputModeChoices(
  t: TFunction = zhT.t,
  locale: Locale = "zh-CN",
): Choice<"" | OutputMode>[] {
  return [
    { value: "", label: t("sceneEditor.follow") },
    ...OUTPUT_MODES.map((mode) => ({ value: mode, label: outputModeLabel(mode, locale) })),
  ];
}

export function scriptChoices(t: TFunction = zhT.t): Choice<"" | ChineseScript>[] {
  return [
    { value: "", label: t("sceneEditor.follow") },
    ...CHINESE_SCRIPTS.map((script) => ({
      value: script,
      label: t(SCRIPT_KEYS[script]),
    })),
  ];
}

/** A language override's name: 自动检测 for `auto`, the language's own name for the codes the
 *  engines dialog offers, the code itself for anything else the core accepted. */
export function languageName(code: string, t: TFunction = zhT.t): string {
  if (code === LANGUAGE_AUTO) return t("language.auto");
  for (const known of LANGUAGE_CODES) {
    if (known !== "" && known === code) return t(`language.${known}`);
  }
  return code;
}

/** 跟随全局, 自动检测, the engines dialog's languages, and `current` when it is none of them. */
export function languageChoices(current: string, t: TFunction = zhT.t): Choice<string>[] {
  const out: Choice<string>[] = [
    { value: "", label: t("sceneEditor.follow") },
    { value: LANGUAGE_AUTO, label: t("language.auto") },
  ];
  for (const code of LANGUAGE_CODES) {
    if (code !== "") out.push({ value: code, label: t(`language.${code}`) });
  }
  if (current !== "" && !out.some((o) => o.value === current))
    out.push({ value: current, label: current });
  return out;
}

/** The overrides a scene sets, one short phrase each, in the editor's order. */
export function overrideSummary(
  o: SceneOverrides,
  t: TFunction = zhT.t,
  locale: Locale = "zh-CN",
  presets: readonly CustomPreset[] = [],
): string[] {
  const out: string[] = [];
  if (o.refine_enabled != null)
    out.push(
      t(
        o.refine_enabled ? "settings.scenes.summary.refineOn" : "settings.scenes.summary.refineOff",
      ),
    );
  if (o.refine_preset != null)
    out.push(
      t("settings.scenes.summary.preset", {
        preset: presetLabel(o.refine_preset, presets, locale),
      }),
    );
  if (o.output_mode != null)
    out.push(t("settings.scenes.summary.output", { mode: outputModeLabel(o.output_mode, locale) }));
  if (o.language != null)
    out.push(t("settings.scenes.summary.language", { language: languageName(o.language, t) }));
  if (o.chinese_script != null)
    out.push(t("settings.scenes.summary.script", { script: t(SCRIPT_KEYS[o.chinese_script]) }));
  if (o.prompt != null) out.push(t("settings.scenes.summary.prompt"));
  return out;
}
