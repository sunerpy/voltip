import {
  type CustomPreset,
  MAX_PRESET_PROMPT_CHARS,
  type PresetDraft,
  errorText,
  formatMs,
  presetChars,
  presetProblems,
  sampleProblem,
} from "@voltip/shared";
import { useId, useState } from "react";
import { useBackend, useUiState } from "../../backend/BackendProvider";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { Input, Textarea } from "../../components/Input";
import { useI18n } from "../../i18n/I18nProvider";
import { useFeatureShell } from "../shell";
import { usePresetTrial } from "./usePresetTrial";

export interface PresetEditorProps {
  /** The preset being edited; absent for a new one. */
  preset?: CustomPreset;
  /** What a new preset starts from (复制为自定义 passes the built-in text). */
  initial?: PresetDraft;
  onClose: () => void;
}

/** 新建预设 / 编辑预设: a name, the prompt with its count, and 试运行 on a sample text through the
 *  current clean-up (`presets_try`, nothing saved). The checks are the core's; the core checks
 *  the draft again on `presets_add` / `presets_update`, and a refusal is shown here. */
export function PresetEditor({ preset, initial, onClose }: PresetEditorProps) {
  const { backend } = useBackend();
  const shell = useFeatureShell();
  const { t } = useI18n();
  const state = useUiState();
  const promptId = useId();
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
      shell.notify(t("presets.editor.saved", { name: payload.name }));
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

  return (
    <Dialog
      open
      title={preset === undefined ? t("presets.editor.titleNew") : t("presets.editor.titleEdit")}
      width={720}
      onClose={onClose}
      actions={
        <>
          <Button size="sm" variant="ghost" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button
            size="sm"
            variant="primary"
            keys="Ctrl S"
            onClick={() => {
              void save();
            }}>
            {t("presets.editor.save")}
          </Button>
        </>
      }>
      <div
        className="flex max-h-[64vh] flex-col gap-4 overflow-y-auto pr-1"
        data-testid="preset-editor"
        onKeyDown={(e) => {
          if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
            e.preventDefault();
            void save();
          }
        }}>
        <Input
          label={t("presets.editor.name")}
          size="sm"
          value={draft.name}
          placeholder={t("presets.editor.namePlaceholder")}
          error={shown(problems.name)}
          data-autofocus
          onChange={(e) => {
            update({ name: e.target.value });
          }}
        />
        <div className="flex flex-col gap-1">
          <Textarea
            id={promptId}
            label={t("presets.editor.prompt")}
            rows={8}
            value={draft.prompt}
            aria-describedby={`${promptId}-help`}
            aria-invalid={shown(problems.prompt) === undefined ? undefined : true}
            onChange={(e) => {
              update({ prompt: e.target.value });
            }}
          />
          <div
            id={`${promptId}-help`}
            className="flex items-start justify-between gap-3 text-[12px]">
            <span
              className={shown(problems.prompt) === undefined ? "text-fg-subtle" : "text-danger"}>
              {shown(problems.prompt) ?? t("presets.editor.promptHelp")}
            </span>
            <span
              className={`mono shrink-0 text-[11px] ${chars > MAX_PRESET_PROMPT_CHARS ? "text-danger" : "text-fg-subtle"}`}
              data-testid="preset-prompt-count">
              {t("presets.editor.count", { n: chars, max: MAX_PRESET_PROMPT_CHARS })}
            </span>
          </div>
        </div>

        <section
          className="flex flex-col gap-2 rounded-10 bg-inset p-3"
          aria-label={t("presets.editor.trial.title")}
          data-testid="preset-trial">
          <div className="flex items-center justify-between gap-3">
            <span className="text-[13px] font-medium text-fg">
              {t("presets.editor.trial.title")}
            </span>
            <Button
              size="sm"
              variant="outline"
              icon="play"
              disabled={!canTry}
              loading={trial.pending}
              onClick={() => {
                trial.run({ prompt: draft.prompt }, sample.trim());
              }}>
              {trial.pending ? t("presets.editor.trial.running") : t("presets.editor.trial.run")}
            </Button>
          </div>
          <p className="text-[12px] text-fg-muted">
            {ready ? t("presets.editor.trial.help") : t("presets.editor.trial.unavailable")}
          </p>
          <Textarea
            label={t("presets.editor.trial.sample")}
            rows={3}
            value={sample}
            onChange={(e) => {
              setSample(e.target.value);
              trial.reset();
            }}
          />
          {sampleIssue !== undefined && sample.trim().length > 0 && (
            <span className="text-[12px] text-danger">{sampleIssue}</span>
          )}
          {outcome?.status === "ok" && (
            <div className="flex flex-col gap-1" data-testid="preset-trial-result">
              <span className="mono text-[11px] text-fg-subtle">
                {t("presets.editor.trial.result", {
                  model: outcome.model,
                  time: formatMs(outcome.latency_ms),
                })}
              </span>
              <div
                className="rounded-6 bg-surface p-3 text-[13px] leading-5 whitespace-pre-wrap text-fg hairline"
                data-user-text>
                {outcome.text}
              </div>
            </div>
          )}
          {outcome?.status === "failed" && (
            <p className="text-[12px] text-danger" role="alert" data-testid="preset-trial-error">
              {t("presets.editor.trial.failed", { reason: outcome.reason })}
            </p>
          )}
        </section>

        {saveError !== undefined && (
          <div
            className="mono text-[12px] whitespace-pre-wrap text-danger"
            role="alert"
            data-testid="preset-editor-error">
            {saveError}
          </div>
        )}
      </div>
    </Dialog>
  );
}
