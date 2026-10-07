// 替换规则 (apps/mobile's Rules, docs/dictation.md §16): the core's rules in execution order; each
// switches on or off in its row and opens in the full-screen editor, which checks the draft with the
// core (`vocabulary_preview`, the Rust regex dialect) and tries it on a text. The rules come in as
// pasted TOML (`rules_import`) and go out copied or through the share sheet (`rules_export`).
import {
  type ImportMode,
  MAX_RULES,
  type PreviewDraft,
  type ReplacementRule,
  type RuleDraft,
  errorText,
  movedBy,
  ruleDraftProblem,
} from "@voltip/shared";
import { useMemo, useState } from "react";
import { View } from "react-native";
import {
  IconButton,
  SegmentedButtons,
  Switch,
  Text,
  TextInput,
  TouchableRipple,
} from "react-native-paper";

import { useBackend, useUiState } from "../backend/BackendProvider";
import { useI18n } from "../backend/i18n";
import { useVocabularyPreview } from "../hooks/useVocabularyPreview";
import { useShell } from "../shell";
import { Button } from "../ui/Button";
import { FullScreenDialog } from "../ui/FullScreenDialog";
import {
  EmptyState,
  Hint,
  Lede,
  Mono,
  Page,
  RowDivider,
  Section,
  SwitchRow,
  styles as kit,
  useAppTheme,
  FloatingAction,
} from "../ui/kit";

const NEW_RULE: RuleDraft = {
  name: "",
  kind: "literal",
  pattern: "",
  replacement: "",
  case_sensitive: true,
  enabled: true,
};

function draftOf(rule: ReplacementRule): RuleDraft {
  return {
    name: rule.name,
    kind: rule.kind,
    pattern: rule.pattern,
    replacement: rule.replacement,
    case_sensitive: rule.case_sensitive,
    enabled: rule.enabled,
  };
}

function RuleEditor({
  rule,
  onClose,
}: {
  rule?: ReplacementRule | undefined;
  onClose: () => void;
}) {
  const theme = useAppTheme();
  const { backend } = useBackend();
  const shell = useShell();
  const { t } = useI18n();
  const rules = useUiState().rules;
  const [draft, setDraft] = useState<RuleDraft>(() => (rule ? draftOf(rule) : NEW_RULE));
  const [sample, setSample] = useState("");
  const [saveError, setSaveError] = useState<string | undefined>(undefined);
  const problem = ruleDraftProblem(
    draft.name,
    draft.pattern,
    rules.filter((r) => r.id !== rule?.id),
    t,
  );
  const previewDraft = useMemo<PreviewDraft | undefined>(
    () => (problem === undefined ? { id: rule?.id ?? null, rule: draft } : undefined),
    [draft, problem, rule?.id],
  );
  const check = useVocabularyPreview(sample, previewDraft, previewDraft !== undefined);
  const index = rule === undefined ? -1 : rules.findIndex((r) => r.id === rule.id);
  const update = (patch: Partial<RuleDraft>) => {
    setDraft((d) => ({ ...d, ...patch }));
    setSaveError(undefined);
  };
  const fail = (e: unknown) => {
    shell.toast(t("mobile.toast.error", { message: errorText(e) }), "danger");
  };
  const save = async () => {
    if (problem !== undefined) return;
    try {
      if (rule === undefined) await backend.invoke("rules_add", { rule: draft });
      else await backend.invoke("rules_update", { id: rule.id, rule: draft });
      onClose();
    } catch (e) {
      setSaveError(errorText(e));
    }
  };
  const move = (delta: -1 | 1) => {
    backend
      .invoke("rules_reorder", {
        ids: movedBy(
          rules.map((r) => r.id),
          index,
          delta,
        ),
      })
      .catch(fail);
  };
  const status =
    saveError ??
    (draft.pattern.length > 0 ? problem?.pattern : undefined) ??
    (check.kind === "error" ? check.message : undefined);
  const nameProblem = draft.name.length > 0 ? problem?.name : undefined;
  return (
    <FullScreenDialog
      visible
      title={rule === undefined ? t("rules.empty.newRule") : t("mobile.rules.editTitle")}
      onClose={onClose}
      action={{
        label: t("common.save"),
        onPress: () => void save(),
        disabled: problem !== undefined || check.kind === "error",
      }}
      testID="rule-editor">
      <View style={{ gap: 4 }}>
        <TextInput
          mode="outlined"
          label={t("rules.editor.name")}
          value={draft.name}
          autoFocus
          error={nameProblem !== undefined}
          onChangeText={(name) => {
            update({ name });
          }}
        />
        {nameProblem !== undefined && <Hint tone="danger">{nameProblem}</Hint>}
      </View>
      <View style={{ gap: 8 }}>
        <Text variant="bodySmall" style={{ color: theme.colors.onSurfaceVariant }}>
          {t("rules.editor.kind")}
        </Text>
        <SegmentedButtons
          value={draft.kind}
          onValueChange={(kind) => {
            update({ kind: kind === "regex" ? "regex" : "literal" });
          }}
          buttons={[
            { value: "literal", label: t("rules.kind.literal") },
            { value: "regex", label: t("rules.kind.regex") },
          ]}
        />
      </View>
      <TextInput
        mode="outlined"
        label={t("rules.editor.from")}
        value={draft.pattern}
        autoCapitalize="none"
        autoCorrect={false}
        style={kit.mono}
        onChangeText={(pattern) => {
          update({ pattern });
        }}
      />
      <View style={{ gap: 4 }}>
        <TextInput
          mode="outlined"
          label={t("rules.editor.to")}
          value={draft.replacement}
          autoCapitalize="none"
          autoCorrect={false}
          style={kit.mono}
          onChangeText={(replacement) => {
            update({ replacement });
          }}
        />
        <Hint>{t("rules.editor.toHelp")}</Hint>
      </View>
      <Section>
        <SwitchRow
          title={t("rules.editor.caseSensitive")}
          value={draft.case_sensitive}
          onValueChange={(case_sensitive) => {
            update({ case_sensitive });
          }}
        />
      </Section>
      <Mono
        style={{
          color: status === undefined ? theme.colors.onSurfaceVariant : theme.colors.error,
        }}>
        {status ??
          (problem !== undefined
            ? ""
            : check.kind === "ok"
              ? t("rules.editor.ok")
              : t("rules.editor.checking"))}
      </Mono>
      <TextInput
        mode="outlined"
        label={t("mobile.rules.test")}
        value={sample}
        multiline
        numberOfLines={2}
        placeholder={t("mobile.rules.testPlaceholder")}
        onChangeText={setSample}
      />
      {sample.length > 0 && check.kind === "ok" && (
        <View
          style={{
            padding: 12,
            borderRadius: 12,
            backgroundColor: theme.colors.surfaceVariant,
            gap: 4,
          }}
          testID="rule-test-result">
          <Text variant="labelSmall" style={{ color: theme.colors.onSurfaceVariant }}>
            {t("rules.dryRun.after")}
          </Text>
          <Mono variant="bodyMedium" selectable style={{ color: theme.colors.onSurface }}>
            {check.preview.output}
          </Mono>
        </View>
      )}
      {rule !== undefined && rules.length > 1 && (
        <View style={{ flexDirection: "row", alignItems: "center", gap: 4 }}>
          <IconButton
            icon="arrow-up"
            mode="outlined"
            accessibilityLabel={t("rules.row.up", { name: rule.name })}
            disabled={index <= 0}
            onPress={() => move(-1)}
          />
          <IconButton
            icon="arrow-down"
            mode="outlined"
            accessibilityLabel={t("rules.row.down", { name: rule.name })}
            disabled={index === rules.length - 1}
            onPress={() => move(1)}
          />
          <Text variant="bodySmall" style={{ marginLeft: 8, color: theme.voltip.subtle }}>
            {t("mobile.dictionary.order", { n: index + 1, total: rules.length })}
          </Text>
        </View>
      )}
      {rule !== undefined && (
        <Button
          icon="trash-can-outline"
          textColor={theme.colors.error}
          style={{ alignSelf: "flex-start" }}
          onPress={() => {
            shell.confirm({
              title: t("rules.confirm.deleteTitle", { name: rule.name }),
              body: t("rules.confirm.deleteBody"),
              confirmLabel: t("common.delete"),
              onConfirm: () => {
                backend.invoke("rules_remove", { id: rule.id }).then(onClose, fail);
              },
            });
          }}>
          {t("common.delete")}
        </Button>
      )}
    </FullScreenDialog>
  );
}

function ImportDialog({ onClose }: { onClose: () => void }) {
  const theme = useAppTheme();
  const { backend } = useBackend();
  const shell = useShell();
  const { t } = useI18n();
  const [text, setText] = useState("");
  const [mode, setMode] = useState<ImportMode>("merge");
  const [error, setError] = useState<string | undefined>(undefined);
  const modeLabel =
    mode === "merge" ? t("rules.importDialog.merge") : t("rules.importDialog.replace");
  const submit = async () => {
    try {
      await backend.invoke("rules_import", { toml: text, mode });
      shell.toast(t("rules.importDialog.done", { mode: modeLabel }));
      onClose();
    } catch (e) {
      setError(errorText(e));
    }
  };
  return (
    <FullScreenDialog
      visible
      title={t("rules.importDialog.title")}
      onClose={onClose}
      action={{
        label: t("rules.importDialog.import"),
        onPress: () => void submit(),
        disabled: text.trim().length === 0,
      }}>
      <Text variant="bodyMedium">{t("rules.importDialog.body")}</Text>
      <TextInput
        mode="outlined"
        label={t("rules.importDialog.label")}
        value={text}
        multiline
        numberOfLines={12}
        autoCapitalize="none"
        autoCorrect={false}
        style={kit.mono}
        onChangeText={(next) => {
          setText(next);
          setError(undefined);
        }}
      />
      <SegmentedButtons
        value={mode}
        onValueChange={(next) => {
          setMode(next === "replace" ? "replace" : "merge");
        }}
        buttons={[
          { value: "merge", label: t("rules.importDialog.merge") },
          { value: "replace", label: t("rules.importDialog.replace") },
        ]}
      />
      <Hint>
        {mode === "merge" ? t("rules.importDialog.mergeHelp") : t("rules.importDialog.replaceHelp")}
      </Hint>
      {error !== undefined && (
        <Mono selectable style={{ color: theme.colors.error }}>
          {error}
        </Mono>
      )}
    </FullScreenDialog>
  );
}

function ExportDialog({ text, onClose }: { text: string; onClose: () => void }) {
  const { backend } = useBackend();
  const shell = useShell();
  const { t } = useI18n();
  const fail = (e: unknown) => {
    shell.toast(t("mobile.toast.error", { message: errorText(e) }), "danger");
  };
  const copy = () => {
    void backend.pasteText(text).then(
      (outcome) => {
        if (outcome.kind === "failed") shell.toast(t("mobile.recent.copyFailed"), "danger");
        else shell.toast(t("rules.exportDialog.copied"));
      },
      () => {
        shell.toast(t("mobile.recent.copyFailed"), "danger");
      },
    );
  };
  return (
    <FullScreenDialog visible title={t("rules.exportDialog.title")} onClose={onClose}>
      <Text variant="bodyMedium">{t("rules.exportDialog.body")}</Text>
      <View style={{ flexDirection: "row", gap: 8 }}>
        <Button mode="contained" icon="content-copy" onPress={copy}>
          {t("rules.exportDialog.copy")}
        </Button>
        <Button
          mode="outlined"
          icon="share-variant-outline"
          onPress={() => void backend.invoke("phone_share_text", { text }).catch(fail)}>
          {t("mobile.rules.share")}
        </Button>
      </View>
      <Section padded>
        <Mono selectable testID="rules-export-text">
          {text}
        </Mono>
      </Section>
    </FullScreenDialog>
  );
}

export function Rules() {
  const theme = useAppTheme();
  const { backend } = useBackend();
  const shell = useShell();
  const { t } = useI18n();
  const rules = useUiState().rules;
  const [editing, setEditing] = useState<{ rule?: ReplacementRule } | undefined>(undefined);
  const [importing, setImporting] = useState(false);
  const [exported, setExported] = useState<string | undefined>(undefined);
  const fail = (e: unknown) => {
    shell.toast(t("mobile.toast.error", { message: errorText(e) }), "danger");
  };
  const toggle = (rule: ReplacementRule, enabled: boolean) => {
    backend
      .invoke("rules_update", { id: rule.id, rule: { ...draftOf(rule), enabled } })
      .catch(fail);
  };
  const openExport = () => {
    backend.rulesExport().then(setExported, (e: unknown) => {
      shell.toast(t("rules.exportDialog.failed", { message: errorText(e) }), "danger");
    });
  };
  return (
    <View style={{ flex: 1 }}>
      <Page testID="phone-rules">
        <Lede>{t("mobile.rules.intro")}</Lede>
        <View style={{ flexDirection: "row", gap: 8 }}>
          <Button mode="outlined" icon="tray-arrow-down" onPress={() => setImporting(true)}>
            {t("rules.importToml")}
          </Button>
          <Button mode="outlined" icon="tray-arrow-up" onPress={openExport}>
            {t("rules.exportToml")}
          </Button>
        </View>
        {rules.length === 0 ? (
          <Section>
            <EmptyState icon="find-replace" title={t("rules.empty.title")}>
              {t("mobile.rules.emptyBody")}
            </EmptyState>
          </Section>
        ) : (
          <Section>
            <View accessibilityLabel={t("mobile.title.rules")}>
              {rules.map((rule, i) => (
                <View key={rule.id}>
                  {i > 0 && <RowDivider />}
                  <View style={{ flexDirection: "row", alignItems: "center", paddingRight: 12 }}>
                    <TouchableRipple
                      style={{ flex: 1 }}
                      accessibilityRole="button"
                      accessibilityLabel={t("rules.row.edit", { name: rule.name })}
                      onPress={() => {
                        setEditing({ rule });
                      }}>
                      <View style={{ paddingLeft: 16, paddingVertical: 12, gap: 2 }}>
                        <Text
                          variant="bodyLarge"
                          numberOfLines={1}
                          style={{
                            color: rule.enabled ? theme.colors.onSurface : theme.voltip.subtle,
                          }}>
                          {rule.name}
                          <Text
                            variant="labelSmall"
                            style={{
                              color:
                                rule.kind === "regex"
                                  ? theme.colors.primary
                                  : theme.colors.onSurfaceVariant,
                            }}>
                            {`  ${t(`rules.kind.${rule.kind}`)}`}
                          </Text>
                        </Text>
                        <Mono numberOfLines={1}>
                          {rule.pattern} →{" "}
                          {rule.replacement.length === 0
                            ? t("rules.column.emptyTo")
                            : rule.replacement}
                        </Mono>
                      </View>
                    </TouchableRipple>
                    <Switch
                      value={rule.enabled}
                      accessibilityLabel={t("rules.row.enable", { name: rule.name })}
                      onValueChange={(enabled) => toggle(rule, enabled)}
                    />
                  </View>
                </View>
              ))}
            </View>
          </Section>
        )}
        <Mono style={{ paddingHorizontal: 4, paddingBottom: 72 }}>
          {t("rules.footer", { n: rules.length, limit: MAX_RULES })}
        </Mono>
      </Page>
      <FloatingAction
        icon="plus"
        label={t("rules.empty.newRule")}
        disabled={rules.length >= MAX_RULES}
        onPress={() => {
          setEditing({});
        }}
        testID="rules-add"
      />
      {editing !== undefined && (
        <RuleEditor
          key={editing.rule?.id ?? "new"}
          rule={editing.rule}
          onClose={() => {
            setEditing(undefined);
          }}
        />
      )}
      {importing && <ImportDialog onClose={() => setImporting(false)} />}
      {exported !== undefined && (
        <ExportDialog text={exported} onClose={() => setExported(undefined)} />
      )}
    </View>
  );
}
