// Pure helpers of the AI presets (docs/dictation.md §21): what the preset menus list, the editor's
// instant checks in the core's rules, and the name a copy of a built-in preset starts with. The core
// validates every draft again; its refusal comes back as the command's rejection.
import {
  BUILTIN_PRESETS,
  type BuiltinPreset,
  type CustomPreset,
  MAX_PRESET_NAME_CHARS,
  MAX_PRESET_PROMPT_CHARS,
  MAX_PRESET_TRY_CHARS,
  type PresetDraft,
  type PresetId,
  type TFunction,
  zhT,
} from "@voltip/shared";
import type { MenuSection } from "@voltip/ui";

/** The menu row that opens the 预设 section of AI 模型 (no preset has this id: built-in ones have
 *  their names, custom ones a UUID). */
export const MANAGE_PRESETS = "manage";

/** The built-in presets, the custom ones (when there are any), then 管理预设…. */
export function presetMenuSections(
  current: PresetId,
  custom: readonly CustomPreset[],
  t: TFunction = zhT.t,
): MenuSection[] {
  const sections: MenuSection[] = [
    {
      label: t("presets.menu.builtin"),
      items: BUILTIN_PRESETS.map((id) => ({
        kind: "radio" as const,
        id,
        label: t(`presets.${id}.name`),
        checked: current === id,
      })),
    },
  ];
  if (custom.length > 0) {
    sections.push({
      label: t("presets.menu.custom"),
      items: custom.map((p) => ({
        kind: "radio" as const,
        id: p.id,
        label: p.name,
        checked: current === p.id,
        userText: true,
      })),
    });
  }
  sections.push({
    items: [{ kind: "action", id: MANAGE_PRESETS, label: t("presets.menu.manage") }],
  });
  return sections;
}

/** Characters as the core counts them (Unicode scalar values). */
export function presetChars(text: string): number {
  return Array.from(text).length;
}

/** ASCII case folding only, as the core compares names. */
function asciiLower(text: string): string {
  return text.replace(/[A-Z]/g, (c) => c.toLowerCase());
}

export interface PresetProblem {
  text: string;
  /** An empty field: shown only once the user tried to save. */
  missing: boolean;
}

export interface PresetProblems {
  name?: PresetProblem;
  prompt?: PresetProblem;
}

/** The editor's checks: a name of 1–24 characters that no other preset has (ignoring ASCII case),
 *  a prompt of 1–4000 characters, both counted after trimming. */
export function presetProblems(
  draft: PresetDraft,
  others: readonly CustomPreset[],
  t: TFunction = zhT.t,
): PresetProblems {
  const out: PresetProblems = {};
  const name = draft.name.trim();
  if (name.length === 0) out.name = { text: t("presets.editor.error.name"), missing: true };
  else if (presetChars(name) > MAX_PRESET_NAME_CHARS)
    out.name = {
      text: t("presets.editor.error.nameTooLong", { max: MAX_PRESET_NAME_CHARS }),
      missing: false,
    };
  else {
    const clash = others.find((p) => asciiLower(p.name) === asciiLower(name));
    if (clash !== undefined)
      out.name = {
        text: t("presets.editor.error.duplicate", { name: clash.name }),
        missing: false,
      };
  }
  const prompt = draft.prompt.trim();
  if (prompt.length === 0) out.prompt = { text: t("presets.editor.error.prompt"), missing: true };
  else if (presetChars(prompt) > MAX_PRESET_PROMPT_CHARS)
    out.prompt = {
      text: t("presets.editor.error.promptTooLong", { max: MAX_PRESET_PROMPT_CHARS }),
      missing: false,
    };
  return out;
}

/** Why the 试运行 sample cannot go out: empty, or over 2000 characters after trimming. */
export function sampleProblem(sample: string, t: TFunction = zhT.t): string | undefined {
  const text = sample.trim();
  if (text.length === 0) return t("presets.editor.error.sample");
  if (presetChars(text) > MAX_PRESET_TRY_CHARS)
    return t("presets.editor.error.sampleTooLong", { max: MAX_PRESET_TRY_CHARS });
  return undefined;
}

/** The draft 复制为自定义 opens the editor with: 「校对（副本）」 and the built-in text. */
export function copyDraft(id: BuiltinPreset, prompt: string, t: TFunction = zhT.t): PresetDraft {
  return { name: t("presets.section.copyOf", { name: t(`presets.${id}.name`) }), prompt };
}
