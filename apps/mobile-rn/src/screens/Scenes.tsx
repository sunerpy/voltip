// 场景 (apps/mobile's Scenes, docs/dictation.md §18): the scenes, the built-in ones included,
// without matching — the phone cannot tell which app the text goes to, so a scene is picked by hand
// on the talk card and lists no applications. `@voltip/ui`'s SceneCards and SceneEditor as the
// phone shows them (no matching order, switches, apps or output modes), on native views.
import {
  type BuiltinSceneTerms,
  MAX_SCENES,
  MAX_SCENE_PROMPT_CHARS,
  type Scene,
  type SceneEditorDraft,
  type SceneEditorProblems,
  errorText,
  hasSceneProblems,
  sceneDraftOf,
  sceneEditorDraftFrom,
  sceneEditorProblems,
  sceneLabel,
  sceneLanguageChoices,
  sceneOverrideSummary,
  scenePresetChoices,
  scenePromptChars,
  sceneRefineChoices,
  sceneScriptChoices,
} from "@voltip/shared";
import { useCallback, useEffect, useState } from "react";
import { View } from "react-native";
import { Chip, Dialog, IconButton, Portal, Text, TextInput } from "react-native-paper";

import { useBackend, useUiState } from "../backend/BackendProvider";
import { useI18n } from "../backend/i18n";
import { useShell } from "../shell";
import { Button } from "../ui/Button";
import { FullScreenDialog } from "../ui/FullScreenDialog";
import {
  EmptyState,
  FloatingAction,
  Hint,
  Lede,
  Mono,
  Page,
  Section,
  SectionTitle,
  useAppTheme,
} from "../ui/kit";
import { SelectField } from "../ui/Select";

function SceneEditor({ scene, onClose }: { scene?: Scene | undefined; onClose: () => void }) {
  const theme = useAppTheme();
  const { backend } = useBackend();
  const shell = useShell();
  const { t, locale } = useI18n();
  const state = useUiState();
  const presets = state.presets;
  const [draft, setDraft] = useState<SceneEditorDraft>(() => sceneEditorDraftFrom(scene));
  const [attempted, setAttempted] = useState(false);
  const [saveError, setSaveError] = useState<string | undefined>(undefined);
  // A built-in scene (§18.10) keeps its name.
  const builtin = scene?.builtin !== undefined;
  const problems = sceneEditorProblems(
    draft,
    state.scenes.filter((s) => s.id !== scene?.id),
    t,
    { builtin, needApps: false },
  );
  const shown = (p: SceneEditorProblems[keyof SceneEditorProblems]) =>
    p !== undefined && (attempted || !p.missing) ? p.text : undefined;
  const update = (patch: Partial<SceneEditorDraft>) => {
    setDraft((d) => ({ ...d, ...patch }));
    setSaveError(undefined);
  };
  const save = async () => {
    setAttempted(true);
    if (hasSceneProblems(problems)) return;
    const payload = sceneDraftOf(draft);
    try {
      if (scene === undefined) await backend.invoke("scenes_add", { scene: payload });
      else await backend.invoke("scenes_update", { id: scene.id, scene: payload });
      shell.toast(t("sceneEditor.saved", { name: payload.name }));
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
      onConfirm: () => {
        backend.invoke("scenes_restore", { id: scene.id }).then(
          () => {
            shell.toast(t("sceneEditor.restored", { name }));
            onClose();
          },
          (e: unknown) => {
            setSaveError(errorText(e));
          },
        );
      },
    });
  };
  const chars = scenePromptChars(draft.prompt);
  const nameProblem = shown(problems.name);
  const promptProblem = shown(problems.prompt);
  return (
    <FullScreenDialog
      visible
      title={scene === undefined ? t("sceneEditor.titleNew") : t("sceneEditor.titleEdit")}
      onClose={onClose}
      action={{ label: t("common.save"), onPress: () => void save() }}
      testID="scene-editor">
      {builtin && scene !== undefined ? (
        <View style={{ gap: 4 }}>
          <TextInput
            mode="outlined"
            label={t("sceneEditor.name")}
            value={sceneLabel(scene, locale)}
            editable={false}
            testID="scene-builtin-name"
          />
          <Hint>{t("sceneEditor.builtinName")}</Hint>
        </View>
      ) : (
        <View style={{ gap: 4 }}>
          <TextInput
            mode="outlined"
            label={t("sceneEditor.name")}
            value={draft.name}
            placeholder={t("sceneEditor.namePlaceholder")}
            error={nameProblem !== undefined}
            autoFocus
            onChangeText={(name) => {
              update({ name });
            }}
          />
          {nameProblem !== undefined && <Hint tone="danger">{nameProblem}</Hint>}
        </View>
      )}
      <SectionTitle>{t("sceneEditor.overrides")}</SectionTitle>
      <SelectField
        label={t("sceneEditor.refine")}
        value={draft.refine}
        options={sceneRefineChoices(t)}
        onChange={(refine) => update({ refine })}
      />
      <SelectField
        label={t("sceneEditor.preset")}
        value={draft.preset}
        options={scenePresetChoices(presets, draft.preset, t)}
        onChange={(preset) => update({ preset })}
        testID="scene-preset"
      />
      <SelectField
        label={t("sceneEditor.language")}
        value={draft.language}
        options={sceneLanguageChoices(draft.language, t)}
        onChange={(language) => update({ language })}
      />
      <SelectField
        label={t("sceneEditor.script")}
        value={draft.script}
        options={sceneScriptChoices(t)}
        onChange={(script) => update({ script })}
      />
      <View style={{ gap: 4 }}>
        <TextInput
          mode="outlined"
          label={t("sceneEditor.prompt")}
          value={draft.prompt}
          multiline
          numberOfLines={4}
          error={problems.prompt !== undefined}
          onChangeText={(prompt) => {
            update({ prompt });
          }}
        />
        <View style={{ flexDirection: "row", justifyContent: "space-between", gap: 12 }}>
          <Hint {...(problems.prompt === undefined ? {} : { tone: "danger" as const })}>
            {promptProblem ?? t("sceneEditor.promptHelp")}
          </Hint>
          <Mono
            testID="scene-prompt-count"
            style={{
              color: chars > MAX_SCENE_PROMPT_CHARS ? theme.colors.error : theme.voltip.subtle,
            }}>
            {t("sceneEditor.promptCount", { n: chars, max: MAX_SCENE_PROMPT_CHARS })}
          </Mono>
        </View>
      </View>
      {builtin && (
        <Button
          icon="restore"
          style={{ alignSelf: "flex-start" }}
          onPress={restore}
          testID="scene-restore">
          {t("sceneEditor.restore")}
        </Button>
      )}
      {saveError !== undefined && <Hint tone="danger">{saveError}</Hint>}
    </FullScreenDialog>
  );
}

function SceneCard({
  scene,
  terms,
  onEdit,
  onRemove,
  onTerms,
}: {
  scene: Scene;
  terms: readonly string[] | undefined;
  onEdit: () => void;
  onRemove: () => void;
  onTerms: () => void;
}) {
  const theme = useAppTheme();
  const { t, locale } = useI18n();
  const presets = useUiState().presets;
  const builtin = scene.builtin;
  const name = sceneLabel(scene, locale);
  const summary = sceneOverrideSummary(scene.overrides, t, locale, presets);
  return (
    <Section padded testID="scene-card">
      <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
        <Text variant="titleMedium" numberOfLines={1} style={{ flexShrink: 1 }}>
          {name}
        </Text>
        {builtin !== undefined && (
          <Chip compact mode="outlined" textStyle={{ fontSize: 11 }}>
            {t("settings.scenes.builtinBadge")}
          </Chip>
        )}
        <View style={{ flex: 1 }} />
        <IconButton
          icon="pencil-outline"
          accessibilityLabel={t("settings.scenes.edit", { name })}
          onPress={onEdit}
          style={{ margin: 0 }}
        />
        {builtin === undefined && (
          <IconButton
            icon="trash-can-outline"
            iconColor={theme.colors.error}
            accessibilityLabel={t("settings.scenes.remove", { name })}
            onPress={onRemove}
            style={{ margin: 0 }}
          />
        )}
      </View>
      {builtin !== undefined && (
        <Text
          variant="bodySmall"
          style={{ color: theme.colors.onSurfaceVariant }}
          testID="scene-builtin-description">
          {t(`builtinScenes.${builtin}.description`)}
        </Text>
      )}
      <Text
        variant="bodySmall"
        style={{ color: theme.colors.onSurfaceVariant }}
        testID="scene-summary">
        {summary.length === 0 ? t("settings.scenes.followsGlobal") : summary.join(" · ")}
      </Text>
      {terms !== undefined && terms.length > 0 && (
        <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }} testID="scene-terms">
          <Text variant="bodySmall" style={{ color: theme.colors.onSurfaceVariant }}>
            {t("settings.scenes.terms", { n: terms.length })}
          </Text>
          <Button
            compact
            accessibilityLabel={t("settings.scenes.viewTerms", { name })}
            onPress={onTerms}>
            {t("settings.scenes.viewTermsShort")}
          </Button>
        </View>
      )}
    </Section>
  );
}

export function Scenes() {
  const theme = useAppTheme();
  const { backend } = useBackend();
  const shell = useShell();
  const { t, locale } = useI18n();
  const scenes = useUiState().scenes;
  const own = scenes.filter((s) => s.builtin === undefined).length;
  const [editing, setEditing] = useState<{ scene?: Scene } | undefined>(undefined);
  const [packs, setPacks] = useState<readonly BuiltinSceneTerms[]>([]);
  const [viewing, setViewing] = useState<Scene | undefined>(undefined);
  const close = useCallback(() => {
    setEditing(undefined);
  }, []);
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
  const remove = (scene: Scene) => {
    shell.confirm({
      title: t("settings.scenes.confirmRemove.title", { name: scene.name }),
      body: t("settings.scenes.confirmRemove.body"),
      confirmLabel: t("common.delete"),
      onConfirm: () => {
        backend.invoke("scenes_remove", { id: scene.id }).catch((e: unknown) => {
          shell.toast(t("app.error", { message: errorText(e) }), "danger");
        });
      },
    });
  };
  const viewingTerms = viewing === undefined ? [] : (termsOf(viewing) ?? []);
  return (
    <View style={{ flex: 1 }}>
      <Page testID="phone-scenes">
        <Lede>{t("mobile.scenes.lede")}</Lede>
        {scenes.length === 0 ? (
          <Section>
            <EmptyState icon="view-grid-outline" title={t("settings.scenes.emptyTitle")}>
              {t("settings.scenes.emptyBodyPicked")}
            </EmptyState>
          </Section>
        ) : (
          <View style={{ gap: 12 }} accessibilityLabel={t("settings.scenes.listLabel")}>
            {scenes.map((scene) => (
              <SceneCard
                key={scene.id}
                scene={scene}
                terms={termsOf(scene)}
                onTerms={() => {
                  setViewing(scene);
                }}
                onEdit={() => {
                  setEditing({ scene });
                }}
                onRemove={() => {
                  remove(scene);
                }}
              />
            ))}
          </View>
        )}
        <Mono style={{ paddingHorizontal: 4, paddingBottom: 72 }}>
          {t("mobile.scenes.footnote", { limit: MAX_SCENES })}
        </Mono>
      </Page>
      <FloatingAction
        icon="plus"
        label={t("settings.scenes.add")}
        disabled={own >= MAX_SCENES}
        onPress={() => {
          setEditing({});
        }}
        testID="scenes-add"
      />
      {editing !== undefined && (
        <SceneEditor key={editing.scene?.id ?? "new"} scene={editing.scene} onClose={close} />
      )}
      <Portal>
        <Dialog
          visible={viewing !== undefined}
          onDismiss={() => {
            setViewing(undefined);
          }}>
          <Dialog.Title>
            {viewing === undefined
              ? ""
              : t("settings.scenes.termsTitle", { name: sceneLabel(viewing, locale) })}
          </Dialog.Title>
          <Dialog.ScrollArea style={{ maxHeight: 420, paddingHorizontal: 24 }}>
            <Text
              variant="bodySmall"
              style={{ color: theme.colors.onSurfaceVariant, paddingTop: 8 }}>
              {t("settings.scenes.termsNote")}
            </Text>
            <View
              style={{ flexDirection: "row", flexWrap: "wrap", gap: 6, paddingVertical: 12 }}
              testID="scene-terms-list">
              {viewingTerms.map((term) => (
                <Chip key={term} compact>
                  {term}
                </Chip>
              ))}
            </View>
          </Dialog.ScrollArea>
          <Dialog.Actions>
            <Button
              onPress={() => {
                setViewing(undefined);
              }}>
              {t("common.close")}
            </Button>
          </Dialog.Actions>
        </Dialog>
      </Portal>
    </View>
  );
}
