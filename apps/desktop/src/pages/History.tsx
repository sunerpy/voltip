import {
  type EditRecord,
  type HistoryEntry,
  type TFunction,
  type VocabularyHit,
  diffSegments,
  formatCount,
  activationHint,
  formatMs,
  formatSeconds,
  outcomeLabel,
  outputModeLabel,
} from "@voltip/shared";
import {
  Badge,
  Button,
  Card,
  Dialog,
  EmptyState,
  IconButton,
  Input,
  Keycaps,
  Lamp,
  LampText,
  Panel,
  Readout,
  Segmented,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { useId, useMemo, useState } from "react";
import { inTextField, usePageShortcuts, withCommand } from "../app/page-shortcuts";
import { useRouter } from "../app/router";
import { copyWithToast, useShell } from "../app/shell-context";
import { useTickingNow } from "../features/dictation/useDictation";
import { errorText, splitHeardAs } from "../features/vocabulary/vocabulary";
import {
  HISTORY_FILTERS,
  type HistoryFilter,
  clockLabel,
  dayLabel,
  filterHistory,
  groupByDay,
  historyFilterLabel,
  isHistoryFilter,
  matchesHistoryQuery,
  textChars,
} from "../features/history/stats";
import { shortModel } from "../shell/page-meta";

type View = "raw" | "polished" | "diff";

export interface HistoryProps {
  /** `today | week | month | all | starred | failed` from the home tiles, or an entry id. */
  initialFilter?: string;
}

/** A phone's text rather than a recognised take (docs/dictation.md §20.6): no model, no timings. */
function sentAsText(entry: HistoryEntry): boolean {
  return entry.origin?.kind === "typed" || entry.origin?.kind === "clipboard";
}

/** History: the store is the core's `history.json` (`state.history`, newest first, capped
 *  at 500). Filters, search, star, delete, clear, copy and the raw-vs-refined diff are all real;
 *  the detail names the dictionary corrections and rules that fired (`HistoryEntry.vocabulary`)
 *  and 加入词典 sends `dictionary_add` with the row's id (docs/dictation.md §16). A take with a
 *  context shows its app and scene in the row and the detail (§18.6); search matches them too. A
 *  voice edit (§19.5) is badged 编辑, reads 「指令 → 结果」 in the row, and its detail shows the
 *  instruction, the rewrite and the original selection (expandable) instead of the diff views.
 *  What a phone sent (§20.6) carries its origin: its takes are badged with the phone, its texts
 *  too, without the model and timings a text does not have. */
export function History({ initialFilter }: HistoryProps) {
  const shell = useShell();
  const { backend } = useBackend();
  const { navigate } = useRouter();
  const { t, locale } = useI18n();
  const state = useUiState();
  const entries = state.history;
  // Settings › 隐私与历史: whether takes are recorded and how many are kept.
  const retention = state.settings.history;
  const now = useTickingNow(false);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<HistoryFilter>(
    isHistoryFilter(initialFilter) ? initialFilter : "all",
  );
  // A route value that is not a range name is an entry id (the home table's row click).
  const [selectedId, setSelectedId] = useState<string | undefined>(
    isHistoryFilter(initialFilter) ? undefined : initialFilter,
  );
  const [view, setView] = useState<View>("polished");
  const [adding, setAdding] = useState<{ entryId: string; heard: string } | undefined>(undefined);

  const visible = useMemo(
    () => filterHistory(entries, filter, now).filter((e) => matchesHistoryQuery(e, query)),
    [entries, filter, query, now],
  );
  const groups = useMemo(() => groupByDay(visible, now, locale), [visible, now, locale]);
  // Nothing picked yet, or the pick was deleted: the newest row is the detail, so the pane is never
  // stale and a fresh dictation shows up on the right as soon as the core appends it.
  const selected = entries.find((e) => e.id === selectedId) ?? entries[0];

  const star = (entry: HistoryEntry) => {
    void backend.invoke("history_star", { id: entry.id, starred: !entry.starred });
  };
  const remove = (entry: HistoryEntry) => {
    shell.confirm({
      title: t("history.confirm.deleteTitle"),
      body: t("history.confirm.deleteBody", {
        when: `${dayLabel(entry.at_ms, now, locale)} ${clockLabel(entry.at_ms)}`,
        excerpt: `${entry.text.slice(0, 40)}${entry.text.length > 40 ? "…" : ""}`,
      }),
      confirmLabel: t("common.delete"),
      tone: "danger",
      onConfirm: () => {
        const idx = visible.findIndex((e) => e.id === entry.id);
        const next = visible[idx + 1] ?? visible[idx - 1];
        setSelectedId(next?.id);
        void backend.invoke("history_delete", { id: entry.id });
      },
    });
  };
  const clearAll = () => {
    shell.confirm({
      title: t("history.confirm.clearTitle", { n: entries.length }),
      body: t("history.confirm.clearBody"),
      confirmLabel: t("history.confirm.clear"),
      tone: "danger",
      onConfirm: () => {
        setSelectedId(undefined);
        void backend.invoke("history_clear");
      },
    });
  };
  const starLabel = (entry: HistoryEntry) =>
    entry.starred ? t("history.unstar") : t("history.star");
  const copy = (entry: HistoryEntry) => {
    // A voice edit copies its rewrite (the detail shows no raw / diff views).
    const text = view === "raw" && entry.kind !== "edit" ? entry.raw_text : entry.text;
    void copyWithToast(shell, text, t("history.detail.copied", { n: textChars(text) }));
  };

  // The footer's keys: Ctrl F searches, Ctrl C copies the entry in the detail (unless text is
  // selected or a field has the focus: then the copy is the text's), Del deletes it (confirmed).
  const searchId = useId();
  usePageShortcuts((e) => {
    if (withCommand(e) && e.key.toLowerCase() === "f") {
      const search = document.getElementById(searchId);
      search?.focus();
      if (search instanceof HTMLInputElement) search.select();
      return true;
    }
    if (selected === undefined || inTextField(e.target)) return false;
    if (withCommand(e) && e.key.toLowerCase() === "c") {
      if ((window.getSelection()?.toString() ?? "").length > 0) return false;
      copy(selected);
      return true;
    }
    if (e.key === "Delete" && !withCommand(e) && !e.altKey && !e.shiftKey) {
      remove(selected);
      return true;
    }
    return false;
  });

  return (
    <div
      className="mx-auto flex w-full max-w-[1440px] flex-col gap-4 p-6"
      data-testid="page-history">
      <Card
        padding="none"
        className="flex min-h-16 flex-wrap items-center gap-x-6 gap-y-2 px-4 py-3">
        <Lamp tone={retention.enabled && entries.length > 0 ? "ok" : "idle"} />
        <div className="min-w-0 flex-1">
          <div className="text-[14px] font-medium text-fg">{t("history.banner.title")}</div>
          <div
            className="text-[12px] text-fg-muted"
            data-testid="history-retention"
            data-enabled={retention.enabled}>
            {retention.enabled
              ? t("history.banner.retention", { keep: retention.keep })
              : t("history.banner.off", { n: entries.length })}
          </div>
        </div>
        <Readout
          label={t("history.banner.saved")}
          value={`${entries.length} / ${retention.keep}`}
          size="sm"
        />
        <Button
          size="sm"
          variant="text"
          onClick={() => {
            navigate({ name: "settings", section: "privacy" });
          }}>
          {t("history.banner.settings")}
        </Button>
        <Button
          size="sm"
          variant="text"
          className="text-danger"
          onClick={clearAll}
          disabled={entries.length === 0}>
          {t("history.banner.clearAll")}
        </Button>
      </Card>

      {/* session log left, entry detail right. The log column follows the window between
          280 and 360 px (designed at 320) and the detail takes the rest; below `lg` they stack. */}
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-[minmax(280px,360px)_minmax(0,1fr)]">
        <Panel
          eyebrow={t("history.eyebrow.log")}
          title={String(visible.length)}
          className="min-h-[552px]"
          bodyClassName="flex flex-col gap-3">
          <Input
            id={searchId}
            icon="search"
            keys="Ctrl F"
            placeholder={t("history.search.placeholder")}
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
            }}
            size="sm"
            aria-label={t("history.search.label")}
          />
          <Segmented
            label={t("history.filter.label")}
            variant="ink"
            size="sm"
            value={filter}
            onChange={setFilter}
            options={HISTORY_FILTERS.map((f) => ({ value: f, label: historyFilterLabel(f, t) }))}
          />
          {entries.length === 0 && (
            <EmptyState compact title={t("history.empty.none")}>
              {t("history.empty.noneBody", {
                hint: activationHint(state.settings.activation, state.settings.hotkey, locale),
              })}
            </EmptyState>
          )}
          {entries.length > 0 && visible.length === 0 && (
            <EmptyState
              compact
              title={
                query
                  ? t("history.empty.noMatch", { query })
                  : t("history.empty.noneInFilter", { filter: historyFilterLabel(filter, t) })
              }
              actions={
                query ? (
                  <Button
                    size="sm"
                    variant="text"
                    onClick={() => {
                      setQuery("");
                    }}>
                    {t("history.empty.clearSearch")}
                  </Button>
                ) : undefined
              }>
              {query
                ? t("history.empty.noMatchBody", { n: entries.length })
                : filter === "starred"
                  ? t("history.empty.starHint")
                  : t("history.empty.rangeHint")}
            </EmptyState>
          )}
          <ul className="flex flex-col" aria-label={t("history.log")}>
            {groups.map((g) => (
              <li key={g.day}>
                <div className="flex items-center justify-between py-1 text-[12px] text-fg-muted">
                  <span>{g.day}</span>
                  <span className="mono">{g.items.length}</span>
                </div>
                <ul>
                  {g.items.map((e) => {
                    const active = e.id === selected?.id;
                    const outcome = outcomeLabel(e.outcome, locale);
                    return (
                      <li key={e.id}>
                        <button
                          type="button"
                          aria-pressed={active}
                          onClick={() => {
                            setSelectedId(e.id);
                          }}
                          className={`group flex w-full items-start gap-2 rounded-6 px-2 py-2 text-left hover:bg-canvas ${active ? "bg-canvas shadow-[inset_2px_0_0_var(--primary)]" : ""}`}>
                          <span className="mono pt-0.5 text-[12px] text-fg-subtle">
                            {clockLabel(e.at_ms).slice(0, 5)}
                          </span>
                          <span className="min-w-0 flex-1">
                            {e.kind === "edit" && e.edit !== undefined ? (
                              // docs/dictation.md §19.5: instruction → result.
                              <span
                                className="block truncate text-[14px] text-fg"
                                data-testid="history-edit-row">
                                <span className="text-fg-muted" data-user-text>
                                  {e.edit.instruction}
                                </span>
                                <span className="px-1 text-fg-subtle" aria-hidden>
                                  →
                                </span>
                                <span data-user-text>{e.text}</span>
                              </span>
                            ) : (
                              <span className="block truncate text-[14px] text-fg" data-user-text>
                                {e.text}
                              </span>
                            )}
                            <span className="mono flex items-center gap-1.5 truncate text-[11px] text-fg-subtle">
                              {e.kind === "edit" && (
                                <span data-testid="history-kind" data-kind={e.kind}>
                                  <Badge tone="accent">{t("history.edit.badge")}</Badge>
                                </span>
                              )}
                              {e.origin !== undefined && (
                                // docs/dictation.md §20.6: a phone's take or text.
                                <span data-testid="history-origin" data-origin={e.origin.kind}>
                                  <Badge>
                                    <span data-user-text>
                                      {t(`history.origin.${e.origin.kind}`, {
                                        device: e.origin.device,
                                      })}
                                    </span>
                                  </Badge>
                                </span>
                              )}
                              {e.mode !== "whole_take" && (
                                // docs/dictation.md §12: only the streaming modes get a badge; the
                                // whole take is the default and stays quiet.
                                <span data-testid="history-mode" data-mode={e.mode}>
                                  <Badge tone="accent">{outputModeLabel(e.mode, locale)}</Badge>
                                </span>
                              )}
                              {e.app !== undefined && (
                                // docs/dictation.md §18.6: the app the take was dictated into and
                                // the scene that ran it.
                                <span
                                  className="flex min-w-0 shrink items-center gap-1"
                                  data-testid="history-context"
                                  title={t("history.context.label")}>
                                  <span className="truncate text-fg-muted" data-user-text>
                                    {e.app.name}
                                  </span>
                                  {e.scene !== undefined && (
                                    <Badge>
                                      <span data-user-text>{e.scene.name}</span>
                                    </Badge>
                                  )}
                                </span>
                              )}
                              <span className="truncate">
                                {sentAsText(e)
                                  ? outcome.text
                                  : `${shortModel(e.asr_model)} · ${formatMs(e.asr_ms + (e.refine_ms ?? 0))} · ${outcome.text}`}
                              </span>
                            </span>
                          </span>
                          <span
                            role="button"
                            tabIndex={0}
                            aria-label={starLabel(e)}
                            onClick={(ev) => {
                              ev.stopPropagation();
                              star(e);
                            }}
                            onKeyDown={(ev) => {
                              if (ev.key === "Enter") {
                                ev.stopPropagation();
                                star(e);
                              }
                            }}
                            className={
                              e.starred
                                ? "text-fg"
                                : "text-fg-subtle opacity-0 group-hover:opacity-100 hover:text-fg"
                            }>
                            ★
                          </span>
                        </button>
                      </li>
                    );
                  })}
                </ul>
              </li>
            ))}
          </ul>
        </Panel>

        <Panel
          eyebrow={t("history.eyebrow.entry")}
          title={
            selected
              ? `${dayLabel(selected.at_ms, now, locale)} · ${clockLabel(selected.at_ms)}`
              : undefined
          }
          className="min-h-[552px]"
          right={
            selected && (
              <span className="flex items-center gap-2">
                {selected.kind === "edit" && (
                  <span data-testid="history-detail-kind" data-kind={selected.kind}>
                    <Badge tone="accent">{t("history.edit.badge")}</Badge>
                  </span>
                )}
                {selected.origin !== undefined && (
                  <span data-testid="history-detail-origin" data-origin={selected.origin.kind}>
                    <Badge>
                      <span data-user-text>
                        {t(`history.origin.${selected.origin.kind}`, {
                          device: selected.origin.device,
                        })}
                      </span>
                    </Badge>
                  </span>
                )}
                {selected.mode !== "whole_take" && (
                  <span data-testid="history-detail-mode" data-mode={selected.mode}>
                    <Badge tone="accent">{outputModeLabel(selected.mode, locale)}</Badge>
                  </span>
                )}
                <LampText tone={outcomeLabel(selected.outcome, locale).tone} mono>
                  {outcomeLabel(selected.outcome, locale).text}
                </LampText>
              </span>
            )
          }>
          {!selected ? (
            <EmptyState title={t("history.empty.pick")} mono={t("history.empty.pickHint")} />
          ) : (
            <div className="flex h-full flex-col gap-4">
              {selected.kind === "edit" && selected.edit !== undefined ? (
                <EditDetail entry={selected} edit={selected.edit} t={t} />
              ) : (
                <>
                  <div className="flex items-center justify-between">
                    <Segmented
                      label={t("history.view.label")}
                      size="sm"
                      value={view}
                      onChange={setView}
                      options={[
                        { value: "raw", label: t("history.view.raw") },
                        {
                          value: "polished",
                          label: selected.refined
                            ? t("history.view.polished")
                            : t("history.view.inserted"),
                        },
                        { value: "diff", label: t("history.view.diff") },
                      ]}
                    />
                    {view === "diff" && (
                      <span className="flex gap-2">
                        <Badge tone="danger">{t("history.view.deleted")}</Badge>
                        <Badge tone="accent">{t("history.view.added")}</Badge>
                      </span>
                    )}
                  </div>
                  <div
                    className="min-h-[120px] rounded-10 bg-inset p-4 text-[15px] leading-7 text-fg"
                    data-testid="entry-text"
                    data-user-text>
                    {view === "raw" && selected.raw_text}
                    {view === "polished" && selected.text}
                    {view === "diff" &&
                      diffSegments({ rawText: selected.raw_text, text: selected.text }).map(
                        (seg, i) =>
                          seg.kind === "same" ? (
                            <span key={i}>{seg.text}</span>
                          ) : seg.kind === "del" ? (
                            <span
                              key={i}
                              className="mx-0.5 rounded-4 bg-diff-del px-1 text-danger line-through">
                              {seg.text}
                            </span>
                          ) : (
                            <span
                              key={i}
                              className="mx-0.5 rounded-4 bg-diff-add px-1 text-ok-text">
                              {seg.text}
                            </span>
                          ),
                      )}
                    {view !== "raw" && (
                      <div className="mt-3 text-[12px] text-fg-subtle">
                        {t("history.view.rawOutput", { text: selected.raw_text })}
                      </div>
                    )}
                  </div>
                </>
              )}
              {selected.live_error !== undefined && (
                // §12: a streaming mode was asked for but the take (or part of it) fell back to the
                // whole-take path; the core's reason, verbatim.
                <p
                  className="rounded-6 bg-warning-soft px-3 py-2 text-[12px] leading-4 text-warning"
                  data-testid="history-live-error">
                  {t("history.liveError", { reason: selected.live_error })}
                </p>
              )}
              {selected.vocabulary !== undefined && (
                <VocabularyHits hits={selected.vocabulary} t={t} />
              )}
              {!sentAsText(selected) && <TimingBar entry={selected} t={t} />}
              <div className="grid grid-cols-2 gap-x-6 gap-y-2 border-t border-border pt-3">
                <Readout
                  label={t("history.detail.asrModel")}
                  value={selected.asr_model}
                  size="sm"
                />
                <Readout
                  label={t("history.detail.refineModel")}
                  value={
                    selected.refined
                      ? (selected.refine_model ?? "—")
                      : t("history.detail.notRefined")
                  }
                  size="sm"
                  muted={!selected.refined}
                />
                <Readout
                  label={t("history.detail.outcome")}
                  value={outcomeLabel(selected.outcome, locale).text}
                  size="sm"
                />
                <Readout
                  label={t("history.detail.duration")}
                  value={formatSeconds(selected.duration_ms)}
                  size="sm"
                />
                <Readout
                  label={t("history.detail.chars")}
                  value={t("count.chars", { n: textChars(selected.text) })}
                  size="sm"
                />
                <Readout label={t("history.detail.id")} value={selected.id} size="sm" />
                {selected.app !== undefined && (
                  <>
                    <Readout
                      label={t("history.context.app")}
                      value={
                        <span data-user-text data-testid="history-detail-app">
                          {selected.app.name} · {selected.app.id}
                        </span>
                      }
                      size="sm"
                    />
                    <Readout
                      label={t("history.context.scene")}
                      value={
                        selected.scene === undefined ? (
                          t("history.context.noScene")
                        ) : (
                          <span data-user-text data-testid="history-detail-scene">
                            {selected.scene.name}
                          </span>
                        )
                      }
                      muted={selected.scene === undefined}
                      size="sm"
                    />
                  </>
                )}
              </div>
              <div className="mt-auto flex flex-wrap items-center gap-2 border-t border-border pt-3">
                <Button
                  size="sm"
                  onClick={() => {
                    copy(selected);
                  }}>
                  {t("common.copy")}
                </Button>
                <Keycaps keys="Ctrl C" />
                <IconButton
                  icon="star"
                  label={starLabel(selected)}
                  onClick={() => {
                    star(selected);
                  }}
                  className={selected.starred ? "text-fg" : ""}
                />
                <Button
                  size="sm"
                  icon="book"
                  onMouseDown={(e) => {
                    // Keep the text selection: it pre-fills the misheard form.
                    e.preventDefault();
                  }}
                  onClick={() => {
                    setAdding({ entryId: selected.id, heard: selectedFragment(selected) });
                  }}>
                  {t("history.addToDictionary.button")}
                </Button>
                <Button
                  size="sm"
                  variant="text"
                  className="ml-auto text-danger"
                  onClick={() => {
                    remove(selected);
                  }}>
                  {t("common.delete")}
                </Button>
                <Keycaps keys="Del" />
              </div>
            </div>
          )}
        </Panel>
      </div>
      {adding !== undefined && (
        <AddToDictionary
          historyId={adding.entryId}
          heard={adding.heard}
          onClose={() => {
            setAdding(undefined);
          }}
        />
      )}
    </div>
  );
}

/** The text the user selected inside this entry's raw or final text (what 加入词典 pre-fills). */
/** A voice edit's detail (docs/dictation.md §19.5): the instruction as sent to the LLM, the
 *  rewrite that was pasted, and the original selection behind a disclosure. */
function EditDetail({ entry, edit, t }: { entry: HistoryEntry; edit: EditRecord; t: TFunction }) {
  return (
    <div className="flex flex-col gap-3" data-testid="history-edit">
      <div>
        <div className="eyebrow mb-1">{t("history.edit.instruction")}</div>
        <div
          className="rounded-10 bg-inset px-4 py-2 text-[14px] leading-6 text-fg"
          data-testid="history-edit-instruction"
          data-user-text>
          {edit.instruction}
        </div>
      </div>
      <div>
        <div className="eyebrow mb-1">{t("history.edit.result")}</div>
        <div
          className="min-h-[96px] rounded-10 bg-inset p-4 text-[15px] leading-7 whitespace-pre-wrap text-fg"
          data-testid="entry-text"
          data-user-text>
          {entry.text}
        </div>
      </div>
      <details className="hairline rounded-10 px-3 py-2" data-testid="history-edit-selection">
        <summary className="cursor-pointer text-[12px] text-fg-muted">
          {t("history.edit.selection", { n: textChars(edit.selection) })}
        </summary>
        <div
          className="mt-2 text-[14px] leading-6 whitespace-pre-wrap text-fg-muted"
          data-user-text>
          {edit.selection}
        </div>
      </details>
    </div>
  );
}

function selectedFragment(entry: HistoryEntry): string {
  const text = window.getSelection()?.toString().trim() ?? "";
  if (text.length === 0 || text.includes("\n")) return "";
  return entry.raw_text.includes(text) || entry.text.includes(text) ? text : "";
}

/** Which dictionary entries and rules fired in this take, named from the current lists. */
function VocabularyHits({
  hits,
  t,
}: {
  hits: NonNullable<HistoryEntry["vocabulary"]>;
  t: TFunction;
}) {
  const state = useUiState();
  const named = (
    list: readonly VocabularyHit[],
    name: (id: string) => string | undefined,
    gone: string,
  ) =>
    list
      .map((hit) => t("history.vocabulary.hit", { name: name(hit.id) ?? gone, n: hit.count }))
      .join("、");
  const rows = [
    {
      label: t("history.vocabulary.corrections"),
      list: hits.corrections,
      text: named(
        hits.corrections,
        (id) => state.dictionary.find((e) => e.id === id)?.term,
        t("history.vocabulary.deletedEntry"),
      ),
    },
    {
      label: t("history.vocabulary.rules"),
      list: hits.rules,
      text: named(
        hits.rules,
        (id) => state.rules.find((r) => r.id === id)?.name,
        t("history.vocabulary.deletedRule"),
      ),
    },
  ].filter((row) => row.list.length > 0);
  return (
    <div data-testid="history-vocabulary">
      <div className="eyebrow">{t("history.vocabulary.title")}</div>
      <dl className="mt-1 grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 text-[12px]">
        {rows.map((row) => (
          <div key={row.label} className="contents">
            <dt className="text-fg-subtle">{row.label}</dt>
            <dd className="text-fg" data-user-text>
              {row.text}
            </dd>
          </div>
        ))}
      </dl>
    </div>
  );
}

/** 加入词典: the right spelling and the misheard forms, sent as `dictionary_add { historyId }`. A
 *  draft the core refuses keeps the dialog open with the core's reason. */
function AddToDictionary({
  historyId,
  heard: initialHeard,
  onClose,
}: {
  historyId: string;
  heard: string;
  onClose: () => void;
}) {
  const { backend } = useBackend();
  const shell = useShell();
  const { t } = useI18n();
  const [term, setTerm] = useState("");
  const [heard, setHeard] = useState(initialHeard);
  const [error, setError] = useState<string | undefined>(undefined);
  const submit = async () => {
    const entry = { term: term.trim(), heard_as: splitHeardAs(heard), enabled: true };
    try {
      await backend.invoke("dictionary_add", { entry, historyId });
      shell.toast({
        message: t("history.addToDictionary.added", { term: entry.term }),
        duration: 3000,
      });
      onClose();
    } catch (e) {
      setError(errorText(e));
    }
  };
  return (
    <Dialog
      open
      title={t("history.addToDictionary.title")}
      width={480}
      onClose={onClose}
      actions={
        <>
          <Button size="sm" variant="ghost" onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button
            size="sm"
            variant="primary"
            disabled={term.trim().length === 0}
            onClick={() => {
              void submit();
            }}>
            {t("history.addToDictionary.add")}
          </Button>
        </>
      }>
      <div className="flex flex-col gap-3">
        <p>{t("history.addToDictionary.body")}</p>
        <Input
          label={t("history.addToDictionary.heard")}
          mono
          size="sm"
          value={heard}
          help={t("history.addToDictionary.heardHelp")}
          onChange={(e) => {
            setHeard(e.target.value);
            setError(undefined);
          }}
        />
        <Input
          label={t("history.addToDictionary.term")}
          mono
          size="sm"
          value={term}
          data-autofocus
          error={error}
          onChange={(e) => {
            setTerm(e.target.value);
            setError(undefined);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" && term.trim().length > 0) {
              e.preventDefault();
              void submit();
            }
          }}
        />
      </div>
    </Dialog>
  );
}

function TimingBar({ entry, t }: { entry: HistoryEntry; t: TFunction }) {
  const asr = entry.asr_ms;
  const refine = entry.refine_ms ?? 0;
  const total = asr + refine;
  if (total === 0)
    return <div className="mono text-[11px] text-fg-subtle">{t("history.timing.none")}</div>;
  const pct = (n: number) => `${(n / total) * 100}%`;
  return (
    <div>
      <div className="eyebrow">{t("history.timing.title")}</div>
      <div
        className="mt-2 flex h-3 w-full overflow-hidden rounded-pill bg-led-off"
        role="img"
        aria-label={t("history.timing.label")}>
        <span className="bg-primary" style={{ width: pct(asr) }} />
        {refine > 0 && <span className="bg-accent" style={{ width: pct(refine) }} />}
      </div>
      <div className="mono mt-1.5 flex items-center gap-4 text-[11px] text-fg-muted">
        <span className="flex items-center gap-1.5">
          <Lamp tone="neutral" size={6} />
          {t("history.timing.asr", { n: formatCount(asr) })}
        </span>
        <span className="flex items-center gap-1.5">
          <Lamp tone="accent" size={6} />
          {entry.refined
            ? t("history.timing.refine", { n: formatCount(refine) })
            : t("history.timing.refineOff")}
        </span>
        <span className="ml-auto text-fg">
          {t("history.timing.total", { n: formatCount(total) })}
        </span>
      </div>
    </div>
  );
}
