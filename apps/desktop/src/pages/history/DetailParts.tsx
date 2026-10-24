// Parts of the history detail moved out of `History.tsx` unchanged: a voice edit's detail, the
// dictionary corrections and rules that fired, and the timing bar.
import {
  type EditRecord,
  type HistoryEntry,
  type TFunction,
  type VocabularyHit,
  formatCount,
} from "@voltip/shared";
import { Lamp, useUiState } from "@voltip/ui";
import { textChars } from "../../features/history/stats";

/** A voice edit's detail (docs/dictation.md §19.5): the instruction as sent to the LLM, the
 *  rewrite that was pasted, and the original selection behind a disclosure. */
export function EditDetail({
  entry,
  edit,
  t,
}: {
  entry: HistoryEntry;
  edit: EditRecord;
  t: TFunction;
}) {
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

/** Which dictionary entries and rules fired in this take, named from the current lists. */
export function VocabularyHits({
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

export function TimingBar({ entry, t }: { entry: HistoryEntry; t: TFunction }) {
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
            : entry.refine_failure !== undefined
              ? // §3.6: a clean-up was asked for and failed; it was not off (2026-10-24: the row
                // said 未启用 for a request the service turned down).
                t("history.timing.refineFailed", {
                  reason: t(`dictation.refineFailure.${entry.refine_failure}`),
                })
              : t("history.timing.refineOff")}
        </span>
        <span className="ml-auto text-fg">
          {t("history.timing.total", { n: formatCount(total) })}
        </span>
      </div>
    </div>
  );
}
