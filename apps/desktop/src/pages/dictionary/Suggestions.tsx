import {
  MAX_HEARD_AS,
  type Suggestion,
  type SuggestionRefusal,
  type TFunction,
  relativeTime,
  shortForm,
  suggestionDraft,
  suggestionTarget,
} from "@voltip/shared";
import {
  Button,
  Card,
  Checkbox,
  EmptyState,
  IconButton,
  Panel,
  Segmented,
  Table,
  type TableColumn,
  useI18n,
  useNow,
  useUiState,
} from "@voltip/ui";
import { type ReactNode, useId } from "react";
import { useRouter } from "../../app/router";
import { useBacktest } from "../history/useBacktest";
import { IgnoredList } from "./IgnoredList";
import { Line, Pair, ROW_BAND } from "./parts";
import {
  type BatchNotice,
  type BatchUndoFailure,
  type RowAction,
  type SuggestionsFilter,
  type SuggestionsView,
  type UndoFailure,
  passes,
  suggestionKey,
} from "./useSuggestions";

const CLASH_KEY = {
  heard_is_term: "heardIsTerm",
  heard_taken: "heardTaken",
  term_taken: "termTaken",
  disabled: "disabled",
  too_many: "tooMany",
} as const;

function refusalText(refusal: SuggestionRefusal, row: Suggestion, t: TFunction): string {
  switch (refusal.kind) {
    case "rule":
      return t("dictionary.suggestions.refusal.rule", {
        name: refusal.name,
        polished: row.polished,
        becomes: refusal.becomes,
      });
    case "entry":
      return t(`dictionary.suggestions.refusal.${CLASH_KEY[refusal.clash]}`, {
        heard: row.heard,
        polished: row.polished,
        term: refusal.term,
        max: MAX_HEARD_AS,
      });
    case "full":
      return t("dictionary.suggestions.refusal.full", { max: refusal.max });
    case "failed":
      return t("dictionary.suggestions.refusal.failed", { message: refusal.message });
  }
}

function undoFailureText(failure: UndoFailure, t: TFunction): string {
  return failure.kind === "failed"
    ? t("dictionary.suggestions.undoFailed.failed", { message: failure.message })
    : t(`dictionary.suggestions.undoFailed.${failure.kind}`);
}

function batchUndoFailureText(failure: BatchUndoFailure, t: TFunction): string {
  switch (failure.kind) {
    case "expired":
      return t("dictionary.suggestions.batch.undoFailed.expired");
    case "gone":
    case "changed":
      return t(`dictionary.suggestions.batch.undoFailed.${failure.kind}`, { term: failure.term });
    case "failed":
      return t("dictionary.suggestions.batch.undoFailed.failed", { message: failure.message });
  }
}

/** What a batch did, in a sentence. */
function batchText(
  notice: Extract<BatchNotice, { kind: "accept" | "ignore" }>,
  t: TFunction,
): string {
  const n = notice.keys.length;
  if (notice.kind === "ignore") return t("dictionary.suggestions.batch.ignored", { n });
  if (n === 0) return t("dictionary.suggestions.batch.refused", { n: notice.refused });
  return notice.refused === 0
    ? t("dictionary.suggestions.batch.accepted", { n })
    : t("dictionary.suggestions.batch.acceptedSome", { n, refused: notice.refused });
}

/** The batches over the list, newest first: what each did, and its 撤销 while there is one. */
function BatchNotices({ view }: { view: SuggestionsView }) {
  const { t } = useI18n();
  if (view.batches.length === 0) return null;
  return (
    <div className="mb-2 flex flex-col gap-1.5 text-[12px] leading-[18px]">
      {view.batches.map((notice) => {
        if (notice.kind === "failed") {
          return (
            <div key={notice.id} role="status" data-testid="suggestions-batch">
              <Line
                tone="danger"
                text={t("dictionary.suggestions.actionFailed", { message: notice.message })}
              />
            </div>
          );
        }
        const text = batchText(notice, t);
        const tone = notice.keys.length === 0 || notice.refused > 0 ? "warn" : "info";
        return (
          <div
            key={notice.id}
            role="status"
            data-testid="suggestions-batch"
            className="flex flex-col gap-1">
            <div className="flex flex-wrap items-center gap-x-2">
              <Line tone={tone} text={text} />
              {notice.undo !== undefined && notice.failure === undefined && (
                <Button
                  size="sm"
                  variant="text"
                  aria-label={t("dictionary.suggestions.batch.undo", { what: text })}
                  loading={notice.undoing}
                  onClick={() => {
                    view.undoBatch(notice.id);
                  }}>
                  {t("dictionary.suggestions.undo")}
                </Button>
              )}
            </div>
            {notice.failure !== undefined && (
              <Line tone="danger" text={batchUndoFailureText(notice.failure, t)} />
            )}
          </div>
        );
      })}
    </div>
  );
}

/** Over the list while rows are checked: how many, and 加入词典 / 忽略 for them. */
function SelectionBar({ view }: { view: SuggestionsView }) {
  const { t } = useI18n();
  const n = view.checked.size;
  if (n === 0) return null;
  return (
    <div
      className="mb-2 flex flex-wrap items-center gap-2 rounded-6 bg-inset px-3 py-1.5 text-[12px]"
      data-testid="suggestions-selection">
      <span className="text-fg">{t("dictionary.suggestions.selection.count", { n })}</span>
      <Button
        size="sm"
        aria-label={t("dictionary.suggestions.selection.acceptLabel", { n })}
        disabled={view.busy}
        onClick={view.acceptChecked}>
        {t("dictionary.suggestions.accept")}
      </Button>
      <Button
        size="sm"
        variant="text-muted"
        aria-label={t("dictionary.suggestions.selection.ignoreLabel", { n })}
        disabled={view.busy}
        onClick={view.ignoreChecked}>
        {t("dictionary.suggestions.ignore")}
      </Button>
      <Button
        size="sm"
        variant="text"
        className="ml-auto"
        onClick={() => {
          view.select(view.selectable, false);
        }}>
        {t("dictionary.suggestions.selection.clear")}
      </Button>
    </div>
  );
}

/** The recognised text around the heard words, with those words lit. */
function Example({ snippet, heard }: { snippet: string; heard: string }) {
  const at = snippet.indexOf(heard);
  if (at < 0) return <>{snippet}</>;
  return (
    <>
      {snippet.slice(0, at)}
      <span className="font-medium text-accent-text">{heard}</span>
      {snippet.slice(at + heard.length)}
    </>
  );
}

/** What 加入词典 would save and what it would change in the recent takes (the history page's
 *  backtest, none left out), and a warning when the heard form is short enough to change other
 *  words; once the row is decided, only where it came from. */
function Details({ row, decided }: { row: Suggestion; decided: boolean }) {
  const { t } = useI18n();
  const { dictionary } = useUiState();
  const label = useId();
  const backtest = useBacktest(
    decided ? undefined : suggestionDraft(dictionary, row.heard, row.polished),
  );
  const target = suggestionTarget(dictionary, row.polished);
  const short = shortForm([row.heard]);
  return (
    <div className="flex flex-col gap-2" data-testid="suggestion-details">
      <div id={label} className="eyebrow">
        {t("dictionary.suggestions.examples")}
      </div>
      <ul aria-labelledby={label} className="flex flex-col">
        {row.examples.map((e) => (
          <li
            key={e.history_id}
            className="border-t border-border py-1.5 text-fg first:border-t-0 first:pt-0"
            data-user-text>
            <Example snippet={e.snippet} heard={row.heard} />
          </li>
        ))}
      </ul>
      {!decided && (
        <>
          <Line
            tone="info"
            text={
              target === undefined
                ? t("dictionary.suggestions.willCreate", { term: row.polished, heard: row.heard })
                : t("dictionary.suggestions.willAdd", {
                    heard: row.heard,
                    term: target.term,
                    n: target.heard_as.length,
                  })
            }
          />
          {backtest.kind === "computing" && (
            <Line tone="busy" text={t("dictionary.suggestions.computing")} />
          )}
          {backtest.kind === "error" && (
            <Line
              tone="danger"
              text={t("dictionary.suggestions.impactFailed", { message: backtest.message })}
            />
          )}
          {backtest.kind === "ok" && (
            <Line
              tone="info"
              text={
                backtest.backtest.matched === 0
                  ? t("dictionary.suggestions.impactNone", { scanned: backtest.backtest.scanned })
                  : t("dictionary.suggestions.impact", {
                      scanned: backtest.backtest.scanned,
                      n: backtest.backtest.matched,
                    })
              }
            />
          )}
          {backtest.kind === "ok" && backtest.backtest.kept > 0 && (
            <Line
              tone="warn"
              text={t("dictionary.suggestions.kept", {
                n: backtest.backtest.kept,
                heard: row.heard,
              })}
            />
          )}
          {short !== undefined && (
            <Line
              tone="warn"
              text={
                short.latin
                  ? t("dictionary.suggestions.shortLatin", { heard: row.heard, n: short.chars })
                  : t("dictionary.suggestions.short", { heard: row.heard })
              }
            />
          )}
        </>
      )}
    </div>
  );
}

/** A row's 加入词典 / 忽略, or what became of them and its 撤销, and the details toggle. */
function RowButtons({
  row,
  action,
  expanded,
  view,
}: {
  row: Suggestion;
  action: RowAction | undefined;
  expanded: boolean;
  view: SuggestionsView;
}) {
  const { t } = useI18n();
  const names = { heard: row.heard, polished: row.polished };
  const undo = (busy: boolean) => (
    <Button
      size="sm"
      variant="text"
      aria-label={t("dictionary.suggestions.row.undo", names)}
      disabled={busy}
      onClick={() => {
        view.undo(row);
      }}>
      {t("dictionary.suggestions.undo")}
    </Button>
  );
  const decided =
    action?.kind === "accepted" ? (
      <>
        <span className="text-[12px] text-ok-text">{t("dictionary.suggestions.accepted")}</span>
        {undo(action.busy)}
      </>
    ) : action?.kind === "kept" ? (
      <span className="text-[12px] text-ok-text">{t("dictionary.suggestions.accepted")}</span>
    ) : action?.kind === "ignored" ? (
      <>
        <span className="text-[12px] text-fg-muted">{t("dictionary.suggestions.ignored")}</span>
        {undo(action.busy)}
      </>
    ) : action?.kind === "batched" ? (
      // 撤销 is the batch's, over the list.
      action.what === "accept" ? (
        <span className="text-[12px] text-ok-text">{t("dictionary.suggestions.accepted")}</span>
      ) : (
        <span className="text-[12px] text-fg-muted">{t("dictionary.suggestions.ignored")}</span>
      )
    ) : undefined;
  const busy = action?.kind === "busy" ? action.what : undefined;
  return (
    <span className="inline-flex items-center gap-2">
      {decided ?? (
        <>
          <Button
            size="sm"
            aria-label={t("dictionary.suggestions.row.accept", names)}
            loading={busy === "accept"}
            disabled={busy !== undefined}
            onClick={() => {
              view.accept(row);
            }}>
            {t("dictionary.suggestions.accept")}
          </Button>
          <Button
            size="sm"
            variant="text-muted"
            aria-label={t("dictionary.suggestions.row.ignore", names)}
            loading={busy === "ignore"}
            disabled={busy !== undefined}
            onClick={() => {
              view.ignore(row);
            }}>
            {t("dictionary.suggestions.ignore")}
          </Button>
        </>
      )}
      <IconButton
        icon={expanded ? "chevronUp" : "chevronDown"}
        label={t("dictionary.suggestions.row.details", names)}
        aria-expanded={expanded}
        onClick={() => {
          view.toggle(row);
        }}
      />
    </span>
  );
}

export interface SuggestionsPanelProps {
  view: SuggestionsView;
  /** 查看词条 on a refusal: the entry in the way, in the entries view. */
  onShowEntry: (id: string) => void;
}

/** 学习建议 (docs/dictation.md §16.10): the pairs AI polish changed in the recent takes that the
 *  dictionary does not correct yet, most frequent first; by default those seen twice or more. 加入词典
 *  and 忽略 go to the shell, which checks and saves them, for one row or for the checked rows at
 *  once (全部忽略: every row shown); a row acted on stays in place, with its 撤销 or its batch's, until
 *  the page closes. The ignored pairs follow under 已忽略. */
export function SuggestionsPanel({ view, onShowEntry }: SuggestionsPanelProps) {
  const { t, locale } = useI18n();
  const { navigate } = useRouter();
  const now = useNow();
  const { load, visible, actions, expanded, selectable, checked } = view;
  const answer = load.kind === "answered" ? load.answer : undefined;
  const allChecked = selectable.length > 0 && checked.size === selectable.length;

  const columns: TableColumn<Suggestion>[] = [
    {
      id: "select",
      header: (
        <Checkbox
          label={t("dictionary.suggestions.select.all")}
          checked={allChecked}
          indeterminate={checked.size > 0 && !allChecked}
          disabled={selectable.length === 0 || view.busy}
          onChange={(on) => {
            view.select(selectable, on);
          }}
        />
      ),
      width: 32,
      align: "center",
      mono: false,
      cell: (s) => {
        const key = suggestionKey(s);
        const action = actions.get(key);
        if (action?.kind === "busy") {
          return (
            <Checkbox
              label={t("dictionary.suggestions.select.row", {
                heard: s.heard,
                polished: s.polished,
              })}
              checked={false}
              disabled
              onChange={() => undefined}
            />
          );
        }
        if (!selectable.includes(s)) return null;
        return (
          <Checkbox
            label={t("dictionary.suggestions.select.row", { heard: s.heard, polished: s.polished })}
            checked={checked.has(key)}
            disabled={view.busy}
            onChange={(on) => {
              view.select([s], on);
            }}
          />
        );
      },
    },
    {
      id: "pair",
      header: t("dictionary.suggestions.column.pair"),
      mono: false,
      // The header and a short pair stay whole when the window is narrow.
      minWidth: 160,
      cell: (s) => <Pair heard={s.heard} polished={s.polished} testId="suggestion-pair" />,
    },
    {
      id: "count",
      header: t("dictionary.suggestions.column.count"),
      width: 56,
      align: "right",
      cell: (s) => ({ type: "mono", text: String(s.count) }),
    },
    {
      id: "last",
      header: t("dictionary.suggestions.column.last"),
      fit: true,
      mono: false,
      cell: (s) => ({
        type: "text",
        text: relativeTime(Math.floor(s.last_at_ms / 1000), now, locale),
        muted: true,
      }),
    },
    {
      id: "actions",
      header: "",
      fit: true,
      align: "right",
      mono: false,
      cell: (s) => {
        const key = suggestionKey(s);
        return (
          <RowButtons row={s} action={actions.get(key)} expanded={expanded === key} view={view} />
        );
      },
    },
  ];

  /** Why an action on a row did nothing, with a way to what is in the way. */
  const messageOf = (row: Suggestion, action: RowAction | undefined): ReactNode => {
    switch (action?.kind) {
      case "refused": {
        const { refusal } = action;
        const link =
          refusal.kind === "entry"
            ? {
                label: t("dictionary.suggestions.showEntry"),
                onClick: () => {
                  onShowEntry(refusal.entry_id);
                },
              }
            : refusal.kind === "rule"
              ? {
                  label: t("dictionary.suggestions.openRules"),
                  onClick: () => {
                    navigate({ name: "rules" });
                  },
                }
              : undefined;
        return <Line tone="danger" text={refusalText(refusal, row, t)} link={link} />;
      }
      case "kept":
        return <Line tone="danger" text={undoFailureText(action.failure, t)} />;
      case "failed":
        return (
          <Line
            tone="danger"
            text={t("dictionary.suggestions.actionFailed", { message: action.message })}
          />
        );
      default:
        return undefined;
    }
  };
  /** Under a row: why an action did nothing, and its details when open. */
  const more = (row: Suggestion): ReactNode => {
    const key = suggestionKey(row);
    const action = actions.get(key);
    const message = messageOf(row, action);
    const open = expanded === key;
    if (message === undefined && !open) return null;
    const decided =
      action?.kind === "accepted" ||
      action?.kind === "kept" ||
      action?.kind === "ignored" ||
      action?.kind === "batched";
    return (
      <div className={ROW_BAND}>
        {message !== undefined && <div role="status">{message}</div>}
        {open && <Details row={row} decided={decided} />}
      </div>
    );
  };

  const empty =
    load.kind === "loading" ? (
      <EmptyState compact title={t("dictionary.suggestions.loading")} />
    ) : load.kind === "failed" ? (
      <EmptyState
        compact
        title={t("dictionary.suggestions.loadFailed", { message: load.message })}
      />
    ) : load.answer.status === "history_off" ? (
      <EmptyState
        title={t("dictionary.suggestions.empty.historyOffTitle")}
        actions={
          <Button
            size="sm"
            variant="primary"
            onClick={() => {
              navigate({ name: "settings", section: "privacy" });
            }}>
            {t("dictionary.suggestions.empty.openPrivacy")}
          </Button>
        }>
        {t("dictionary.suggestions.empty.historyOff")}
      </EmptyState>
    ) : load.answer.refined === 0 ? (
      <EmptyState title={t("dictionary.suggestions.empty.noPolishedTitle")}>
        {t("dictionary.suggestions.empty.noPolished")}
      </EmptyState>
    ) : load.answer.total > 0 && view.filter === "repeated" ? (
      <EmptyState
        title={t("dictionary.suggestions.empty.noneRepeatedTitle")}
        actions={
          <Button
            size="sm"
            onClick={() => {
              view.setFilter("all");
            }}>
            {t("dictionary.suggestions.empty.showAll")}
          </Button>
        }>
        {t("dictionary.suggestions.empty.noneRepeated", { n: load.answer.total })}
      </EmptyState>
    ) : (
      <EmptyState title={t("dictionary.suggestions.empty.noneTitle")}>
        {t("dictionary.suggestions.empty.none", { n: load.answer.refined })}
      </EmptyState>
    );

  /** Under the list: the takes compared, then what the list shows and what it does not. */
  const footer = (() => {
    if (answer?.status !== "ready") return undefined;
    // The suggestions the filter takes, of all of them, and how many the answer lists.
    const inView = view.filter === "all" ? answer.total : answer.repeated;
    const listed = answer.pairs.filter((p) => passes(p, view.filter)).length;
    const hiddenOnce = view.filter === "repeated" ? answer.total - answer.repeated : 0;
    return [
      t("dictionary.suggestions.footer", { n: answer.scanned, refined: answer.refined }),
      ...(answer.skipped > 0 ? [t("dictionary.suggestions.skipped", { n: answer.skipped })] : []),
      inView > listed
        ? t("dictionary.suggestions.more", { shown: listed, total: inView })
        : t("dictionary.suggestions.shown", { n: visible.length }),
      ...(hiddenOnce > 0 ? [t("dictionary.suggestions.hiddenOnce", { n: hiddenOnce })] : []),
    ].join(" · ");
  })();

  /** Over the list once there is something to choose from: the filter and 全部忽略. */
  const tools =
    answer?.status === "ready" && (answer.total > 0 || view.rows.length > 0) ? (
      <>
        <Segmented<SuggestionsFilter>
          label={t("dictionary.suggestions.filter.label")}
          variant="ink"
          size="sm"
          value={view.filter}
          onChange={view.setFilter}
          options={[
            { value: "repeated", label: t("dictionary.suggestions.filter.repeated") },
            { value: "all", label: t("dictionary.suggestions.filter.all") },
          ]}
        />
        {selectable.length > 0 && (
          <Button size="sm" variant="text-muted" disabled={view.busy} onClick={view.ignoreShown}>
            {t("dictionary.suggestions.ignoreAll", { n: selectable.length })}
          </Button>
        )}
      </>
    ) : undefined;

  return (
    <>
      <Card padding="sm" className="flex flex-col" data-testid="suggestions-explain">
        <div className="text-[13px] font-medium text-fg">
          {t("dictionary.suggestions.explainTitle")}
        </div>
        <div className="text-[12px] leading-5 text-fg-muted">
          {t("dictionary.suggestions.explain")}
        </div>
        <div className="mt-1 text-[11px] text-fg-subtle">
          {t("dictionary.suggestions.undoNote")}
        </div>
      </Card>
      <Panel eyebrow={t("dictionary.suggestions.list")} padding="sm" right={tools}>
        <SelectionBar view={view} />
        <BatchNotices view={view} />
        {visible.length === 0 ? (
          empty
        ) : (
          <Table
            label={t("dictionary.suggestions.table")}
            columns={columns}
            rows={visible}
            rowKey={suggestionKey}
            expandRow={more}
          />
        )}
        {footer !== undefined && (
          <div className="mono mt-2 text-[11px] text-fg-subtle" data-testid="suggestions-footer">
            {footer}
          </div>
        )}
      </Panel>
      <IgnoredList />
    </>
  );
}
