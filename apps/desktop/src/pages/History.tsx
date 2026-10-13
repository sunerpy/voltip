import {
  type HistoryEntry,
  formatCount,
  activationHint,
  formatMs,
  outcomeLabel,
  outputModeLabel,
  sceneLabel,
  syncsToPhones,
} from "@voltip/shared";
import {
  Badge,
  Button,
  Card,
  EmptyState,
  Input,
  Lamp,
  LampText,
  Panel,
  Readout,
  Segmented,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { useEffect, useId, useMemo, useState } from "react";
import { inTextField, usePageShortcuts, withCommand } from "../app/page-shortcuts";
import { useRouter } from "../app/router";
import { copyWithToast, useShell } from "../app/shell-context";
import { useTickingNow } from "../features/dictation/useDictation";
import {
  HISTORY_FILTERS,
  type HistoryFilter,
  clockLabel,
  dayLabel,
  groupByDay,
  historyFilterLabel,
  isHistoryFilter,
  textChars,
} from "../features/history/stats";
import {
  SEARCH_DEBOUNCE_MS,
  useDebounced,
  useHistoryEntry,
  useHistoryList,
} from "../features/history/useHistoryList";
import { ResultActions } from "../features/history/ResultActions";
import { shortModel } from "../shell/page-meta";
import { HistoryDetail, type View } from "./history/HistoryDetail";

export interface HistoryProps {
  /** `today | week | month | all | starred | failed` from the home tiles, or an entry id. */
  initialFilter?: string;
}

/** `onSeen` whenever the returned element scrolls into view while `active` (where the webview has
 *  an `IntersectionObserver`; the element's button covers the rest). */
function useLoadMoreWhenSeen(active: boolean, onSeen: () => void) {
  const [element, setElement] = useState<HTMLDivElement | null>(null);
  useEffect(() => {
    if (!active || element === null || typeof IntersectionObserver === "undefined") return;
    const observer = new IntersectionObserver((seen) => {
      if (seen.some((e) => e.isIntersecting)) onSeen();
    });
    observer.observe(element);
    return () => {
      observer.disconnect();
    };
  }, [active, element, onSeen]);
  return setElement;
}

/** A phone's text rather than a recognised take (docs/dictation.md §20.6): no model, no timings. */
function sentAsText(entry: HistoryEntry): boolean {
  return entry.origin?.kind === "typed" || entry.origin?.kind === "clipboard";
}

/** History: the core's database (docs/dictation.md §4.4, up to 20 000 entries), read a page of
 *  100 at a time through `history_query` — the filters and the search run in the core, more
 *  entries load as the list scrolls, the detail comes from the list or `history_entry`. Star,
 *  delete, clear, copy and the raw-vs-refined diff are all real;
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
  const total = state.history_total;
  // Settings › 隐私与历史: whether takes are recorded and how many are kept.
  const retention = state.settings.history;
  // A phone with Sync on gets a copy (docs/dictation.md §20.8).
  const synced = syncsToPhones(state.devices);
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

  const search = useDebounced(query, SEARCH_DEBOUNCE_MS);
  const list = useHistoryList(filter, search, now);
  const visible = list.entries;
  const groups = useMemo(() => groupByDay(visible, now, locale), [visible, now, locale]);
  // Nothing picked yet, or the pick was deleted: the newest row is the detail, so the pane is never
  // stale and a fresh dictation shows up on the right as soon as the core appends it.
  const selected = useHistoryEntry(selectedId, visible) ?? visible[0];
  const moreRef = useLoadMoreWhenSeen(list.more, list.loadMore);

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
      title: t("history.confirm.clearTitle", { n: total }),
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

  // The page's keys: Ctrl F searches, Ctrl C copies the entry in the detail (unless text is
  // selected or a field has the focus: then the copy is the text's), Del deletes it (confirmed).
  const searchId = useId();
  usePageShortcuts((e) => {
    if (withCommand(e) && e.key.toLowerCase() === "f") {
      const box = document.getElementById(searchId);
      box?.focus();
      if (box instanceof HTMLInputElement) box.select();
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
        <Lamp tone={retention.enabled && total > 0 ? "ok" : "idle"} />
        <div className="min-w-0 flex-1">
          <div className="text-[14px] font-medium text-fg" data-testid="history-banner-title">
            {synced ? t("history.banner.titleSynced") : t("history.banner.title")}
          </div>
          <div
            className="text-[12px] text-fg-muted"
            data-testid="history-retention"
            data-enabled={retention.enabled}>
            {retention.enabled
              ? t("history.banner.retention", { keep: formatCount(retention.keep) })
              : t("history.banner.off", { n: formatCount(total) })}
          </div>
        </div>
        <Readout
          label={t("history.banner.saved")}
          value={`${formatCount(total)} / ${formatCount(retention.keep)}`}
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
          variant="text-danger"

          onClick={clearAll}
          disabled={total === 0}>
          {t("history.banner.clearAll")}
        </Button>
      </Card>

      {/* session log left, entry detail right, two parts to three: at 1440 px the log is wide
          enough for a row's model, time and result on one line (user feedback 2026-09-29, plan
          1.6), and never narrower than 280 px; below `lg` they stack. */}
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-[minmax(280px,2fr)_minmax(0,3fr)]">
        <Panel
          eyebrow={t("history.eyebrow.log")}
          title={formatCount(list.matching)}
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
          {total === 0 && (
            <EmptyState compact title={t("history.empty.none")}>
              {t("history.empty.noneBody", {
                hint: activationHint(state.settings.activation, state.settings.hotkey, locale),
              })}
            </EmptyState>
          )}
          {/* Only for an answer to the current filter and search, and in the words of that search
              (the box may already hold newer text): no flash before the first answer. */}
          {total > 0 && list.settled && list.matching === 0 && (
            <EmptyState
              compact
              title={
                search
                  ? t("history.empty.noMatch", { query: search })
                  : t("history.empty.noneInFilter", { filter: historyFilterLabel(filter, t) })
              }
              actions={
                search ? (
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
              {search
                ? t("history.empty.noMatchBody", { n: formatCount(total) })
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
                    const meta = sentAsText(e)
                      ? outcome.text
                      : `${shortModel(e.asr_model)} · ${formatMs(e.asr_ms + (e.refine_ms ?? 0))} · ${outcome.text}`;
                    return (
                      <li
                        key={e.id}
                        className={`group flex items-start rounded-6 hover:bg-canvas ${active ? "bg-canvas shadow-[inset_2px_0_0_var(--primary)]" : ""}`}>
                        <button
                          type="button"
                          aria-pressed={active}
                          onClick={() => {
                            setSelectedId(e.id);
                          }}
                          className="flex min-w-0 flex-1 items-start gap-2 rounded-6 px-2 py-2 text-left">
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
                                      <span
                                        {...(e.scene.builtin === undefined
                                          ? { "data-user-text": "" }
                                          : {})}>
                                        {sceneLabel(e.scene, locale)}
                                      </span>
                                    </Badge>
                                  )}
                                </span>
                              )}
                              <span
                                className="truncate"
                                title={meta}
                                data-testid="history-row-meta">
                                {meta}
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
                        {/* Copy / paste into the previous window (plan 1.4): shown on the chosen row,
                            and on the others while hovered or focused. */}
                        <ResultActions
                          entry={e}
                          className={`pt-1.5 pr-1 ${active ? "" : "opacity-0 group-focus-within:opacity-100 group-hover:opacity-100"}`}
                        />
                      </li>
                    );
                  })}
                </ul>
              </li>
            ))}
          </ul>
          {list.more && (
            // docs/dictation.md §4.4: the next page loads when this comes into view; the button is
            // the same for the keyboard.
            <div ref={moreRef} className="flex items-center justify-between gap-2 pt-1">
              <span className="mono text-[11px] text-fg-subtle" data-testid="history-loaded">
                {t("history.loaded", {
                  shown: formatCount(visible.length),
                  n: formatCount(list.matching),
                })}
              </span>
              <Button size="sm" variant="text" onClick={list.loadMore} data-testid="history-more">
                {t("history.loadMore")}
              </Button>
            </div>
          )}
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
            <HistoryDetail
              entry={selected}
              view={view}
              onView={setView}
              onCopy={() => {
                copy(selected);
              }}
              onStar={() => {
                star(selected);
              }}
              onRemove={() => {
                remove(selected);
              }}
              starLabel={starLabel(selected)}
            />
          )}
        </Panel>
      </div>
    </div>
  );
}
