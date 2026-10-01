// The AI preset editor's checks in the core's rules, and the name a copy of a built-in preset starts
// with (docs/dictation.md §21), shared by the desktop and the phone (user decision 2026-10-01). The
// core validates every draft again; its refusal comes back as the command's rejection. Moved here
// from `apps/desktop/src/features/presets/presets.ts`, which re-exports it.
import { zhT, type TFunction } from "./i18n";
import { coreMessageText } from "./labels";
import {
  type BuiltinPreset,
  type CustomPreset,
  MAX_PRESET_NAME_CHARS,
  MAX_PRESET_PROMPT_CHARS,
  MAX_PRESET_TRY_CHARS,
  type PresetDraft,
} from "./schema";

/** A command's rejection as the sentence the interface shows (the core's own message, in the
 *  interface's words where it has them). */
export function errorText(e: unknown): string {
  return coreMessageText(e instanceof Error ? e.message : String(e));
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
