import {
  type DictionaryEntry,
  MAX_DICTIONARY_ENTRIES,
  MAX_HEARD_AS,
  type VocabularyHit,
} from "@voltip/shared";
import {
  Badge,
  Button,
  Card,
  Chip,
  EmptyState,
  IconButton,
  Input,
  Lamp,
  LampText,
  Panel,
  Segmented,
  Table,
  type TableColumn,
  Textarea,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { type KeyboardEvent, useCallback, useMemo, useState } from "react";
import { usePageShortcuts, withCommand } from "../app/page-shortcuts";
import { copyWithToast, useShell } from "../app/shell-context";
import { useHitTotals } from "../features/vocabulary/useHitTotals";
import { useVocabularyPreview } from "../features/vocabulary/usePreview";
import {
  HEARD_AS_JOINER,
  dictionaryDraftProblem,
  errorText,
  movedBy,
  splitHeardAs,
} from "../features/vocabulary/vocabulary";

type Filter = "all" | "enabled" | "disabled";

/** The row being edited; `id` is absent for a new entry (shown above the table). */
interface Editing {
  id: string | undefined;
  term: string;
  heard: string;
}

const ASCII_ONLY = /^[\x20-\x7e]+$/;

/** The personal dictionary (docs/dictation.md §16): the core's `state.dictionary`, in matching
 *  order. Add / edit / enable / reorder / delete go through the `dictionary_*` commands; the hit
 *  counts come from the history rows (`HistoryEntry.vocabulary`); the test panel asks the core
 *  (`vocabulary_preview`), so it corrects exactly as a take would. */
export function Dictionary() {
  const shell = useShell();
  const { backend } = useBackend();
  const { t } = useI18n();
  const state = useUiState();
  const entries = state.dictionary;
  const [filter, setFilter] = useState<Filter>("all");
  const [query, setQuery] = useState("");
  const [highlight, setHighlight] = useState<string | undefined>(undefined);
  const [editing, setEditing] = useState<Editing | undefined>(undefined);
  const [saveError, setSaveError] = useState<string | undefined>(undefined);
  const [deleting, setDeleting] = useState<string | undefined>(undefined);
  const [sample, setSample] = useState("");
  const preview = useVocabularyPreview(sample);

  const hits = useHitTotals("corrections");
  const enabled = entries.filter((e) => e.enabled).length;
  const top = useMemo(() => {
    const fired = entries.filter((e) => (hits.get(e.id) ?? 0) > 0);
    fired.sort((a, b) => (hits.get(b.id) ?? 0) - (hits.get(a.id) ?? 0));
    return fired.slice(0, 5);
  }, [entries, hits]);
  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    return entries.filter((e) => {
      if (filter === "enabled" && !e.enabled) return false;
      if (filter === "disabled" && e.enabled) return false;
      return (
        q.length === 0 ||
        e.term.toLowerCase().includes(q) ||
        e.heard_as.some((h) => h.toLowerCase().includes(q))
      );
    });
  }, [entries, filter, query]);

  const startNew = useCallback(() => {
    setEditing({ id: undefined, term: "", heard: "" });
    setSaveError(undefined);
    setDeleting(undefined);
  }, []);
  // Ctrl N: a new entry (not while a modal is over the page).
  usePageShortcuts((e) => {
    if (!withCommand(e) || e.key.toLowerCase() !== "n") return false;
    startNew();
    return true;
  });

  const heardAs = editing ? splitHeardAs(editing.heard) : [];
  const problem = editing
    ? dictionaryDraftProblem(
        editing.term,
        heardAs,
        entries.filter((e) => e.id !== editing.id),
        t,
      )
    : undefined;

  const save = async () => {
    if (!editing || problem) return;
    const current = entries.find((e) => e.id === editing.id);
    const entry = {
      term: editing.term.trim(),
      heard_as: heardAs,
      enabled: current?.enabled ?? true,
    };
    try {
      if (editing.id === undefined) await backend.invoke("dictionary_add", { entry });
      else await backend.invoke("dictionary_update", { id: editing.id, entry });
      setEditing(undefined);
      setSaveError(undefined);
    } catch (e) {
      setSaveError(errorText(e));
    }
  };
  const editorKeys = (e: KeyboardEvent) => {
    // The row's own Enter / Space selection must not see the keys typed into its editor.
    e.stopPropagation();
    if (e.key === "Enter") {
      e.preventDefault();
      void save();
    } else if (e.key === "Escape") {
      e.preventDefault();
      setEditing(undefined);
    }
  };
  /** Escape anywhere in the editor (its buttons included) cancels it. */
  const cancelOnEscape = (e: KeyboardEvent) => {
    if (e.key !== "Escape") return;
    e.stopPropagation();
    setEditing(undefined);
  };
  const run = (action: Promise<void>) => {
    action.catch((e: unknown) => {
      shell.toast({
        message: t("app.error", { message: errorText(e) }),
        duration: 5000,
        tone: "danger",
      });
    });
  };
  const toggle = (entry: DictionaryEntry, next: boolean) => {
    run(
      backend.invoke("dictionary_update", {
        id: entry.id,
        entry: { term: entry.term, heard_as: entry.heard_as, enabled: next },
      }),
    );
  };
  const move = (entry: DictionaryEntry, delta: -1 | 1) => {
    const ids = entries.map((e) => e.id);
    run(backend.invoke("dictionary_reorder", { ids: movedBy(ids, ids.indexOf(entry.id), delta) }));
  };

  const termInput = (value: Editing) => (
    <Input
      aria-label={t("dictionary.column.term")}
      mono
      size="sm"
      value={value.term}
      error={problem?.term ?? saveError}
      autoFocus
      onKeyDown={editorKeys}
      onChange={(e) => {
        setEditing({ ...value, term: e.target.value });
        setSaveError(undefined);
      }}
    />
  );
  const heardInput = (value: Editing) => (
    <Input
      aria-label={t("dictionary.column.heard")}
      mono
      size="sm"
      value={value.heard}
      error={problem?.heard}
      help={t("dictionary.column.heardHelp")}
      onKeyDown={editorKeys}
      onChange={(e) => {
        setEditing({ ...value, heard: e.target.value });
        setSaveError(undefined);
      }}
    />
  );
  const editorButtons = (
    <span className="inline-flex gap-1" onKeyDown={cancelOnEscape}>
      <Button
        size="sm"
        variant="primary"
        onClick={() => {
          void save();
        }}
        disabled={problem !== undefined}>
        {t("common.save")}
      </Button>
      <Button
        size="sm"
        variant="ghost"
        onClick={() => {
          setEditing(undefined);
        }}>
        {t("common.cancel")}
      </Button>
    </span>
  );

  const columns: TableColumn<DictionaryEntry>[] = [
    {
      id: "lamp",
      header: "",
      width: 24,
      align: "center",
      cell: (e) => (
        <Lamp
          tone={e.enabled ? "ok" : "idle"}
          label={e.enabled ? t("dictionary.filter.enabled") : t("dictionary.filter.disabled")}
        />
      ),
    },
    {
      id: "term",
      header: t("dictionary.column.term"),
      mono: false,
      cell: (e) =>
        editing?.id === e.id ? (
          termInput(editing)
        ) : (
          <span
            data-user-text
            className={`${ASCII_ONLY.test(e.term) ? "mono" : ""} font-medium ${e.enabled ? "text-fg" : "text-fg-subtle"}`}>
            {e.term}
          </span>
        ),
    },
    {
      id: "heard",
      header: t("dictionary.column.heard"),
      mono: false,
      cell: (e) =>
        editing?.id === e.id ? (
          heardInput(editing)
        ) : (
          <span
            data-user-text
            className={e.enabled ? "text-fg-muted" : "text-fg-subtle"}
            title={e.heard_as.join(HEARD_AS_JOINER)}>
            {e.heard_as.length > 0 ? e.heard_as.join(HEARD_AS_JOINER) : "—"}
          </span>
        ),
    },
    {
      id: "source",
      header: t("dictionary.column.source"),
      width: 88,
      mono: false,
      cell: (e) => ({
        type: "badge",
        text: t(`dictionary.source.${e.source.kind}`),
        tone: "neutral",
      }),
    },
    {
      id: "hits",
      header: t("dictionary.column.hits"),
      width: 56,
      align: "right",
      cell: (e) => {
        const n = hits.get(e.id) ?? 0;
        return { type: "mono", text: n > 0 ? String(n) : "—", muted: n === 0 };
      },
    },
    {
      id: "enabled",
      header: t("dictionary.column.enabled"),
      width: 56,
      align: "center",
      mono: false,
      cell: (e) => ({
        type: "toggle",
        checked: e.enabled,
        label: t("dictionary.row.enable", { term: e.term }),
        onChange: (next) => {
          toggle(e, next);
        },
      }),
    },
    {
      id: "actions",
      header: "",
      width: 168,
      align: "right",
      mono: false,
      cell: (e) =>
        editing?.id === e.id ? (
          editorButtons
        ) : deleting === e.id ? (
          <span className="inline-flex items-center gap-1 text-[12px]">
            <span className="text-fg-muted">{t("dictionary.row.deleteAsk", { term: e.term })}</span>
            <Button
              size="sm"
              variant="text-danger"

              onClick={() => {
                setDeleting(undefined);
                run(backend.invoke("dictionary_remove", { id: e.id }));
              }}>
              {t("common.delete")}
            </Button>
            <Button
              size="sm"
              variant="text"
              onClick={() => {
                setDeleting(undefined);
              }}>
              {t("common.cancel")}
            </Button>
          </span>
        ) : (
          <span className="inline-flex gap-0.5">
            <IconButton
              icon="chevronUp"
              label={t("dictionary.row.up", { term: e.term })}
              disabled={entries[0]?.id === e.id}
              onClick={() => {
                move(e, -1);
              }}
            />
            <IconButton
              icon="chevronDown"
              label={t("dictionary.row.down", { term: e.term })}
              disabled={entries.at(-1)?.id === e.id}
              onClick={() => {
                move(e, 1);
              }}
            />
            <IconButton
              icon="edit"
              label={t("dictionary.row.edit", { term: e.term })}
              onClick={() => {
                setEditing({ id: e.id, term: e.term, heard: e.heard_as.join(HEARD_AS_JOINER) });
                setSaveError(undefined);
              }}
            />
            <IconButton
              icon="trash"
              label={t("dictionary.row.delete", { term: e.term })}
              tone="danger"
              onClick={() => {
                setDeleting(e.id);
              }}
            />
          </span>
        ),
    },
  ];

  const lastRaw = state.history_recent[0]?.raw_text;
  const termOf = (hit: VocabularyHit) => entries.find((e) => e.id === hit.id)?.term ?? hit.id;

  return (
    <div
      className="mx-auto flex w-full max-w-[1440px] flex-col gap-4 p-6"
      data-testid="page-dictionary">
      {/* list left, test panel right. The test column follows the window between 280 and
          360 px (designed at 320); below `lg` the panel drops under the list. */}
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(280px,360px)]">
        <div className="flex min-w-0 flex-col gap-3">
          <div className="flex flex-wrap items-center gap-3">
            <h2 className="text-[18px] font-semibold text-fg">{t("dictionary.title")}</h2>
            <Badge mono>{t("dictionary.countBadge", { n: entries.length })}</Badge>
            <Badge tone="ok">{t("dictionary.enabledBadge", { n: enabled })}</Badge>
            <div className="ml-auto flex gap-2">
              <Button
                size="sm"
                variant="primary"
                icon="plus"
                keys="Ctrl N"
                disabled={entries.length >= MAX_DICTIONARY_ENTRIES}
                onClick={startNew}>
                {t("dictionary.newEntry")}
              </Button>
            </div>
          </div>

          <Card padding="sm" className="flex items-start gap-3" data-testid="dictionary-explain">
            <Lamp tone={enabled > 0 ? "ok" : "idle"} className="mt-1" />
            <div className="min-w-0 flex-1">
              <div className="text-[13px] font-medium text-fg">{t("dictionary.explainTitle")}</div>
              <div className="text-[12px] leading-5 text-fg-muted">{t("dictionary.explain")}</div>
              <div className="mono mt-1 text-[11px] text-fg-subtle">
                {t("dictionary.facts", { limit: MAX_DICTIONARY_ENTRIES, heard: MAX_HEARD_AS })}
              </div>
            </div>
          </Card>

          {top.length > 0 && (
            <div className="flex flex-wrap items-center gap-2" data-testid="dictionary-hits">
              <span className="eyebrow mr-1">{t("dictionary.hitsTitle")}</span>
              {top.map((e) => (
                <Chip
                  key={e.id}
                  count={`×${hits.get(e.id) ?? 0}`}
                  active={highlight === e.id}
                  onClick={() => {
                    setHighlight(e.id);
                    setQuery("");
                    setFilter("all");
                  }}>
                  <span data-user-text>{e.term}</span>
                </Chip>
              ))}
            </div>
          )}

          <Panel
            eyebrow={t("dictionary.list")}
            right={
              <div className="flex flex-wrap items-center justify-end gap-2">
                <Input
                  icon="search"
                  placeholder={t("dictionary.searchPlaceholder")}
                  size="sm"
                  value={query}
                  onChange={(e) => {
                    setQuery(e.target.value);
                  }}
                  className="w-56"
                  aria-label={t("dictionary.searchLabel")}
                />
                <Segmented
                  label={t("dictionary.filterLabel")}
                  variant="ink"
                  size="sm"
                  value={filter}
                  onChange={setFilter}
                  options={[
                    { value: "all", label: t("dictionary.filter.all") },
                    { value: "enabled", label: t("dictionary.filter.enabled") },
                    { value: "disabled", label: t("dictionary.filter.disabled") },
                  ]}
                />
              </div>
            }
            padding="sm">
            {editing?.id === undefined && editing && (
              <div
                className="mb-2 flex items-start gap-2 rounded-6 bg-inset p-2"
                data-testid="add-row"
                onKeyDown={cancelOnEscape}>
                <div className="flex-1">{termInput(editing)}</div>
                <div className="flex-1">{heardInput(editing)}</div>
                {editorButtons}
              </div>
            )}
            <Table
              label={t("dictionary.table")}
              columns={columns}
              rows={visible}
              rowKey={(e) => e.id}
              selectedKey={highlight}
              onSelect={(e) => {
                setHighlight(e.id);
              }}
              empty={
                entries.length === 0 ? (
                  <EmptyState
                    compact
                    title={t("dictionary.empty.title")}
                    actions={
                      <Button size="sm" variant="primary" onClick={startNew}>
                        {t("dictionary.newEntry")}
                      </Button>
                    }>
                    {t("dictionary.empty.body")}
                  </EmptyState>
                ) : (
                  <EmptyState compact title={t("dictionary.noMatch")} />
                )
              }
            />
            <div className="mt-2 text-[11px] text-fg-subtle">{t("dictionary.legend")}</div>
          </Panel>
        </div>

        <Panel
          eyebrow={t("dictionary.test.title")}
          radius={14}
          className="min-w-0"
          bodyClassName="flex flex-col gap-3">
          <Textarea
            aria-label={t("dictionary.test.inputLabel")}
            rows={6}
            value={sample}
            placeholder={t("dictionary.test.placeholder")}
            onChange={(e) => {
              setSample(e.target.value);
            }}
          />
          <div className="flex flex-wrap gap-2">
            <Button
              size="sm"
              disabled={lastRaw === undefined}
              onClick={() => {
                setSample(lastRaw ?? "");
              }}>
              {t("dictionary.test.useLast")}
            </Button>
            <Button
              size="sm"
              onClick={() => {
                void navigator.clipboard
                  ?.readText()
                  .then((clip) => {
                    setSample(clip);
                  })
                  .catch(() => {
                    shell.toast({
                      message: t("dictionary.test.clipboardFailed"),
                      duration: 2000,
                      tone: "danger",
                    });
                  });
              }}>
              {t("dictionary.test.useClipboard")}
            </Button>
            <Button
              size="sm"
              variant="text"
              onClick={() => {
                setSample("");
              }}>
              {t("dictionary.test.clear")}
            </Button>
          </div>
          <TestResult
            preview={preview}
            empty={sample.length === 0}
            termOf={termOf}
            onCopy={(text) => {
              void copyWithToast(shell, text, t("dictionary.test.copiedCorrected"));
            }}
          />
        </Panel>
      </div>
    </div>
  );
}

function TestResult({
  preview,
  empty,
  termOf,
  onCopy,
}: {
  preview: ReturnType<typeof useVocabularyPreview>;
  empty: boolean;
  termOf: (hit: VocabularyHit) => string;
  onCopy: (text: string) => void;
}) {
  const { t } = useI18n();
  if (empty || preview.kind === "idle")
    return <div className="mono text-[12px] text-fg-muted">{t("dictionary.test.prompt")}</div>;
  if (preview.kind === "error")
    return (
      <div className="text-[12px] text-danger" role="status" data-testid="dictionary-test-error">
        {t("dictionary.test.failed", { message: preview.message })}
      </div>
    );
  const { corrected, output, corrections, error } = preview.preview;
  const total = corrections.reduce((sum, hit) => sum + hit.count, 0);
  return (
    <>
      <div className="mono text-[12px] text-fg-muted" data-testid="dictionary-test-summary">
        {total === 0 ? t("dictionary.test.noHit") : t("dictionary.test.hits", { n: total })}
      </div>
      {corrections.length > 0 && (
        <ul
          className="flex flex-col divide-y divide-border"
          aria-label={t("dictionary.test.hitList")}>
          {corrections.map((hit) => (
            <li key={hit.id} className="flex items-center justify-between py-2 text-[12px]">
              <span className="text-fg" data-user-text>
                {termOf(hit)}
              </span>
              <LampText tone="ok" size="sm">
                ×{hit.count}
              </LampText>
            </li>
          ))}
        </ul>
      )}
      <div className="rounded-10 bg-inset p-3">
        <div className="eyebrow mb-1 flex items-center justify-between">
          {t("dictionary.test.after")}
          <IconButton
            icon="copy"
            label={t("dictionary.test.copyCorrected")}
            onClick={() => {
              onCopy(corrected);
            }}
          />
        </div>
        <div className="text-[13px] leading-5 text-fg" data-testid="corrected" data-user-text>
          {corrected}
        </div>
        {output !== corrected && (
          <div className="mt-2 text-[12px] text-fg-muted" data-testid="corrected-rules">
            <span className="eyebrow block">{t("dictionary.test.afterRules")}</span>
            <span data-user-text>{output}</span>
          </div>
        )}
      </div>
      {error !== undefined && (
        <p className="border-l-2 border-warning pl-2 text-[11px] text-fg-muted">
          {t("dictionary.test.fallback", { reason: error })}
        </p>
      )}
    </>
  );
}
