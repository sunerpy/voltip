// Presets as the core keeps them (docs/dictation.md §21), for the in-memory backend and the preset
// editor's local checks: the same limits and refusal texts as `voltip_core::presets` and the
// bridge, and the rule that names a take's preset. The desktop app still has the core decide (a
// draft is validated again by the bridge); this module only keeps the mock and the editor honest.
import {
  type BuiltinPreset,
  type CustomPreset,
  DEFAULT_PRESET,
  MAX_PRESET_NAME_CHARS,
  MAX_PRESET_PROMPT_CHARS,
  MAX_PRESET_TRY_CHARS,
  MAX_PRESETS,
  type PresetDraft,
  type PresetId,
  type PresetRef,
  isBuiltinPreset,
} from "./schema";

/** A refusal carrying the core's text (`presets: …`). */
export class PresetError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "PresetError";
  }
}

const presetErr = (message: string) => new PresetError(`presets: ${message}`);

/** `BuiltinPreset::display_name`: the name the core writes into a `PresetRef` (the history, the
 *  status). The interface names a built-in preset by its id (`presets.<id>.name`) instead. */
export const BUILTIN_PRESET_NAMES: Readonly<Record<BuiltinPreset, string>> = {
  proofread: "校对",
  prompt: "提示词优化",
  intent: "意图整理",
  chat: "口语聊天",
  translate: "中英互译",
  notes: "要点纪要",
  punctuation: "只加标点",
  formal: "书面语",
};

/** Characters (Unicode scalar values), as Rust's `chars().count()`. */
function charCount(text: string): number {
  return Array.from(text).length;
}

/** Rust's `char::is_control`: the general category Cc. */
const CONTROL = /\p{Cc}/u;
/** A control character other than the newline and the tab. */
const CONTROL_BUT_LINES = /[^\P{Cc}\n\t]/u;

/** `clean_preset_prompt`: line endings normalised, ends trimmed, 1–`MAX_PRESET_PROMPT_CHARS`
 *  characters, no control character but the newline and the tab. */
export function cleanPresetPrompt(raw: string): string {
  const text = raw.replace(/\r\n?/g, "\n").trim();
  if (text.length === 0) throw presetErr("预设内容不能为空");
  const chars = charCount(text);
  if (chars > MAX_PRESET_PROMPT_CHARS) {
    throw presetErr(`预设内容最多 ${MAX_PRESET_PROMPT_CHARS} 个字符（当前 ${chars}）`);
  }
  if (CONTROL_BUT_LINES.test(text)) throw presetErr("预设内容不能包含控制字符");
  return text;
}

/** `validate_preset_draft`: the name trimmed and on one line, the instruction cleaned. Checks
 *  across the list happen in `checkPresets`. */
export function validatePresetDraft(draft: PresetDraft): PresetDraft {
  const name = draft.name.trim();
  if (name.length === 0) throw presetErr("预设名称不能为空");
  const chars = charCount(name);
  if (chars > MAX_PRESET_NAME_CHARS) {
    throw presetErr(`预设名称最多 ${MAX_PRESET_NAME_CHARS} 个字符（当前 ${chars}）`);
  }
  if (CONTROL.test(name)) throw presetErr("预设名称不能包含换行或控制字符");
  return { name, prompt: cleanPresetPrompt(draft.prompt) };
}

/** ASCII case folding only, as Rust's `eq_ignore_ascii_case`. */
function asciiLower(text: string): string {
  return text.replace(/[A-Z]/g, (c) => c.toLowerCase());
}

/** `check_presets`: at most `MAX_PRESETS`, names unique ignoring ASCII case. */
export function checkPresets(presets: readonly CustomPreset[]): void {
  if (presets.length > MAX_PRESETS) throw presetErr(`自定义预设最多 ${MAX_PRESETS} 个`);
  const seen = new Map<string, string>();
  for (const preset of presets) {
    const key = asciiLower(preset.name);
    const earlier = seen.get(key);
    if (earlier !== undefined) throw presetErr(`已有名为「${earlier}」的预设`);
    seen.set(key, preset.name);
  }
}

/** `presets::resolve` as a `PresetRef`: a built-in preset by its name, a custom one as it is now;
 *  a custom preset that is gone is 校对 (`missing` says so). */
export function resolvePreset(
  id: PresetId,
  presets: readonly CustomPreset[],
): { preset: PresetRef; missing: boolean } {
  if (isBuiltinPreset(id))
    return { preset: { id, name: BUILTIN_PRESET_NAMES[id] }, missing: false };
  const custom = presets.find((p) => p.id === id);
  if (custom !== undefined) return { preset: { id: custom.id, name: custom.name }, missing: false };
  return {
    preset: { id: DEFAULT_PRESET, name: BUILTIN_PRESET_NAMES[DEFAULT_PRESET] },
    missing: true,
  };
}

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** What one 试一试 runs: a saved preset, or the instruction being edited. */
export type PresetTrial = { preset: PresetId } | { prompt: string };

/** The bridge's `preset_trial`: exactly one of a preset id (`PresetId::parse`: a built-in name,
 *  `default` of the refine styles of old, or a UUID) and an instruction (cleaned like a draft's). */
export function presetTrial(preset: string | null, prompt: string | null): PresetTrial {
  if (preset !== null && prompt === null) {
    const id = preset.trim();
    if (id === "default") return { preset: DEFAULT_PRESET };
    if (isBuiltinPreset(id)) return { preset: id };
    if (UUID.test(id)) return { preset: id.toLowerCase() };
    throw presetErr(`没有名为「${preset}」的预设`);
  }
  if (preset === null && prompt !== null) return { prompt: cleanPresetPrompt(prompt) };
  throw presetErr("试一试需要一个预设或一段预设内容");
}

/** The bridge's `preset_try_text`: the sample trimmed, 1–`MAX_PRESET_TRY_CHARS` characters. */
export function presetTryText(text: string): string {
  const trimmed = text.trim();
  const chars = charCount(trimmed);
  if (chars === 0) throw presetErr("请输入要试运行的文字");
  if (chars > MAX_PRESET_TRY_CHARS) {
    throw presetErr(`试运行的文字最多 ${MAX_PRESET_TRY_CHARS} 个字符（当前 ${chars}）`);
  }
  return trimmed;
}
