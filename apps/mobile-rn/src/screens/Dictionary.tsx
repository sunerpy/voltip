// 个人词典 (apps/mobile's Dictionary, docs/dictation.md §16): the core's dictionary in matching
// order, searchable; each entry switches on or off in its row and opens in the full-screen editor,
// which also moves it in the order and deletes it after a confirmation. A floating button adds one.
import {
  type DictionaryEntry,
  HEARD_AS_JOINER,
  MAX_DICTIONARY_ENTRIES,
  MAX_HEARD_AS,
  dictionaryDraftProblem,
  errorText,
  movedBy,
  splitHeardAs,
} from "@voltip/shared";
import { useMemo, useState } from "react";
import { View } from "react-native";
import {
  IconButton,
  Searchbar,
  Switch,
  Text,
  TextInput,
  TouchableRipple,
} from "react-native-paper";

import { useBackend, useUiState } from "../backend/BackendProvider";
import { useI18n } from "../backend/i18n";
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
  useAppTheme,
  FloatingAction,
} from "../ui/kit";

function EntryEditor({
  entry,
  onClose,
}: {
  entry?: DictionaryEntry | undefined;
  onClose: () => void;
}) {
  const theme = useAppTheme();
  const { backend } = useBackend();
  const shell = useShell();
  const { t } = useI18n();
  const entries = useUiState().dictionary;
  const [term, setTerm] = useState(entry?.term ?? "");
  const [heard, setHeard] = useState(entry?.heard_as.join(HEARD_AS_JOINER) ?? "");
  const [saveError, setSaveError] = useState<string | undefined>(undefined);
  const heardAs = splitHeardAs(heard);
  const problem = dictionaryDraftProblem(
    term,
    heardAs,
    entries.filter((e) => e.id !== entry?.id),
    t,
  );
  const index = entry === undefined ? -1 : entries.findIndex((e) => e.id === entry.id);
  const fail = (e: unknown) => {
    shell.toast(t("mobile.toast.error", { message: errorText(e) }), "danger");
  };
  const save = async () => {
    if (problem !== undefined) return;
    const draft = { term: term.trim(), heard_as: heardAs, enabled: entry?.enabled ?? true };
    try {
      if (entry === undefined) await backend.invoke("dictionary_add", { entry: draft });
      else await backend.invoke("dictionary_update", { id: entry.id, entry: draft });
      onClose();
    } catch (e) {
      setSaveError(errorText(e));
    }
  };
  const move = (delta: -1 | 1) => {
    backend
      .invoke("dictionary_reorder", {
        ids: movedBy(
          entries.map((e) => e.id),
          index,
          delta,
        ),
      })
      .catch(fail);
  };
  const termProblem = term.length > 0 ? problem?.term : undefined;
  return (
    <FullScreenDialog
      visible
      title={entry === undefined ? t("dictionary.newEntry") : t("mobile.dictionary.editTitle")}
      onClose={onClose}
      action={{
        label: t("common.save"),
        onPress: () => void save(),
        disabled: problem !== undefined,
      }}
      testID="dictionary-editor">
      <View style={{ gap: 4 }}>
        <TextInput
          mode="outlined"
          label={t("dictionary.column.term")}
          value={term}
          autoFocus
          error={termProblem !== undefined}
          onChangeText={(text) => {
            setTerm(text);
            setSaveError(undefined);
          }}
          testID="dictionary-term"
        />
        {termProblem !== undefined && <Hint tone="danger">{termProblem}</Hint>}
      </View>
      <View style={{ gap: 4 }}>
        <TextInput
          mode="outlined"
          label={t("dictionary.column.heard")}
          value={heard}
          error={problem?.heard !== undefined}
          onChangeText={(text) => {
            setHeard(text);
            setSaveError(undefined);
          }}
          testID="dictionary-heard"
        />
        <Hint {...(problem?.heard === undefined ? {} : { tone: "danger" as const })}>
          {problem?.heard ?? t("dictionary.column.heardHelp")}
        </Hint>
      </View>
      {entry !== undefined && entries.length > 1 && (
        <View style={{ flexDirection: "row", alignItems: "center", gap: 4 }}>
          <IconButton
            icon="arrow-up"
            mode="outlined"
            accessibilityLabel={t("dictionary.row.up", { term: entry.term })}
            disabled={index <= 0}
            onPress={() => {
              move(-1);
            }}
          />
          <IconButton
            icon="arrow-down"
            mode="outlined"
            accessibilityLabel={t("dictionary.row.down", { term: entry.term })}
            disabled={index === entries.length - 1}
            onPress={() => {
              move(1);
            }}
          />
          <Text variant="bodySmall" style={{ marginLeft: 8, color: theme.voltip.subtle }}>
            {t("mobile.dictionary.order", { n: index + 1, total: entries.length })}
          </Text>
        </View>
      )}
      {saveError !== undefined && <Hint tone="danger">{saveError}</Hint>}
      {entry !== undefined && (
        <Button
          icon="trash-can-outline"
          textColor={theme.colors.error}
          style={{ alignSelf: "flex-start" }}
          onPress={() => {
            shell.confirm({
              title: t("dictionary.row.deleteAsk", { term: entry.term }),
              body: t("mobile.dictionary.deleteBody"),
              confirmLabel: t("common.delete"),
              onConfirm: () => {
                backend.invoke("dictionary_remove", { id: entry.id }).then(onClose, fail);
              },
            });
          }}>
          {t("common.delete")}
        </Button>
      )}
    </FullScreenDialog>
  );
}

export function Dictionary() {
  const theme = useAppTheme();
  const { backend } = useBackend();
  const shell = useShell();
  const { t } = useI18n();
  const entries = useUiState().dictionary;
  const [query, setQuery] = useState("");
  const [editing, setEditing] = useState<{ entry?: DictionaryEntry } | undefined>(undefined);
  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    return q.length === 0
      ? entries
      : entries.filter(
          (e) =>
            e.term.toLowerCase().includes(q) || e.heard_as.some((h) => h.toLowerCase().includes(q)),
        );
  }, [entries, query]);
  const toggle = (entry: DictionaryEntry, enabled: boolean) => {
    backend
      .invoke("dictionary_update", {
        id: entry.id,
        entry: { term: entry.term, heard_as: entry.heard_as, enabled },
      })
      .catch((e: unknown) => {
        shell.toast(t("mobile.toast.error", { message: errorText(e) }), "danger");
      });
  };
  const full = entries.length >= MAX_DICTIONARY_ENTRIES;
  return (
    <View style={{ flex: 1 }}>
      <Page testID="phone-dictionary">
        <Lede>{t("dictionary.explain")}</Lede>
        <Searchbar
          placeholder={t("dictionary.searchPlaceholder")}
          accessibilityLabel={t("dictionary.searchLabel")}
          value={query}
          onChangeText={setQuery}
        />
        {entries.length === 0 ? (
          <Section>
            <EmptyState icon="book-open-variant" title={t("dictionary.empty.title")}>
              {t("mobile.dictionary.emptyBody")}
            </EmptyState>
          </Section>
        ) : visible.length === 0 ? (
          <Hint>{t("dictionary.noMatch")}</Hint>
        ) : (
          <Section>
            <View accessibilityLabel={t("dictionary.list")}>
              {visible.map((entry, i) => (
                <View key={entry.id}>
                  {i > 0 && <RowDivider />}
                  <View style={{ flexDirection: "row", alignItems: "center", paddingRight: 12 }}>
                    <TouchableRipple
                      style={{ flex: 1 }}
                      accessibilityRole="button"
                      accessibilityLabel={t("dictionary.row.edit", { term: entry.term })}
                      onPress={() => {
                        setEditing({ entry });
                      }}>
                      <View style={{ paddingLeft: 16, paddingVertical: 12, gap: 2 }}>
                        <Text
                          variant="bodyLarge"
                          numberOfLines={1}
                          style={{
                            color: entry.enabled ? theme.colors.onSurface : theme.voltip.subtle,
                          }}>
                          {entry.term}
                        </Text>
                        {entry.heard_as.length > 0 && (
                          <Text
                            variant="bodySmall"
                            numberOfLines={1}
                            style={{ color: theme.colors.onSurfaceVariant }}>
                            {entry.heard_as.join(HEARD_AS_JOINER)}
                          </Text>
                        )}
                      </View>
                    </TouchableRipple>
                    <Switch
                      value={entry.enabled}
                      accessibilityLabel={t("dictionary.row.enable", { term: entry.term })}
                      onValueChange={(enabled) => {
                        toggle(entry, enabled);
                      }}
                    />
                  </View>
                </View>
              ))}
            </View>
          </Section>
        )}
        <Mono style={{ paddingHorizontal: 4, paddingBottom: 72 }}>
          {t("dictionary.facts", { limit: MAX_DICTIONARY_ENTRIES, heard: MAX_HEARD_AS })}
        </Mono>
      </Page>
      <FloatingAction
        icon="plus"
        label={t("dictionary.newEntry")}
        disabled={full}
        onPress={() => {
          setEditing({});
        }}
        testID="dictionary-add"
      />
      {editing !== undefined && (
        <EntryEditor
          key={editing.entry?.id ?? "new"}
          entry={editing.entry}
          onClose={() => {
            setEditing(undefined);
          }}
        />
      )}
    </View>
  );
}
