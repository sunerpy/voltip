import {
  BUILTIN_PRESETS,
  type BuiltinPreset,
  type CustomPreset,
  MAX_PRESETS,
  type PresetDraft,
  type PresetId,
  isBuiltinPreset,
  presetDescription,
  presetLabel,
} from "@voltip/shared";
import {
  Badge,
  Button,
  CardGrid,
  IconButton,
  OptionCard,
  SettingsSection,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { useState } from "react";
import { useShell } from "../../../app/shell-context";
import { copyDraft } from "../../../features/presets/presets";
import { errorText } from "../../../features/vocabulary/vocabulary";
import { PresetEditor } from "./PresetEditor";

type Editing = { preset: CustomPreset } | { initial?: PresetDraft };

/** 预设 on the AI 模型 page (docs/dictation.md §21): the built-in presets (use, 复制为自定义) and
 *  the custom ones (use, edit, delete after a confirmation), with the editor dialog. Choosing a card
 *  saves `refine_preset` through `settings_set_engines` with the rest of the engine settings. */
export function PresetsSection() {
  const { backend } = useBackend();
  const { t, locale } = useI18n();
  const shell = useShell();
  const state = useUiState();
  const engines = state.settings.engines;
  const current = engines.refine_preset;
  const custom = state.presets;
  const [editing, setEditing] = useState<Editing | undefined>(undefined);
  const missing = !isBuiltinPreset(current) && !custom.some((p) => p.id === current);

  const run = (action: Promise<void>) => {
    action.catch((e: unknown) => {
      shell.toast({ message: errorText(e), duration: 5000, tone: "danger" });
    });
  };
  const use = (id: PresetId) => {
    if (id !== current)
      run(backend.invoke("settings_set_engines", { engines: { ...engines, refine_preset: id } }));
  };
  const copy = (id: BuiltinPreset) => {
    backend.presetsBuiltin().then(
      (texts) => {
        setEditing({ initial: copyDraft(id, texts.find((x) => x.id === id)?.prompt ?? "", t) });
      },
      (e: unknown) => {
        shell.toast({ message: errorText(e), duration: 5000, tone: "danger" });
      },
    );
  };
  const remove = (preset: CustomPreset) => {
    shell.confirm({
      title: t("presets.section.confirmRemove.title", { name: preset.name }),
      body: t("presets.section.confirmRemove.body"),
      confirmLabel: t("common.delete"),
      tone: "danger",
      onConfirm: () => {
        run(
          backend.invoke("presets_remove", { id: preset.id }).then(() => {
            shell.toast({
              message: t("presets.section.removed", { name: preset.name }),
              duration: 3000,
            });
          }),
        );
      },
    });
  };
  const inUse = <Badge tone="accent">{t("presets.section.inUse")}</Badge>;

  return (
    <SettingsSection
      title={t("presets.section.title")}
      description={t("presets.section.note")}
      data-testid="presets-section"
      aside={
        <span
          className="text-fg-muted"
          data-testid="presets-current"
          // A custom preset's name is the user's text.
          data-user-text={!isBuiltinPreset(current) && !missing ? "" : undefined}>
          {t("presets.section.current", { name: presetLabel(current, custom, locale) })}
        </span>
      }>
      <div className="flex flex-col gap-2">
        <h4 className="text-[13px] font-medium text-fg">{t("presets.section.builtin")}</h4>
        <CardGrid role="listbox" aria-label={t("presets.section.builtin")} min={260}>
          {BUILTIN_PRESETS.map((id) => (
            <OptionCard
              key={id}
              selected={current === id}
              onSelect={() => {
                use(id);
              }}
              title={t(`presets.${id}.name`)}
              badge={current === id ? inUse : undefined}
              footer={
                <Button
                  size="sm"
                  variant="text"
                  icon="copy"
                  disabled={custom.length >= MAX_PRESETS}
                  onClick={() => {
                    copy(id);
                  }}>
                  {t("presets.section.copy")}
                </Button>
              }>
              <p className="text-[12px] leading-4 text-fg-muted">{presetDescription(id, locale)}</p>
            </OptionCard>
          ))}
        </CardGrid>
      </div>

      <div className="flex flex-col gap-2">
        <div className="flex items-center justify-between gap-3">
          <h4 className="text-[13px] font-medium text-fg">{t("presets.section.custom")}</h4>
          <Button
            size="sm"
            variant="outline"
            icon="plus"
            disabled={custom.length >= MAX_PRESETS}
            onClick={() => {
              setEditing({});
            }}>
            {t("presets.section.add")}
          </Button>
        </div>
        {custom.length === 0 ? (
          <p className="text-[12px] text-fg-muted" data-testid="presets-empty">
            {t("presets.section.customEmpty")}
          </p>
        ) : (
          <CardGrid role="listbox" aria-label={t("presets.section.custom")} min={260}>
            {custom.map((preset) => (
              <OptionCard
                key={preset.id}
                aria-label={preset.name}
                selected={current === preset.id}
                onSelect={() => {
                  use(preset.id);
                }}
                title={<span data-user-text>{preset.name}</span>}
                badge={current === preset.id ? inUse : undefined}
                footer={
                  <>
                    <IconButton
                      icon="edit"
                      label={t("presets.section.edit", { name: preset.name })}
                      onClick={() => {
                        setEditing({ preset });
                      }}
                    />
                    <IconButton
                      icon="trash"
                      tone="danger"
                      label={t("presets.section.remove", { name: preset.name })}
                      onClick={() => {
                        remove(preset);
                      }}
                    />
                  </>
                }>
                <p
                  className="line-clamp-2 text-[12px] leading-4 whitespace-pre-line text-fg-muted"
                  data-user-text>
                  {preset.prompt}
                </p>
              </OptionCard>
            ))}
          </CardGrid>
        )}
        <p className="text-[11px] text-fg-subtle">
          {t("presets.section.footnote", { max: MAX_PRESETS })}
        </p>
        {missing && (
          <p className="text-[12px] text-warning" data-testid="presets-missing">
            {t("presets.missing")}
          </p>
        )}
      </div>

      {editing !== undefined && (
        <PresetEditor
          {...("preset" in editing ? { preset: editing.preset } : { initial: editing.initial })}
          onClose={() => {
            setEditing(undefined);
          }}
        />
      )}
    </SettingsSection>
  );
}
