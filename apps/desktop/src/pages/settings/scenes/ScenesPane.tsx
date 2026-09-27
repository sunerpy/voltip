import { type ContextSharing, MAX_SCENES, type Scene } from "@voltip/shared";
import {
  Button,
  Card,
  Chip,
  EmptyState,
  IconButton,
  SettingsPane,
  SettingsRows,
  SettingsSection,
  StatusRow,
  Toggle,
  cx,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { useCallback, useState } from "react";
import { useShell } from "../../../app/shell-context";
import { errorText, moved } from "../../../features/vocabulary/vocabulary";
import { overrideSummary, withEnabled } from "./helpers";
import { SceneEditor } from "./SceneEditor";

interface SceneCardProps {
  scene: Scene;
  index: number;
  count: number;
  onToggle: (enabled: boolean) => void;
  onMove: (delta: -1 | 1) => void;
  onEdit: () => void;
  onRemove: () => void;
}

/** One scene: its place in the matching order, name, switch and actions; the apps it matches and
 *  its title keywords; the overrides it sets (or 全部跟随全局设置). */
function SceneCard({ scene, index, count, onToggle, onMove, onEdit, onRemove }: SceneCardProps) {
  const { t, locale } = useI18n();
  const name = scene.name;
  const summary = overrideSummary(scene.overrides, t, locale);
  return (
    <Card
      padding="sm"
      role="article"
      aria-label={name}
      data-testid="scene-card"
      data-enabled={scene.enabled}
      className={cx("flex flex-col gap-2", !scene.enabled && "opacity-60")}>
      <div className="flex items-center gap-2">
        <span className="mono w-5 shrink-0 text-right text-[11px] text-fg-subtle">{index + 1}</span>
        <span className="min-w-0 flex-1 truncate text-[14px] font-medium text-fg" data-user-text>
          {name}
        </span>
        <Toggle
          checked={scene.enabled}
          onChange={onToggle}
          ariaLabel={t("settings.scenes.enabled", { name })}
        />
        <span className="inline-flex gap-0.5">
          <IconButton
            icon="chevronUp"
            label={t("settings.scenes.moveUp", { name })}
            disabled={index === 0}
            onClick={() => {
              onMove(-1);
            }}
          />
          <IconButton
            icon="chevronDown"
            label={t("settings.scenes.moveDown", { name })}
            disabled={index === count - 1}
            onClick={() => {
              onMove(1);
            }}
          />
          <IconButton icon="edit" label={t("settings.scenes.edit", { name })} onClick={onEdit} />
          <IconButton
            icon="trash"
            tone="danger"
            label={t("settings.scenes.remove", { name })}
            onClick={onRemove}
          />
        </span>
      </div>
      <div className="flex flex-wrap items-center gap-1.5 pl-7" data-testid="scene-match">
        {scene.match.apps.map((id) => (
          <Chip key={id}>
            <span className="mono" data-user-text>
              {id}
            </span>
          </Chip>
        ))}
        <span className="text-[12px] text-fg-muted">
          {scene.match.title_contains.length === 0 ? (
            t("settings.scenes.anyWindow")
          ) : (
            <>
              {t("settings.scenes.titleKeywords")}{" "}
              <span className="text-fg" data-user-text>
                {scene.match.title_contains.join(" · ")}
              </span>
            </>
          )}
        </span>
      </div>
      <div className="pl-7 text-[12px] text-fg-muted" data-testid="scene-summary">
        {summary.length === 0 ? t("settings.scenes.followsGlobal") : summary.join(" · ")}
      </div>
    </Card>
  );
}

/** Settings · 场景 (docs/dictation.md §18): what goes to AI polish as context (the app name, on by
 *  default; the window title, off by default — `settings_set_context_sharing`), and the scenes in
 *  matching order (`state.scenes`): each card switches its scene on / off, moves it
 *  (`scenes_reorder`), edits it in the scene editor or deletes it after a confirm. The core owns the
 *  list; every change comes back as a `scenes` event. */
export function ScenesPane() {
  const { backend } = useBackend();
  const shell = useShell();
  const { t } = useI18n();
  const state = useUiState();
  const scenes = state.scenes;
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
  const move = (scene: Scene, delta: -1 | 1) => {
    const ids = scenes.map((s) => s.id);
    run(backend.invoke("scenes_reorder", { ids: moved(ids, ids.indexOf(scene.id), delta) }));
  };
  const remove = (scene: Scene) => {
    shell.confirm({
      title: t("settings.scenes.confirmRemove.title", { name: scene.name }),
      body: t("settings.scenes.confirmRemove.body"),
      confirmLabel: t("common.delete"),
      tone: "danger",
      onConfirm: () => {
        run(backend.invoke("scenes_remove", { id: scene.id }));
      },
    });
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
            disabled={scenes.length >= MAX_SCENES}
            onClick={() => {
              setEditing({});
            }}>
            {t("settings.scenes.add")}
          </Button>
        }>
        {scenes.length === 0 ? (
          <EmptyState compact title={t("settings.scenes.emptyTitle")}>
            {t("settings.scenes.emptyBody")}
          </EmptyState>
        ) : (
          <ol aria-label={t("settings.scenes.listLabel")} className="flex flex-col gap-2">
            {scenes.map((scene, index) => (
              <li key={scene.id}>
                <SceneCard
                  scene={scene}
                  index={index}
                  count={scenes.length}
                  onToggle={(enabled) => {
                    run(
                      backend.invoke("scenes_update", {
                        id: scene.id,
                        scene: withEnabled(scene, enabled),
                      }),
                    );
                  }}
                  onMove={(delta) => {
                    move(scene, delta);
                  }}
                  onEdit={() => {
                    setEditing({ scene });
                  }}
                  onRemove={() => {
                    remove(scene);
                  }}
                />
              </li>
            ))}
          </ol>
        )}
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
