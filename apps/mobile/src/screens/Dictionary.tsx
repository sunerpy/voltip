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
import {
  Button,
  Dialog,
  EmptyState,
  IconButton,
  Input,
  Toggle,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { useMemo, useState } from "react";
import { useMobileShell } from "../app/shell";

/** Creates or edits one entry: the correct spelling and the forms it is heard as. The instant
 *  checks are the desktop's (`dictionaryDraftProblem`); the core validates again on save and a
 *  refusal is shown here, the dialog staying open. An existing entry also moves in the matching
 *  order or is deleted after a confirm. */
function EntryDialog({ entry, onClose }: { entry?: DictionaryEntry; onClose: () => void }) {
  const { backend } = useBackend();
  const shell = useMobileShell();
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
    const ids = entries.map((e) => e.id);
    backend.invoke("dictionary_reorder", { ids: movedBy(ids, index, delta) }).catch(fail);
  };
  const remove = () => {
    if (entry === undefined) return;
    shell.confirm({
      title: t("dictionary.row.deleteAsk", { term: entry.term }),
      body: t("mobile.dictionary.deleteBody"),
      confirmLabel: t("common.delete"),
      onConfirm: () => {
        backend.invoke("dictionary_remove", { id: entry.id }).then(onClose, fail);
      },
    });
  };

  return (
    <Dialog
      open
      title={entry === undefined ? t("dictionary.newEntry") : t("mobile.dictionary.editTitle")}
      width={420}
      onClose={onClose}
      actions={
        <>
          {entry !== undefined && (
            <Button size="sm" variant="text-danger" className="mr-auto" onClick={remove}>
              {t("common.delete")}
            </Button>
          )}
          <Button size="sm" variant="ghost" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button
            size="sm"
            variant="primary"
            disabled={problem !== undefined}
            onClick={() => {
              void save();
            }}>
            {t("common.save")}
          </Button>
        </>
      }>
      <div className="flex flex-col gap-3" data-testid="dictionary-editor">
        <Input
          label={t("dictionary.column.term")}
          size="sm"
          value={term}
          data-autofocus
          error={term.length > 0 ? problem?.term : undefined}
          onChange={(e) => {
            setTerm(e.target.value);
            setSaveError(undefined);
          }}
        />
        <Input
          label={t("dictionary.column.heard")}
          size="sm"
          value={heard}
          help={t("dictionary.column.heardHelp")}
          error={problem?.heard}
          onChange={(e) => {
            setHeard(e.target.value);
            setSaveError(undefined);
          }}
        />
        {entry !== undefined && entries.length > 1 && (
          <div className="flex items-center gap-1">
            <IconButton
              icon="chevronUp"
              label={t("dictionary.row.up", { term: entry.term })}
              disabled={index <= 0}
              onClick={() => {
                move(-1);
              }}
            />
            <IconButton
              icon="chevronDown"
              label={t("dictionary.row.down", { term: entry.term })}
              disabled={index === entries.length - 1}
              onClick={() => {
                move(1);
              }}
            />
            <span className="text-[12px] text-fg-subtle">
              {t("mobile.dictionary.order", { n: index + 1, total: entries.length })}
            </span>
          </div>
        )}
        {saveError !== undefined && (
          <p className="text-[12px] text-danger" role="alert">
            {saveError}
          </p>
        )}
      </div>
    </Dialog>
  );
}

/** 个人词典 on the phone (docs/dictation.md §16; user decision 2026-10-01: the phone has the
 *  desktop's dictionary for what it recognises itself): the core's `state.dictionary` in matching
 *  order, searchable; each entry switches on or off here and opens in the editor. */
export function Dictionary() {
  const { backend } = useBackend();
  const shell = useMobileShell();
  const { t } = useI18n();
  const entries = useUiState().dictionary;
  const [query, setQuery] = useState("");
  // `{}` is a new entry, `{ entry }` an existing one.
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

  return (
    <div className="flex flex-col gap-3 p-4" data-testid="phone-dictionary">
      <p className="px-1 text-[12px] leading-5 text-fg-muted">{t("dictionary.explain")}</p>
      <div className="flex items-end gap-2">
        <Input
          size="sm"
          className="flex-1"
          aria-label={t("dictionary.searchLabel")}
          placeholder={t("dictionary.searchPlaceholder")}
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
          }}
        />
        <Button
          size="sm"
          variant="primary"
          icon="plus"
          disabled={entries.length >= MAX_DICTIONARY_ENTRIES}
          onClick={() => {
            setEditing({});
          }}>
          {t("dictionary.newEntry")}
        </Button>
      </div>
      {entries.length === 0 ? (
        <EmptyState compact title={t("dictionary.empty.title")}>
          {t("mobile.dictionary.emptyBody")}
        </EmptyState>
      ) : visible.length === 0 ? (
        <p className="px-1 text-[12px] text-fg-subtle">{t("dictionary.noMatch")}</p>
      ) : (
        <ul
          aria-label={t("dictionary.list")}
          className="flex flex-col divide-y divide-border overflow-hidden rounded-10 bg-surface hairline">
          {visible.map((entry) => (
            <li key={entry.id} className="flex items-center gap-3 px-4 py-3">
              <button
                type="button"
                className="flex min-w-0 flex-1 flex-col text-left"
                aria-label={t("dictionary.row.edit", { term: entry.term })}
                onClick={() => {
                  setEditing({ entry });
                }}>
                <span
                  className={`truncate text-[14px] font-medium ${entry.enabled ? "text-fg" : "text-fg-subtle"}`}
                  data-user-text>
                  {entry.term}
                </span>
                {entry.heard_as.length > 0 && (
                  <span className="truncate text-[12px] text-fg-muted" data-user-text>
                    {entry.heard_as.join(HEARD_AS_JOINER)}
                  </span>
                )}
              </button>
              <Toggle
                checked={entry.enabled}
                ariaLabel={t("dictionary.row.enable", { term: entry.term })}
                onChange={(enabled) => {
                  toggle(entry, enabled);
                }}
              />
            </li>
          ))}
        </ul>
      )}
      <p className="mono px-1 text-[11px] text-fg-subtle">
        {t("dictionary.facts", { limit: MAX_DICTIONARY_ENTRIES, heard: MAX_HEARD_AS })}
      </p>
      {editing !== undefined && (
        <EntryDialog
          key={editing.entry?.id ?? "new"}
          entry={editing.entry}
          onClose={() => {
            setEditing(undefined);
          }}
        />
      )}
    </div>
  );
}
