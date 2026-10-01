// Pure helpers of the scene editor and the scene cards, shared by the desktop's 场景 settings and the
// phone's (docs/dictation.md §18; user decision 2026-10-01: the phone has the desktop's scenes,
// picked by hand instead of matched by application): the editor's form state and its conversion to
// the wire `SceneDraft`, the instant localised checks shown while typing, the select options (every
// override starts at 跟随全局), and the one-line summary on a scene card. The core validates the
// draft again (its refusal comes back as the command's rejection) and checks the list (a clash is
// an `error` event). Moved here from `apps/desktop/src/pages/settings/scenes/helpers.ts`.
import { LANGUAGE_CODES } from "./engine-drafts";
import { type Locale, zhT, type TFunction } from "./i18n";
import { outputModeLabel, presetLabel } from "./labels";
import { normalizeAppId } from "./scenes";
import {
  BUILTIN_PRESETS,
  CHINESE_SCRIPTS,
  type ChineseScript,
  type CustomPreset,
  LANGUAGE_AUTO,
  MAX_SCENE_APPS,
  MAX_SCENE_PROMPT_CHARS,
  MAX_TITLE_KEYWORDS,
  OUTPUT_MODES,
  type OutputMode,
  type PresetId,
  type Scene,
  type SceneDraft,
  type SceneOverrides,
} from "./schema";

/** An override that is either unset (`""`, 跟随全局) or switched on / off. */
export type SceneSwitch = "" | "on" | "off";

/** The editor's form: the overrides as select values, `""` meaning 跟随全局 (unset). */
export interface SceneEditorDraft {
  name: string;
  enabled: boolean;
  /** Normalised app ids, in the order they were added. */
  apps: string[];
  keywords: string[];
  refine: SceneSwitch;
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
export function sceneEditorDraftFrom(scene?: Scene): SceneEditorDraft {
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
export function sceneDraftOf(d: SceneEditorDraft): SceneDraft {
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
export function sceneWithEnabled(scene: Scene, enabled: boolean): SceneDraft {
  return { ...sceneDraftOf(sceneEditorDraftFrom(scene)), enabled };
}

/** `list` plus `raw` normalised as an app id; the same list when it is empty or already there. */
export function addSceneApp(list: readonly string[], raw: string): string[] {
  const id = normalizeAppId(raw);
  return id.length === 0 || list.includes(id) ? [...list] : [...list, id];
}

/** `list` plus the trimmed keyword; the same list when it is empty or already there (any case). */
export function addSceneKeyword(list: readonly string[], raw: string): string[] {
  const keyword = raw.trim();
  const lower = keyword.toLowerCase();
  return keyword.length === 0 || list.some((k) => k.toLowerCase() === lower)
    ? [...list]
    : [...list, keyword];
}

/** The prompt's length as the core counts it: characters after the line-ending clean-up. */
export function scenePromptChars(prompt: string): number {
  return Array.from(prompt.replaceAll("\r\n", "\n").trim()).length;
}

function asciiFold(text: string): string {
  return text.replace(/[A-Z]/g, (c) => c.toLowerCase());
}

/** What is wrong with the form, per field (localised). `missing` problems (no name, no app) are
 *  shown after the first save attempt; the others as soon as they appear. */
export interface SceneEditorProblems {
  name?: { text: string; missing: boolean };
  apps?: { text: string; missing: boolean };
  keywords?: { text: string; missing: boolean };
  prompt?: { text: string; missing: boolean };
}

/** How a draft is checked: `builtin`, the draft is a built-in scene's (docs/dictation.md §18.10),
 *  which keeps its name and may list no application (names are unique among the user's scenes
 *  only); `needApps` false, scenes are picked by hand rather than matched (the phone, §18), so no
 *  scene has to list one. */
export interface SceneCheck {
  builtin?: boolean;
  needApps?: boolean;
}

export function sceneEditorProblems(
  d: SceneEditorDraft,
  others: readonly Scene[],
  t: TFunction = zhT.t,
  { builtin = false, needApps = true }: SceneCheck = {},
): SceneEditorProblems {
  const out: SceneEditorProblems = {};
  const name = d.name.trim();
  const clash = builtin
    ? undefined
    : others.find((s) => s.builtin === undefined && asciiFold(s.name) === asciiFold(name));
  if (name.length === 0) out.name = { text: t("sceneEditor.error.name"), missing: true };
  else if (clash !== undefined)
    out.name = { text: t("sceneEditor.error.duplicate", { name: clash.name }), missing: false };
  if (d.apps.length === 0 && !builtin && needApps)
    out.apps = { text: t("sceneEditor.error.noApps"), missing: true };
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
  if (scenePromptChars(d.prompt) > MAX_SCENE_PROMPT_CHARS)
    out.prompt = {
      text: t("sceneEditor.error.promptTooLong", { max: MAX_SCENE_PROMPT_CHARS }),
      missing: false,
    };
  return out;
}

export function hasSceneProblems(problems: SceneEditorProblems): boolean {
  return Object.values(problems).some((p) => p !== undefined);
}

const SCRIPT_KEYS = {
  simplified: "engines.chineseScript.simplified",
  traditional: "engines.chineseScript.traditional",
  as_is: "engines.chineseScript.asIs",
} as const;

export interface SceneChoice<V extends string> {
  value: V;
  label: string;
}

export function sceneRefineChoices(t: TFunction = zhT.t): SceneChoice<SceneSwitch>[] {
  return [
    { value: "", label: t("sceneEditor.follow") },
    { value: "on", label: t("sceneEditor.refineOn") },
    { value: "off", label: t("sceneEditor.refineOff") },
  ];
}

/** 跟随全局, the built-in presets, the custom ones, and `current` when it names a custom preset that
 *  was deleted since (it stays selectable, labelled as such). */
export function scenePresetChoices(
  presets: readonly CustomPreset[],
  current: string,
  t: TFunction = zhT.t,
): SceneChoice<string>[] {
  const out: SceneChoice<string>[] = [
    { value: "", label: t("sceneEditor.follow") },
    ...BUILTIN_PRESETS.map((id) => ({ value: id, label: t(`presets.${id}.name`) })),
    ...presets.map((p) => ({ value: p.id, label: p.name })),
  ];
  if (current !== "" && !out.some((o) => o.value === current))
    out.push({ value: current, label: t("presets.missing") });
  return out;
}

export function sceneOutputModeChoices(
  t: TFunction = zhT.t,
  locale: Locale = "zh-CN",
): SceneChoice<"" | OutputMode>[] {
  return [
    { value: "", label: t("sceneEditor.follow") },
    ...OUTPUT_MODES.map((mode) => ({ value: mode, label: outputModeLabel(mode, locale) })),
  ];
}

export function sceneScriptChoices(t: TFunction = zhT.t): SceneChoice<"" | ChineseScript>[] {
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
export function sceneLanguageChoices(current: string, t: TFunction = zhT.t): SceneChoice<string>[] {
  const out: SceneChoice<string>[] = [
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
export function sceneOverrideSummary(
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
