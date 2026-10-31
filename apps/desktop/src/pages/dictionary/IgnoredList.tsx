import { type IgnoredPair, errorText, relativeTime } from "@voltip/shared";
import {
  Button,
  Panel,
  Table,
  type TableColumn,
  useBackend,
  useI18n,
  useNow,
  useUiState,
} from "@voltip/ui";
import { type ReactNode, useState } from "react";
import { Line, Pair, ROW_BAND } from "./parts";
import { suggestionKey } from "./useSuggestions";

/** How many of the newest ignored pairs 已忽略 shows until 显示全部. */
export const IGNORED_SHOWN = 5;

/** A 取消忽略 on its way to the shell, or why it did nothing. */
type RowState = { kind: "busy" } | { kind: "failed"; message: string };

/** 已忽略 (docs/dictation.md §16.10): the pairs 忽略 left out of 学习建议, newest first, each with
 *  when it was ignored and 取消忽略. The list is the core's (`UiState.vocabulary_ignored`), so it
 *  is there after the page or the app closed; it shows while it has a pair. */
export function IgnoredList() {
  const { t, locale } = useI18n();
  const { backend } = useBackend();
  const { vocabulary_ignored: ignored } = useUiState();
  const now = useNow();
  const [all, setAll] = useState(false);
  const [states, setStates] = useState<ReadonlyMap<string, RowState>>(new Map());
  if (ignored.length === 0) return null;

  // The core keeps them in the order they were ignored, oldest first.
  const newest = ignored.reduceRight<IgnoredPair[]>((out, pair) => {
    out.push(pair);
    return out;
  }, []);
  const rows = all ? newest : newest.slice(0, IGNORED_SHOWN);
  const set = (key: string, state: RowState | undefined) => {
    setStates((current) => {
      const next = new Map(current);
      if (state === undefined) next.delete(key);
      else next.set(key, state);
      return next;
    });
  };
  const unignore = (pair: IgnoredPair) => {
    const key = suggestionKey(pair);
    set(key, { kind: "busy" });
    backend
      .invoke("vocabulary_suggestion_unignore", { heard: pair.heard, polished: pair.polished })
      .then(
        () => {
          set(key, undefined);
        },
        (e: unknown) => {
          set(key, { kind: "failed", message: errorText(e) });
        },
      );
  };

  const columns: TableColumn<IgnoredPair>[] = [
    {
      id: "pair",
      header: t("dictionary.suggestions.ignoredList.column.pair"),
      mono: false,
      minWidth: 160,
      cell: (p) => <Pair heard={p.heard} polished={p.polished} testId="ignored-pair" />,
    },
    {
      id: "at",
      header: t("dictionary.suggestions.ignoredList.column.at"),
      fit: true,
      mono: false,
      cell: (p) => ({
        type: "text",
        text: relativeTime(Math.floor(p.at_ms / 1000), now, locale),
        muted: true,
      }),
    },
    {
      id: "actions",
      header: "",
      fit: true,
      align: "right",
      mono: false,
      cell: (p) => {
        const busy = states.get(suggestionKey(p))?.kind === "busy";
        return (
          <Button
            size="sm"
            variant="text"
            aria-label={t("dictionary.suggestions.ignoredList.row", {
              heard: p.heard,
              polished: p.polished,
            })}
            loading={busy}
            onClick={() => {
              unignore(p);
            }}>
            {t("dictionary.suggestions.ignoredList.unignore")}
          </Button>
        );
      },
    },
  ];
  /** Under a pair: why its 取消忽略 did nothing. */
  const more = (p: IgnoredPair): ReactNode => {
    const state = states.get(suggestionKey(p));
    if (state?.kind !== "failed") return null;
    return (
      <div className={ROW_BAND}>
        <div role="status">
          <Line
            tone="danger"
            text={t("dictionary.suggestions.ignoredList.failed", { message: state.message })}
          />
        </div>
      </div>
    );
  };

  return (
    <Panel
      eyebrow={t("dictionary.suggestions.ignoredList.title", { n: ignored.length })}
      padding="sm"
      data-testid="suggestions-ignored"
      right={
        ignored.length > IGNORED_SHOWN ? (
          <Button
            size="sm"
            variant="text"
            aria-expanded={all}
            onClick={() => {
              setAll(!all);
            }}>
            {all
              ? t("dictionary.suggestions.ignoredList.showLess")
              : t("dictionary.suggestions.ignoredList.showAll", { n: ignored.length })}
          </Button>
        ) : undefined
      }>
      <Table
        label={t("dictionary.suggestions.ignoredList.table")}
        columns={columns}
        rows={rows}
        rowKey={suggestionKey}
        expandRow={more}
      />
    </Panel>
  );
}
