import {
  type AppRef,
  MAX_SCENE_PROMPT_CHARS,
  type Scene,
  isStreamingOutputMode,
  normalizeAppId,
  sceneLabel,
} from "@voltip/shared";
import {
  Button,
  Chip,
  Dialog,
  IconButton,
  Input,
  Select,
  Textarea,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { type KeyboardEvent, useEffect, useId, useState } from "react";
import { useShell } from "../../../app/shell-context";
import { errorText } from "../../../features/vocabulary/vocabulary";
import {
  type EditorDraft,
  type EditorProblems,
  addApp,
  addKeyword,
  editorDraftFrom,
  editorProblems,
  hasProblems,
  languageChoices,
  outputModeChoices,
  presetChoices,
  promptChars,
  refineChoices,
  sceneDraftOf,
  scriptChoices,
} from "./helpers";

export interface SceneEditorProps {
  /** The scene being edited; absent for a new one. */
  scene?: Scene;
  onClose: () => void;
}

/** A removable chip: the user's text (an app id or a title keyword) and an ✕ button. */
function RemovableChip({
  text,
  label,
  mono,
  onRemove,
}: {
  text: string;
  label: string;
  mono: boolean;
  onRemove: () => void;
}) {
  return (
    <li className="inline-flex h-7 items-center gap-0.5 rounded-6 bg-surface pr-0.5 pl-2.5 text-[12px] text-fg hairline">
      <span className={mono ? "mono" : undefined} data-user-text>
        {text}
      </span>
      <IconButton icon="x" label={label} onClick={onRemove} />
    </li>
  );
}

/** Enter adds what was typed (and never submits anything else). */
function onEnter(add: () => void) {
  return (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key !== "Enter") return;
    e.preventDefault();
    add();
  };
}

/** Creates or edits one scene (docs/dictation.md §18.1): the name, the apps (typed ids or picked
 *  from 最近的应用, which is `recent_apps` over the history), the window-title keywords, every
 *  override with its 跟随全局 default, and the extra instruction for the AI. The instant checks are
 *  local; the core validates the draft again on `scenes_add` / `scenes_update` and a refusal is
 *  shown here, the dialog staying open. */
export function SceneEditor({ scene, onClose }: SceneEditorProps) {
  const { backend } = useBackend();
  const shell = useShell();
  const { t, locale } = useI18n();
  const state = useUiState();
  const presets = state.presets;
  const promptId = useId();
  const [draft, setDraft] = useState<EditorDraft>(() => editorDraftFrom(scene));
  const [appInput, setAppInput] = useState("");
  const [keywordInput, setKeywordInput] = useState("");
  const [recent, setRecent] = useState<AppRef[] | undefined>(undefined);
  const [attempted, setAttempted] = useState(false);
  const [saveError, setSaveError] = useState<string | undefined>(undefined);

  useEffect(() => {
    let alive = true;
    backend.recentApps().then(
      (apps) => {
        if (alive) setRecent(apps);
      },
      () => {
        if (alive) setRecent([]);
      },
    );
    return () => {
      alive = false;
    };
  }, [backend]);

  // A built-in scene (§18.10) keeps its name and may list no application.
  const builtin = scene?.builtin !== undefined;
  const problems = editorProblems(
    draft,
    state.scenes.filter((s) => s.id !== scene?.id),
    t,
    builtin,
  );
  const shown = (p: EditorProblems[keyof EditorProblems]) =>
    p !== undefined && (attempted || !p.missing) ? p.text : undefined;
  const update = (patch: Partial<EditorDraft>) => {
    setDraft((d) => ({ ...d, ...patch }));
    setSaveError(undefined);
  };
  const commitApp = () => {
    if (appInput.trim().length === 0) return;
    update({ apps: addApp(draft.apps, appInput) });
    setAppInput("");
  };
  const commitKeyword = () => {
    if (keywordInput.trim().length === 0) return;
    update({ keywords: addKeyword(draft.keywords, keywordInput) });
    setKeywordInput("");
  };
  const toggleRecent = (app: AppRef) => {
    const id = normalizeAppId(app.id);
    update({
      apps: draft.apps.includes(id) ? draft.apps.filter((a) => a !== id) : addApp(draft.apps, id),
    });
  };
  const save = async () => {
    setAttempted(true);
    if (hasProblems(problems)) return;
    const payload = sceneDraftOf(draft);
    try {
      if (scene === undefined) await backend.invoke("scenes_add", { scene: payload });
      else await backend.invoke("scenes_update", { id: scene.id, scene: payload });
      shell.toast({ message: t("sceneEditor.saved", { name: payload.name }), duration: 3000 });
      onClose();
    } catch (e) {
      setSaveError(errorText(e));
    }
  };
  const restore = () => {
    if (scene === undefined) return;
    const name = sceneLabel(scene, locale);
    shell.confirm({
      title: t("sceneEditor.restoreTitle", { name }),
      body: t("sceneEditor.restoreBody"),
      confirmLabel: t("sceneEditor.restore"),
      tone: "primary",
      onConfirm: () => {
        backend.invoke("scenes_restore", { id: scene.id }).then(
          () => {
            shell.toast({ message: t("sceneEditor.restored", { name }), duration: 3000 });
            onClose();
          },
          (e: unknown) => {
            setSaveError(errorText(e));
          },
        );
      },
    });
  };
  const streamingNotReady =
    draft.outputMode !== "" &&
    isStreamingOutputMode(draft.outputMode) &&
    !state.engines.live_preview_ready;
  const chars = promptChars(draft.prompt);

  return (
    <Dialog
      open
      title={scene === undefined ? t("sceneEditor.titleNew") : t("sceneEditor.titleEdit")}
      width={640}
      onClose={onClose}
      actions={
        <>
          {builtin && (
            <Button
              size="sm"
              variant="ghost"
              icon="refresh"
              className="mr-auto"
              data-testid="scene-restore"
              onClick={restore}>
              {t("sceneEditor.restore")}
            </Button>
          )}
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
            {t("common.save")}
          </Button>
        </>
      }>
      <div
        className="flex max-h-[60vh] flex-col gap-4 overflow-y-auto pr-1"
        data-testid="scene-editor"
        onKeyDown={(e) => {
          if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
            e.preventDefault();
            void save();
          }
        }}>
        {builtin && scene !== undefined ? (
          // The stored name is the category; the interface names it in its own language.
          <Input
            label={t("sceneEditor.name")}
            size="sm"
            value={sceneLabel(scene, locale)}
            readOnly
            help={t("sceneEditor.builtinName")}
            data-testid="scene-builtin-name"
          />
        ) : (
          <Input
            label={t("sceneEditor.name")}
            size="sm"
            value={draft.name}
            placeholder={t("sceneEditor.namePlaceholder")}
            error={shown(problems.name)}
            data-autofocus
            onChange={(e) => {
              update({ name: e.target.value });
            }}
          />
        )}

        <div className="flex flex-col gap-2" data-testid="scene-editor-apps">
          <div className="flex items-start gap-2">
            <Input
              label={t("sceneEditor.apps")}
              mono
              size="sm"
              className="flex-1"
              value={appInput}
              placeholder={t("sceneEditor.appPlaceholder")}
              error={shown(problems.apps)}
              help={
                builtin
                  ? `${t("sceneEditor.appsHelp")} ${t("sceneEditor.builtinApps")}`
                  : t("sceneEditor.appsHelp")
              }
              onChange={(e) => {
                setAppInput(e.target.value);
              }}
              onKeyDown={onEnter(commitApp)}
            />
            <Button
              size="sm"
              className="mt-5"
              disabled={appInput.trim().length === 0}
              onClick={commitApp}>
              {t("sceneEditor.add")}
            </Button>
          </div>
          {draft.apps.length > 0 && (
            <ul aria-label={t("sceneEditor.apps")} className="flex flex-wrap gap-1.5">
              {draft.apps.map((id) => (
                <RemovableChip
                  key={id}
                  text={id}
                  mono
                  label={t("sceneEditor.removeApp", { app: id })}
                  onRemove={() => {
                    update({ apps: draft.apps.filter((a) => a !== id) });
                  }}
                />
              ))}
            </ul>
          )}
          <div className="flex flex-col gap-1.5">
            <span className="text-[12px] text-fg-muted">{t("sceneEditor.recent")}</span>
            {recent !== undefined &&
              (recent.length === 0 ? (
                <span className="text-[12px] text-fg-subtle">{t("sceneEditor.recentEmpty")}</span>
              ) : (
                <div
                  role="group"
                  aria-label={t("sceneEditor.recent")}
                  className="flex flex-wrap gap-1.5">
                  {recent.map((app) => (
                    <Chip
                      key={app.id}
                      active={draft.apps.includes(normalizeAppId(app.id))}
                      title={app.id}
                      onClick={() => {
                        toggleRecent(app);
                      }}>
                      <span data-user-text>{app.name}</span>
                    </Chip>
                  ))}
                </div>
              ))}
          </div>
        </div>

        <div className="flex flex-col gap-2" data-testid="scene-editor-keywords">
          <div className="flex items-start gap-2">
            <Input
              label={t("sceneEditor.keywords")}
              size="sm"
              className="flex-1"
              value={keywordInput}
              placeholder={t("sceneEditor.keywordPlaceholder")}
              error={shown(problems.keywords)}
              help={t("sceneEditor.keywordsHelp")}
              onChange={(e) => {
                setKeywordInput(e.target.value);
              }}
              onKeyDown={onEnter(commitKeyword)}
            />
            <Button
              size="sm"
              className="mt-5"
              disabled={keywordInput.trim().length === 0}
              onClick={commitKeyword}>
              {t("sceneEditor.add")}
            </Button>
          </div>
          {draft.keywords.length > 0 && (
            <ul aria-label={t("sceneEditor.keywords")} className="flex flex-wrap gap-1.5">
              {draft.keywords.map((keyword) => (
                <RemovableChip
                  key={keyword}
                  text={keyword}
                  mono={false}
                  label={t("sceneEditor.removeKeyword", { keyword })}
                  onRemove={() => {
                    update({ keywords: draft.keywords.filter((k) => k !== keyword) });
                  }}
                />
              ))}
            </ul>
          )}
        </div>

        <fieldset className="flex flex-col gap-3" data-testid="scene-editor-overrides">
          <legend className="eyebrow mb-2">{t("sceneEditor.overrides")}</legend>
          <div className="grid grid-cols-2 gap-3">
            <Select
              label={t("sceneEditor.refine")}
              size="sm"
              value={draft.refine}
              options={refineChoices(t)}
              onChange={(refine) => {
                update({ refine });
              }}
            />
            <Select
              label={t("sceneEditor.preset")}
              size="sm"
              value={draft.preset}
              options={presetChoices(presets, draft.preset, t)}
              onChange={(preset) => {
                update({ preset });
              }}
              data-testid="scene-preset"
              // Custom presets are named by the user.
              {...(presets.length > 0 ? { "data-user-text": "" } : {})}
            />
            <div className="flex flex-col gap-1">
              <Select
                label={t("sceneEditor.outputMode")}
                size="sm"
                value={draft.outputMode}
                options={outputModeChoices(t, locale)}
                onChange={(outputMode) => {
                  update({ outputMode });
                }}
              />
              {streamingNotReady && (
                <span className="text-[12px] text-warning" data-testid="scene-streaming-note">
                  {t("sceneEditor.streamingNotReady")}
                </span>
              )}
            </div>
            <Select
              label={t("sceneEditor.language")}
              size="sm"
              value={draft.language}
              options={languageChoices(draft.language, t)}
              onChange={(language) => {
                update({ language });
              }}
            />
            <Select
              label={t("sceneEditor.script")}
              size="sm"
              value={draft.script}
              options={scriptChoices(t)}
              onChange={(script) => {
                update({ script });
              }}
            />
          </div>
          <div className="flex flex-col gap-1">
            <Textarea
              id={promptId}
              label={t("sceneEditor.prompt")}
              rows={3}
              value={draft.prompt}
              aria-describedby={`${promptId}-help`}
              aria-invalid={problems.prompt === undefined ? undefined : true}
              onChange={(e) => {
                update({ prompt: e.target.value });
              }}
            />
            <div
              id={`${promptId}-help`}
              className="flex items-start justify-between gap-3 text-[12px]">
              <span className={problems.prompt === undefined ? "text-fg-subtle" : "text-danger"}>
                {shown(problems.prompt) ?? t("sceneEditor.promptHelp")}
              </span>
              <span
                className={`mono shrink-0 text-[11px] ${chars > MAX_SCENE_PROMPT_CHARS ? "text-danger" : "text-fg-subtle"}`}
                data-testid="scene-prompt-count">
                {t("sceneEditor.promptCount", { n: chars, max: MAX_SCENE_PROMPT_CHARS })}
              </span>
            </div>
          </div>
        </fieldset>

        {saveError !== undefined && (
          <div
            className="mono text-[12px] whitespace-pre-wrap text-danger"
            role="alert"
            data-testid="scene-editor-error">
            {saveError}
          </div>
        )}
      </div>
    </Dialog>
  );
}
