import { MAX_SCENES, type Scene } from "@voltip/shared";
import { Button, SceneCards, SceneEditor, useI18n, useUiState } from "@voltip/ui";
import { useCallback, useState } from "react";

/** 场景 on the phone (docs/dictation.md §18; user decision 2026-10-01): the desktop's scenes, the
 *  built-in ones included, without matching. The phone cannot tell which app the text goes to, so
 *  a scene is picked by hand on the talk card (`settings_set_pinned_scene`) and lists no
 *  applications; the cards and the editor are the desktop's (`@voltip/ui`), without the matching
 *  order, the switches and the output modes. */
export function Scenes() {
  const { t } = useI18n();
  const scenes = useUiState().scenes;
  const own = scenes.filter((s) => s.builtin === undefined).length;
  // `{}` is a new scene, `{ scene }` an existing one; the dialog's `onClose` stays stable.
  const [editing, setEditing] = useState<{ scene?: Scene } | undefined>(undefined);
  const close = useCallback(() => {
    setEditing(undefined);
  }, []);
  return (
    <div className="flex flex-col gap-3 p-4" data-testid="phone-scenes">
      <p className="px-1 text-[12px] leading-5 text-fg-muted">{t("mobile.scenes.lede")}</p>
      <Button
        size="sm"
        variant="primary"
        icon="plus"
        className="self-start"
        disabled={own >= MAX_SCENES}
        onClick={() => {
          setEditing({});
        }}>
        {t("settings.scenes.add")}
      </Button>
      <SceneCards
        matchApps={false}
        onEdit={(scene) => {
          setEditing({ scene });
        }}
      />
      <p className="mono px-1 text-[11px] text-fg-subtle">
        {t("mobile.scenes.footnote", { limit: MAX_SCENES })}
      </p>
      {editing !== undefined && (
        <SceneEditor
          key={editing.scene?.id ?? "new"}
          scene={editing.scene}
          matchApps={false}
          outputModes={false}
          onClose={close}
        />
      )}
    </div>
  );
}
