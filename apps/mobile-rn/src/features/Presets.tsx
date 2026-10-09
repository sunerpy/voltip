// 预设 on the AI 模型 page (docs/dictation.md §21), `@voltip/ui`'s PresetsSection, PresetEditor and
// usePresetTrial on native views: the built-in presets (use, 复制为自定义) and the custom ones (use,
// edit, delete after a confirmation), and the editor as a full-screen dialog with 试运行 on a
// sample text (`presets_try`, nothing saved). The checks are `@voltip/shared`'s and the core's.
import {
  BUILTIN_PRESETS,
  type BuiltinPreset,
  type CustomPreset,
  MAX_PRESETS,
  MAX_PRESET_PROMPT_CHARS,
  type PresetDraft,
  type PresetId,
  type PresetTryOutcome,
  copyDraft,
  errorText,
  formatMs,
  isBuiltinPreset,
  presetChars,
  presetDescription,
  presetLabel,
  presetProblems,
  sampleProblem,
} from "@voltip/shared";
import { useCallback, useEffect, useRef, useState } from "react";
import { View } from "react-native";
import { Icon, IconButton, Text, TextInput, TouchableRipple } from "react-native-paper";

import { useBackend, useUiState } from "../backend/BackendProvider";
import { useI18n, useT } from "../backend/i18n";
import { useShell } from "../shell";
import { Button } from "../ui/Button";
import { FullScreenDialog } from "../ui/FullScreenDialog";
import { Hint, Mono, RowDivider, Section, SectionTitle, useAppTheme } from "../ui/kit";

/** How long the editor waits for a `preset_try` answer before it gives up. */
export const PRESET_TRY_WAIT_MS = 60_000;
let lastTrial = 0;

/** 试运行 (`presets_try`): sends the request and listens for the `preset_try` event with its id. */
export function usePresetTrial() {
  const { backend } = useBackend();
  const t = useT();
  const [pending, setPending] = useState(false);
  const [outcome, setOutcome] = useState<PresetTryOutcome | undefined>(undefined);
  const current = useRef<number | undefined>(undefined);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const settle = useCallback((answer: PresetTryOutcome) => {
    clearTimeout(timer.current);
    current.current = undefined;
    setPending(false);
    setOutcome(answer);
  }, []);
  useEffect(
    () =>
      backend.on((event) => {
        if (event.type === "preset_try" && event.id === current.current) settle(event.outcome);
      }),
    [backend, settle],
  );
  useEffect(
    () => () => {
      clearTimeout(timer.current);
    },
    [],
  );
  const run = useCallback(
    (source: { preset: PresetId } | { prompt: string }, text: string) => {
      lastTrial += 1;
      const id = lastTrial;
      current.current = id;
      setPending(true);
      setOutcome(undefined);
      clearTimeout(timer.current);
      timer.current = setTimeout(() => {
        if (current.current === id)
          settle({ status: "failed", reason: t("presets.editor.trial.timeout") });
      }, PRESET_TRY_WAIT_MS);
      backend
        .invoke("presets_try", {
          id,
          preset: "preset" in source ? source.preset : null,
          prompt: "prompt" in source ? source.prompt : null,
          text,
        })
        .catch((e: unknown) => {
          if (current.current === id) settle({ status: "failed", reason: errorText(e) });
        });
    },
    [backend, settle, t],
  );
  const reset = useCallback(() => {
    clearTimeout(timer.current);
    current.current = undefined;
    setPending(false);
    setOutcome(undefined);
  }, []);
  return { pending, outcome, run, reset };
}

function PresetEditor({
  preset,
  initial,
  onClose,
}: {
  preset?: CustomPreset | undefined;
  initial?: PresetDraft | undefined;
  onClose: () => void;
}) {
  const theme = useAppTheme();
  const { backend } = useBackend();
  const shell = useShell();
  const { t } = useI18n();
  const state = useUiState();
  const [draft, setDraft] = useState<PresetDraft>(() =>
    preset !== undefined
      ? { name: preset.name, prompt: preset.prompt }
      : (initial ?? { name: "", prompt: "" }),
  );
  const [sample, setSample] = useState(() => t("presets.editor.trial.sampleText"));
  const [attempted, setAttempted] = useState(false);
  const [saveError, setSaveError] = useState<string | undefined>(undefined);
  const trial = usePresetTrial();
  const problems = presetProblems(
    draft,
    state.presets.filter((p) => p.id !== preset?.id),
    t,
  );
  const shown = (p: (typeof problems)[keyof typeof problems]) =>
    p !== undefined && (attempted || !p.missing) ? p.text : undefined;
  const update = (patch: Partial<PresetDraft>) => {
    setDraft((d) => ({ ...d, ...patch }));
    setSaveError(undefined);
  };
  const save = async () => {
    setAttempted(true);
    if (problems.name !== undefined || problems.prompt !== undefined) return;
    const payload = { name: draft.name.trim(), prompt: draft.prompt };
    try {
      if (preset === undefined) await backend.invoke("presets_add", { preset: payload });
      else await backend.invoke("presets_update", { id: preset.id, preset: payload });
      shell.toast(t("presets.editor.saved", { name: payload.name }));
      onClose();
    } catch (e) {
      setSaveError(errorText(e));
    }
  };
  const chars = presetChars(draft.prompt.trim());
  const ready = state.engines.refine_ready;
  const sampleIssue = sampleProblem(sample, t);
  const canTry =
    ready && !trial.pending && problems.prompt === undefined && sampleIssue === undefined;
  const outcome = trial.outcome;
  const nameProblem = shown(problems.name);
  const promptProblem = shown(problems.prompt);
  return (
    <FullScreenDialog
      visible
      title={preset === undefined ? t("presets.editor.titleNew") : t("presets.editor.titleEdit")}
      onClose={onClose}
      action={{ label: t("presets.editor.save"), onPress: () => void save() }}
      testID="preset-editor">
      <View style={{ gap: 4 }}>
        <TextInput
          mode="outlined"
          label={t("presets.editor.name")}
          value={draft.name}
          placeholder={t("presets.editor.namePlaceholder")}
          error={nameProblem !== undefined}
          autoFocus
          onChangeText={(name) => {
            update({ name });
          }}
        />
        {nameProblem !== undefined && <Hint tone="danger">{nameProblem}</Hint>}
      </View>
      <View style={{ gap: 4 }}>
        <TextInput
          mode="outlined"
          label={t("presets.editor.prompt")}
          value={draft.prompt}
          multiline
          numberOfLines={8}
          error={promptProblem !== undefined}
          onChangeText={(prompt) => {
            update({ prompt });
          }}
        />
        <View style={{ flexDirection: "row", justifyContent: "space-between", gap: 12 }}>
          <Hint {...(promptProblem === undefined ? {} : { tone: "danger" as const })}>
            {promptProblem ?? t("presets.editor.promptHelp")}
          </Hint>
          <Mono
            style={{
              color: chars > MAX_PRESET_PROMPT_CHARS ? theme.colors.error : theme.voltip.subtle,
            }}>
            {t("presets.editor.count", { n: chars, max: MAX_PRESET_PROMPT_CHARS })}
          </Mono>
        </View>
      </View>
      <Section title={t("presets.editor.trial.title")} padded testID="preset-trial">
        <Hint>
          {ready ? t("presets.editor.trial.help") : t("presets.editor.trial.unavailable")}
        </Hint>
        <TextInput
          mode="outlined"
          label={t("presets.editor.trial.sample")}
          value={sample}
          multiline
          numberOfLines={3}
          onChangeText={(text) => {
            setSample(text);
            trial.reset();
          }}
        />
        {sampleIssue !== undefined && sample.trim().length > 0 && (
          <Hint tone="danger">{sampleIssue}</Hint>
        )}
        <Button
          mode="outlined"
          icon="play"
          disabled={!canTry}
          loading={trial.pending}
          style={{ alignSelf: "flex-start" }}
          onPress={() => {
            trial.run({ prompt: draft.prompt }, sample.trim());
          }}>
          {trial.pending ? t("presets.editor.trial.running") : t("presets.editor.trial.run")}
        </Button>
        {outcome?.status === "ok" && (
          <View style={{ gap: 4 }} testID="preset-trial-result">
            <Mono>
              {t("presets.editor.trial.result", {
                model: outcome.model,
                time: formatMs(outcome.latency_ms),
              })}
            </Mono>
            <View
              style={{
                padding: 12,
                borderRadius: 12,
                backgroundColor: theme.colors.surfaceVariant,
              }}>
              <Text variant="bodyMedium" selectable>
                {outcome.text}
              </Text>
            </View>
          </View>
        )}
        {outcome?.status === "failed" && (
          <Hint tone="danger">{t("presets.editor.trial.failed", { reason: outcome.reason })}</Hint>
        )}
      </Section>
      {saveError !== undefined && <Hint tone="danger">{saveError}</Hint>}
    </FullScreenDialog>
  );
}

type Editing = { preset: CustomPreset } | { initial?: PresetDraft };

/** One preset as a selectable row: its name, a line about it, 使用中 when it is in use. */
function PresetRow({
  title,
  description,
  selected,
  onSelect,
  actions,
  testID,
}: {
  title: string;
  description: string;
  selected: boolean;
  onSelect: () => void;
  actions?: React.ReactNode;
  testID?: string;
}) {
  const theme = useAppTheme();
  const t = useT();
  return (
    <TouchableRipple
      onPress={onSelect}
      accessibilityRole="radio"
      accessibilityState={{ checked: selected }}
      accessibilityLabel={title}
      testID={testID}>
      <View
        style={{
          flexDirection: "row",
          alignItems: "center",
          gap: 12,
          paddingLeft: 16,
          paddingRight: 4,
          paddingVertical: 12,
        }}>
        <Icon
          source={selected ? "radiobox-marked" : "radiobox-blank"}
          size={22}
          color={selected ? theme.colors.primary : theme.colors.onSurfaceVariant}
        />
        <View style={{ flex: 1, gap: 2 }}>
          <Text variant="bodyLarge">
            {title}
            {selected && (
              <Text
                variant="labelSmall"
                style={{ color: theme.colors.primary }}>{`  ${t("presets.section.inUse")}`}</Text>
            )}
          </Text>
          <Text
            variant="bodySmall"
            numberOfLines={2}
            style={{ color: theme.colors.onSurfaceVariant }}>
            {description}
          </Text>
        </View>
        {actions}
      </View>
    </TouchableRipple>
  );
}

export function PresetsSection() {
  const theme = useAppTheme();
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
      shell.toast(errorText(e), "danger");
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
        shell.toast(errorText(e), "danger");
      },
    );
  };
  const remove = (preset: CustomPreset) => {
    shell.confirm({
      title: t("presets.section.confirmRemove.title", { name: preset.name }),
      body: t("presets.section.confirmRemove.body"),
      confirmLabel: t("common.delete"),
      onConfirm: () => {
        run(
          backend.invoke("presets_remove", { id: preset.id }).then(() => {
            shell.toast(t("presets.section.removed", { name: preset.name }));
          }),
        );
      },
    });
  };
  return (
    <View style={{ gap: 12 }} testID="presets-section">
      <View style={{ gap: 4, paddingHorizontal: 4 }}>
        <SectionTitle>{t("presets.section.title")}</SectionTitle>
        <Hint>{t("presets.section.note")}</Hint>
        <Text variant="bodySmall" testID="presets-current">
          {t("presets.section.current", { name: presetLabel(current, custom, locale) })}
        </Text>
      </View>
      <Section title={t("presets.section.builtin")}>
        {BUILTIN_PRESETS.map((id, i) => (
          <View key={id}>
            {i > 0 && <RowDivider />}
            <PresetRow
              title={t(`presets.${id}.name`)}
              description={presetDescription(id, locale)}
              selected={current === id}
              onSelect={() => {
                use(id);
              }}
              testID={`preset-${id}`}
              actions={
                <IconButton
                  icon="content-copy"
                  disabled={custom.length >= MAX_PRESETS}
                  accessibilityLabel={`${t("presets.section.copy")} · ${t(`presets.${id}.name`)}`}
                  onPress={() => {
                    copy(id);
                  }}
                />
              }
            />
          </View>
        ))}
      </Section>
      <Section
        title={t("presets.section.custom")}
        right={
          <Button
            compact
            icon="plus"
            disabled={custom.length >= MAX_PRESETS}
            onPress={() => {
              setEditing({});
            }}>
            {t("presets.section.add")}
          </Button>
        }
        footer={t("presets.section.footnote", { max: MAX_PRESETS })}>
        {custom.length === 0 ? (
          <Text
            variant="bodyMedium"
            testID="presets-empty"
            style={{ padding: 16, color: theme.colors.onSurfaceVariant }}>
            {t("presets.section.customEmpty")}
          </Text>
        ) : (
          custom.map((preset, i) => (
            <View key={preset.id}>
              {i > 0 && <RowDivider />}
              <PresetRow
                title={preset.name}
                description={preset.prompt}
                selected={current === preset.id}
                onSelect={() => {
                  use(preset.id);
                }}
                actions={
                  <View style={{ flexDirection: "row" }}>
                    <IconButton
                      icon="pencil-outline"
                      accessibilityLabel={t("presets.section.edit", { name: preset.name })}
                      onPress={() => {
                        setEditing({ preset });
                      }}
                    />
                    <IconButton
                      icon="trash-can-outline"
                      iconColor={theme.colors.error}
                      accessibilityLabel={t("presets.section.remove", { name: preset.name })}
                      onPress={() => {
                        remove(preset);
                      }}
                    />
                  </View>
                }
              />
            </View>
          ))
        )}
      </Section>
      {missing && <Hint tone="danger">{t("presets.missing")}</Hint>}
      {editing !== undefined && (
        <PresetEditor
          {...("preset" in editing ? { preset: editing.preset } : { initial: editing.initial })}
          onClose={() => {
            setEditing(undefined);
          }}
        />
      )}
    </View>
  );
}
