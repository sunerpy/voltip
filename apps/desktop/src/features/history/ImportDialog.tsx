import { baseName, languageOptions, skippedFilesText } from "@voltip/shared";
import { Button, Dialog, Select, Toggle, useBackend, useI18n, useUiState } from "@voltip/ui";
import { useId, useState } from "react";
import { AI_ROUTE, SPEECH_ROUTE, useRouter } from "../../app/router";
import { importOptions, polishOf, recognitionOf } from "./import-options";

/** The names the dialog lists before 还有 n 个文件. */
const SHOWN_FILES = 4;

/** 导入音频或视频's options (docs/dictation.md §31): the files, the speech model, the language and
 *  the polish, which is off until it is turned on. Only an installed local model, a cloud provider
 *  with the user's own key, or Ollama and the custom endpoint as the user set them up are offered
 *  (§31.4); the settings stay as they are. */
export function ImportDialog({
  paths,
  skipped,
  onClose,
}: {
  paths: readonly string[];
  /** The names of dropped files that are not audio or video: said here, where a toast would be
   *  covered by the dialog. */
  skipped: readonly string[];
  onClose: () => void;
}) {
  const { backend } = useBackend();
  const { t, locale } = useI18n();
  const { navigate } = useRouter();
  const state = useUiState();
  const options = importOptions(state.engines, state.models, t, locale);
  const [speech, setSpeech] = useState(options.speechDefault ?? "");
  const [language, setLanguage] = useState(state.settings.engines.language ?? "");
  const [polishOn, setPolishOn] = useState(false);
  const [polish, setPolish] = useState(options.polishDefault ?? "");
  const languageId = useId();
  // A model that went away meanwhile (its key was removed) gives way to the list's first.
  const speechId = options.speech.some((o) => o.value === speech) ? speech : options.speechDefault;
  const asr = speechId === undefined ? undefined : recognitionOf(speechId);
  const polishId = options.polish.some((o) => o.value === polish) ? polish : options.polishDefault;
  const refine = polishOn && polishId !== undefined ? polishOf(polishId) : undefined;
  const ready = asr !== undefined && (!polishOn || refine !== undefined);
  const manage = (route: typeof SPEECH_ROUTE) => {
    onClose();
    navigate(route);
  };
  const start = () => {
    if (asr === undefined) return;
    void backend.invoke("import_add", {
      paths: [...paths],
      asr,
      ...(refine === undefined ? {} : { refine }),
      ...(language === "" ? {} : { language }),
    });
    onClose();
  };
  const hidden = paths.length - SHOWN_FILES;
  return (
    <Dialog
      open
      title={t("history.imports.dialog.title")}
      width={520}
      onClose={onClose}
      actions={
        <>
          <Button size="sm" variant="ghost" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button
            size="sm"
            variant="primary"
            icon="upload"
            disabled={!ready}
            data-autofocus
            data-testid="import-start"
            onClick={start}>
            {t("history.imports.dialog.start")}
          </Button>
        </>
      }>
      <div className="flex flex-col gap-4" data-testid="import-dialog">
        <div className="flex flex-col gap-1">
          <span className="text-[12px] text-fg-muted">
            {t("history.imports.dialog.files", { n: paths.length })}
          </span>
          <ul className="flex flex-col gap-0.5 text-[13px] text-fg" data-testid="import-files">
            {paths.slice(0, SHOWN_FILES).map((path) => (
              <li key={path} className="truncate" title={path} data-user-text>
                {baseName(path)}
              </li>
            ))}
          </ul>
          {hidden > 0 && (
            <span className="text-[12px] text-fg-subtle">
              {t("history.imports.dialog.more", { n: hidden })}
            </span>
          )}
          {skipped.length > 0 && (
            <span className="text-[12px] text-fg-muted" data-testid="import-skipped">
              {skippedFilesText(skipped, t)}
            </span>
          )}
        </div>
        <div className="grid grid-cols-[auto_minmax(0,1fr)] items-center gap-x-4 gap-y-3">
          <span className="text-[13px] text-fg-muted">{t("history.imports.dialog.speech")}</span>
          {options.speech.length === 0 ? (
            <div className="flex flex-col items-start gap-1" data-testid="import-no-speech">
              <span className="text-[12px] text-fg-muted">
                {t("history.imports.dialog.noSpeech")}
              </span>
              <Button
                size="sm"
                variant="text"
                onClick={() => {
                  manage(SPEECH_ROUTE);
                }}>
                {t("history.imports.dialog.manageSpeech")}
              </Button>
            </div>
          ) : (
            <Select
              aria-label={t("history.imports.dialog.speech")}
              size="sm"
              wrap
              value={speechId ?? ""}
              options={options.speech}
              onChange={setSpeech}
              data-testid="import-speech"
            />
          )}
          <label htmlFor={languageId} className="text-[13px] text-fg-muted">
            {t("history.imports.dialog.language")}
          </label>
          <Select
            id={languageId}
            size="sm"
            value={language}
            options={languageOptions(t)}
            onChange={setLanguage}
            data-testid="import-language"
          />
          <span className="text-[13px] text-fg-muted">{t("history.imports.dialog.polish")}</span>
          <Toggle
            checked={polishOn}
            onChange={setPolishOn}
            ariaLabel={t("history.imports.dialog.polish")}
            label={
              <span className="text-[12px] text-fg-subtle">
                {t("history.imports.dialog.polishHint")}
              </span>
            }
            id="import-polish"
          />
          {polishOn && (
            <>
              <span className="text-[13px] text-fg-muted">
                {t("history.imports.dialog.polishModel")}
              </span>
              {options.polish.length === 0 ? (
                <div className="flex flex-col items-start gap-1" data-testid="import-no-polish">
                  <span className="text-[12px] text-fg-muted">
                    {t("history.imports.dialog.noPolish")}
                  </span>
                  <Button
                    size="sm"
                    variant="text"
                    onClick={() => {
                      manage(AI_ROUTE);
                    }}>
                    {t("history.imports.dialog.manageAi")}
                  </Button>
                </div>
              ) : (
                <Select
                  aria-label={t("history.imports.dialog.polishModel")}
                  size="sm"
                  wrap
                  value={polishId ?? ""}
                  options={options.polish}
                  onChange={setPolish}
                  data-testid="import-polish-model"
                />
              )}
            </>
          )}
        </div>
        <p className="text-[12px] leading-[18px] text-fg-subtle">
          {t("history.imports.dialog.note")}
        </p>
      </div>
    </Dialog>
  );
}
