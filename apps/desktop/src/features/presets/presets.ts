// Pure helpers of the AI presets (docs/dictation.md §21): what the preset menus list, the editor's
// instant checks in the core's rules, and the name a copy of a built-in preset starts with. The core
// validates every draft again; its refusal comes back as the command's rejection.
import {
  BUILTIN_PRESETS,
  type CustomPreset,
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

export {
  type PresetProblem,
  type PresetProblems,
  copyDraft,
  presetChars,
  presetProblems,
  sampleProblem,
} from "@voltip/shared";
