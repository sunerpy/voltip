import { presetLabel } from "@voltip/shared";
import { Menu, useBackend, useI18n, useUiState } from "@voltip/ui";
import type { ReactNode } from "react";
import { useRouter } from "../../app/router";
import { MANAGE_PRESETS, presetMenuSections } from "./presets";

export interface PresetMenuProps {
  /** What the button shows; the current preset's name when absent. */
  trigger?: ReactNode;
  triggerClassName?: string;
  align?: "start" | "end";
  title?: string;
  "data-testid"?: string;
}

/** The current AI preset as a menu button (the home page's ready bar and the title bar share it,
 *  docs/dictation.md §21): pick another preset, which `settings_set_engines` saves with the rest
 *  of the engine settings, or open the 预设 section of AI 模型. */
export function PresetMenu({
  trigger,
  triggerClassName,
  align,
  title,
  "data-testid": testId,
}: PresetMenuProps) {
  const { backend } = useBackend();
  const { t, locale } = useI18n();
  const state = useUiState();
  const { navigate } = useRouter();
  const engines = state.settings.engines;
  const name = presetLabel(engines.refine_preset, state.presets, locale);
  return (
    <Menu
      trigger={trigger ?? name}
      label={t("presets.menu.label")}
      triggerLabel={t("presets.menu.trigger", { name })}
      title={title}
      sections={presetMenuSections(engines.refine_preset, state.presets, t)}
      align={align}
      triggerClassName={triggerClassName}
      data-testid={testId}
      onSelect={(id) => {
        if (id === MANAGE_PRESETS) {
          navigate({ name: "ai", section: "presets" });
          return;
        }
        if (id === engines.refine_preset) return;
        void backend.invoke("settings_set_engines", { engines: { ...engines, refine_preset: id } });
      }}
    />
  );
}
