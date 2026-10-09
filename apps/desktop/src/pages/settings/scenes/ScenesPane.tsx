import { type ContextSharing, MAX_SCENES, type Scene } from "@voltip/shared";
import {
  Button,
  SceneCards,
  SceneEditor,
  SettingsPane,
  SettingsRows,
  SettingsSection,
  StatusRow,
  Toggle,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { useCallback, useState } from "react";
import { useShell } from "../../../app/shell-context";
import { errorText } from "../../../features/vocabulary/vocabulary";

/** Settings · 场景 (docs/dictation.md §18): what goes to AI polish as context (the app name, on by
 *  default; the window title, off by default — `settings_set_context_sharing`), and the scenes in
 *  matching order (`state.scenes`, the cards and the editor shared with the phone in
 *  `@voltip/ui`). The core owns the list; every change comes back as a `scenes` event. */
export function ScenesPane() {
  const { backend } = useBackend();
  const shell = useShell();
  const { t } = useI18n();
  const state = useUiState();
  const own = state.scenes.filter((s) => s.builtin === undefined).length;
  const sharing = state.settings.context_sharing;
  // `{}` is a new scene, `{ scene }` an existing one; the dialog's `onClose` stays stable.
  const [editing, setEditing] = useState<{ scene?: Scene } | undefined>(undefined);
  const closeEditor = useCallback(() => {
    setEditing(undefined);
  }, []);

  const run = (action: Promise<void>) => {
    action.catch((e: unknown) => {
      shell.toast({
        message: t("app.error", { message: errorText(e) }),
        duration: 5000,
        tone: "danger",
      });
    });
  };
  const share = (patch: Partial<ContextSharing>) => {
    const next = { ...sharing, ...patch };
    run(
      backend.invoke("settings_set_context_sharing", {
        appName: next.app_name,
        windowTitle: next.window_title,
      }),
    );
  };
  return (
    <SettingsPane
      title={t("settings.scenes.title")}
      lede={t("settings.scenes.lede")}
      data-testid="scenes-pane">
      <SettingsSection
        title={t("settings.scenes.context.title")}
        description={t("settings.scenes.context.lede")}
        data-testid="context-sharing">
        <SettingsRows>
          <StatusRow
            label={t("settings.scenes.context.appName")}
            help={t("settings.scenes.context.appNameHelp")}>
            <Toggle
              checked={sharing.app_name}
              ariaLabel={t("settings.scenes.context.appName")}
              onChange={(app_name) => {
                share({ app_name });
              }}
            />
          </StatusRow>
          <StatusRow
            label={t("settings.scenes.context.windowTitle")}
            help={t("settings.scenes.context.windowTitleHelp")}>
            <Toggle
              checked={sharing.window_title}
              ariaLabel={t("settings.scenes.context.windowTitle")}
              onChange={(window_title) => {
                share({ window_title });
              }}
            />
          </StatusRow>
        </SettingsRows>
      </SettingsSection>

      <SettingsSection
        title={t("settings.scenes.listLabel")}
        data-testid="scene-list"
        aside={
          <Button
            size="sm"
            variant="primary"
            icon="plus"
            disabled={own >= MAX_SCENES}
            onClick={() => {
              setEditing({});
            }}>
            {t("settings.scenes.add")}
          </Button>
        }>
        <SceneCards
          onEdit={(scene) => {
            setEditing({ scene });
          }}
        />
        <div className="mono text-[11px] text-fg-subtle">
          {t("settings.scenes.footnote", { limit: MAX_SCENES })}
        </div>
      </SettingsSection>

      {editing !== undefined && (
        <SceneEditor key={editing.scene?.id ?? "new"} scene={editing.scene} onClose={closeEditor} />
      )}
    </SettingsPane>
  );
}
