import {
  type BuiltinSceneTerms,
  type Scene,
  errorText,
  movedBy,
  sceneLabel,
  sceneOverrideSummary,
  sceneWithEnabled,
} from "@voltip/shared";
import { useEffect, useState } from "react";
import { useBackend, useUiState } from "../../backend/BackendProvider";
import { Badge } from "../../components/Badge";
import { Button } from "../../components/Button";
import { Card } from "../../components/Card";
import { Chip } from "../../components/Chip";
import { Dialog } from "../../components/Dialog";
import { EmptyState } from "../../components/EmptyState";
import { IconButton } from "../../components/IconButton";
import { Toggle } from "../../components/Toggle";
import { useI18n } from "../../i18n/I18nProvider";
import { cx } from "../../cx";
import { useFeatureShell } from "../shell";

interface SceneCardProps {
  scene: Scene;
  index: number;
  count: number;
  matchApps: boolean;
  /** A built-in scene's term pack (§18.10), once `scenes_builtin` answered. */
  terms?: readonly string[];
  onToggle: (enabled: boolean) => void;
  onMove: (delta: -1 | 1) => void;
  onEdit: () => void;
  onRemove: () => void;
  onTerms: () => void;
}

/** One scene: its name and actions, and the overrides it sets (or 全部跟随全局设置). Where scenes are
 *  matched (the desktop) also its place in the matching order, its switch, the apps it matches and
 *  its title keywords. A built-in scene (§18.10) is named in the interface's language with a 内置
 *  badge and its term count, has no delete button, and says when it waits for an application. */
function SceneCard({
  scene,
  index,
  count,
  matchApps,
  terms,
  onToggle,
  onMove,
  onEdit,
  onRemove,
  onTerms,
}: SceneCardProps) {
  const { t, locale } = useI18n();
  const presets = useUiState().presets;
  const builtin = scene.builtin;
  const name = sceneLabel(scene, locale);
  const summary = sceneOverrideSummary(scene.overrides, t, locale, presets);
  const indent = matchApps ? "pl-7" : undefined;
  return (
    <Card
      padding="sm"
      role="article"
      aria-label={name}
      data-testid="scene-card"
      data-enabled={scene.enabled}
      className={cx("flex flex-col gap-2", matchApps && !scene.enabled && "opacity-60")}>
      <div className="flex items-center gap-2">
        {matchApps && (
          <span className="mono w-5 shrink-0 text-right text-[11px] text-fg-subtle">
            {index + 1}
          </span>
        )}
        <span className="flex min-w-0 flex-1 items-center gap-2">
          <span
            className="truncate text-[14px] font-medium text-fg"
            {...(builtin === undefined ? { "data-user-text": "" } : {})}>
            {name}
          </span>
          {builtin !== undefined && (
            <Badge tone="neutral" className="shrink-0">
              {t("settings.scenes.builtinBadge")}
            </Badge>
          )}
        </span>
        {matchApps && (
          <Toggle
            checked={scene.enabled}
            onChange={onToggle}
            ariaLabel={t("settings.scenes.enabled", { name })}
          />
        )}
        <span className="inline-flex gap-0.5">
          {matchApps && (
            <>
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
            </>
          )}
          <IconButton icon="edit" label={t("settings.scenes.edit", { name })} onClick={onEdit} />
          {builtin === undefined && (
            <IconButton
              icon="trash"
              tone="danger"
              label={t("settings.scenes.remove", { name })}
              onClick={onRemove}
            />
          )}
        </span>
      </div>
      {builtin !== undefined && (
        <div
          className={cx(indent, "text-[12px] text-fg-muted")}
          data-testid="scene-builtin-description">
          {t(`builtinScenes.${builtin}.description`)}
        </div>
      )}
      {matchApps && (
        <div className="flex flex-wrap items-center gap-1.5 pl-7" data-testid="scene-match">
          {builtin !== undefined && scene.match.apps.length === 0 && (
            <span className="text-[12px] text-warning" data-testid="scene-needs-apps">
              {t("settings.scenes.needsApps")}
            </span>
          )}
          {scene.match.apps.map((id) => (
            <Chip key={id}>
              <span className="mono" data-user-text>
                {id}
              </span>
            </Chip>
          ))}
          {/* A scene that waits for an app names no window either. */}
          {scene.match.apps.length > 0 && (
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
          )}
        </div>
      )}
      <div className={cx(indent, "text-[12px] text-fg-muted")} data-testid="scene-summary">
        {summary.length === 0 ? t("settings.scenes.followsGlobal") : summary.join(" · ")}
      </div>
      {terms !== undefined && terms.length > 0 && (
        <div
          className={cx(indent, "flex items-center gap-2 text-[12px] text-fg-muted")}
          data-testid="scene-terms">
          <span>{t("settings.scenes.terms", { n: terms.length })}</span>
          <Button
            size="sm"
            variant="text"
            aria-label={t("settings.scenes.viewTerms", { name })}
            onClick={onTerms}>
            {t("settings.scenes.viewTermsShort")}
          </Button>
        </div>
      )}
    </Card>
  );
}

/** 查看术语: a built-in scene's term pack, and what it is for. */
function TermsDialog({
  name,
  terms,
  onClose,
}: {
  name: string;
  terms: readonly string[];
  onClose: () => void;
}) {
  const { t } = useI18n();
  return (
    <Dialog
      open
      title={t("settings.scenes.termsTitle", { name })}
      width={560}
      onClose={onClose}
      actions={
        <Button size="sm" variant="primary" onClick={onClose} data-autofocus>
          {t("common.close")}
        </Button>
      }>
      <p className="text-[12px] text-fg-muted">{t("settings.scenes.termsNote")}</p>
      <ul
        aria-label={t("settings.scenes.termsTitle", { name })}
        className="mt-3 flex max-h-[50vh] flex-wrap gap-1.5 overflow-y-auto"
        data-testid="scene-terms-list">
        {terms.map((term) => (
          <li key={term}>
            <Chip>
              {/* Domain words, not interface copy: the language checks leave them alone. */}
              <span data-user-text>{term}</span>
            </Chip>
          </li>
        ))}
      </ul>
    </Dialog>
  );
}

export interface SceneCardsProps {
  /** Whether scenes are matched by the application in front (the desktop): the cards then show
   *  the matching order, the switches and what each scene matches. The phone picks a scene by hand
   *  (user decision 2026-10-01). */
  matchApps?: boolean;
  /** Open the editor on `scene`. */
  onEdit: (scene: Scene) => void;
}

/** The scenes (`state.scenes`, in matching order) as cards, shared by the desktop's 场景 settings
 *  and the phone's (docs/dictation.md §18): each card switches its scene on or off, moves it
 *  (`scenes_reorder`), opens it in the editor, deletes it after a confirm, or shows a built-in
 *  scene's term pack. The core owns the list; every change comes back as a `scenes` event. */
export function SceneCards({ matchApps = true, onEdit }: SceneCardsProps) {
  const { backend } = useBackend();
  const shell = useFeatureShell();
  const { t, locale } = useI18n();
  const scenes = useUiState().scenes;
  // The built-in scenes' term packs (§18.10), asked for once; a refusal leaves the counts out.
  const [packs, setPacks] = useState<readonly BuiltinSceneTerms[]>([]);
  const [viewing, setViewing] = useState<Scene | undefined>(undefined);
  useEffect(() => {
    let alive = true;
    backend.scenesBuiltin().then(
      (answer) => {
        if (alive) setPacks(answer);
      },
      () => undefined,
    );
    return () => {
      alive = false;
    };
  }, [backend]);
  const termsOf = (scene: Scene) =>
    scene.builtin === undefined ? undefined : packs.find((p) => p.id === scene.builtin)?.terms;

  const run = (action: Promise<void>) => {
    action.catch((e: unknown) => {
      shell.notify(t("app.error", { message: errorText(e) }), "danger");
    });
  };
  const move = (scene: Scene, delta: -1 | 1) => {
    const ids = scenes.map((s) => s.id);
    run(backend.invoke("scenes_reorder", { ids: movedBy(ids, ids.indexOf(scene.id), delta) }));
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
    <>
      {scenes.length === 0 ? (
        <EmptyState compact title={t("settings.scenes.emptyTitle")}>
          {matchApps ? t("settings.scenes.emptyBody") : t("settings.scenes.emptyBodyPicked")}
        </EmptyState>
      ) : (
        <ol aria-label={t("settings.scenes.listLabel")} className="flex flex-col gap-2">
          {scenes.map((scene, index) => (
            <li key={scene.id}>
              <SceneCard
                scene={scene}
                index={index}
                count={scenes.length}
                matchApps={matchApps}
                terms={termsOf(scene)}
                onTerms={() => {
                  setViewing(scene);
                }}
                onToggle={(enabled) => {
                  run(
                    backend.invoke("scenes_update", {
                      id: scene.id,
                      scene: sceneWithEnabled(scene, enabled),
                    }),
                  );
                }}
                onMove={(delta) => {
                  move(scene, delta);
                }}
                onEdit={() => {
                  onEdit(scene);
                }}
                onRemove={() => {
                  remove(scene);
                }}
              />
            </li>
          ))}
        </ol>
      )}
      {viewing !== undefined && (
        <TermsDialog
          name={sceneLabel(viewing, locale)}
          terms={termsOf(viewing) ?? []}
          onClose={() => {
            setViewing(undefined);
          }}
        />
      )}
    </>
  );
}
