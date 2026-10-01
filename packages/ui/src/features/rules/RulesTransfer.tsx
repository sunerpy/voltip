import { type ImportMode, errorText } from "@voltip/shared";
import { type ReactNode, useState } from "react";
import { useBackend } from "../../backend/BackendProvider";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { Textarea } from "../../components/Input";
import { Segmented } from "../../components/Segmented";
import { useI18n } from "../../i18n/I18nProvider";
import { useFeatureShell } from "../shell";

/** Paste a TOML text, pick merge or replace (`rules_import`); the core parses and validates the
 *  whole file synchronously, so a refusal comes back here with its position and nothing is
 *  imported. Shared by the desktop's rules page and the phone's (docs/dictation.md §16). */
export function RulesImportDialog({ onClose }: { onClose: () => void }) {
  const { backend } = useBackend();
  const shell = useFeatureShell();
  const { t } = useI18n();
  const [text, setText] = useState("");
  const [mode, setMode] = useState<ImportMode>("merge");
  const [error, setError] = useState<string | undefined>(undefined);
  const modeLabel =
    mode === "merge" ? t("rules.importDialog.merge") : t("rules.importDialog.replace");
  const submit = async () => {
    try {
      await backend.invoke("rules_import", { toml: text, mode });
      shell.notify(t("rules.importDialog.done", { mode: modeLabel }));
      onClose();
    } catch (e) {
      setError(errorText(e));
    }
  };
  return (
    <Dialog
      open
      title={t("rules.importDialog.title")}
      width={600}
      onClose={onClose}
      actions={
        <>
          <Button size="sm" variant="ghost" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button
            size="sm"
            variant="primary"
            disabled={text.trim().length === 0}
            onClick={() => {
              void submit();
            }}>
            {t("rules.importDialog.import")}
          </Button>
        </>
      }>
      <div className="flex flex-col gap-3">
        <p>{t("rules.importDialog.body")}</p>
        <Textarea
          aria-label={t("rules.importDialog.label")}
          mono
          rows={12}
          value={text}
          data-autofocus
          onChange={(e) => {
            setText(e.target.value);
            setError(undefined);
          }}
        />
        <div className="flex flex-wrap items-center gap-3">
          <Segmented
            label={t("rules.importDialog.mode")}
            size="sm"
            value={mode}
            onChange={setMode}
            options={[
              { value: "merge", label: t("rules.importDialog.merge") },
              { value: "replace", label: t("rules.importDialog.replace") },
            ]}
          />
          <span className="text-[12px] text-fg-subtle">
            {mode === "merge"
              ? t("rules.importDialog.mergeHelp")
              : t("rules.importDialog.replaceHelp")}
          </span>
        </div>
        {error !== undefined && (
          <pre
            className="mono max-h-40 overflow-auto whitespace-pre-wrap text-[12px] text-danger"
            role="alert"
            data-testid="rules-import-error">
            {error}
          </pre>
        )}
      </div>
    </Dialog>
  );
}

/** The rules as TOML (`rules_export`), read-only, with the app's own ways to take the text out:
 *  the desktop copies it, the phone copies or shares it. */
export function RulesExportDialog({
  text,
  onClose,
  actions,
}: {
  text: string;
  onClose: () => void;
  /** The buttons after 关闭. */
  actions: ReactNode;
}) {
  const { t } = useI18n();
  return (
    <Dialog
      open
      title={t("rules.exportDialog.title")}
      width={600}
      onClose={onClose}
      actions={
        <>
          <Button size="sm" variant="ghost" onClick={onClose} data-autofocus>
            {t("common.close")}
          </Button>
          {actions}
        </>
      }>
      <p className="mb-2">{t("rules.exportDialog.body")}</p>
      <Textarea
        aria-label={t("rules.exportDialog.label")}
        mono
        rows={14}
        readOnly
        value={text}
        data-testid="rules-export-text"
      />
    </Dialog>
  );
}
