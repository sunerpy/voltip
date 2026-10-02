import {
  BUILTIN_PRESETS,
  type ExportFormat,
  type HistoryEntry as Entry,
  dayLabel,
  errorText,
  formatCount,
  formatDuration,
  isBuiltinPreset,
  outcomeLabel,
  presetLabel,
  presetRefLabel,
  sceneLabel,
  shortClockLabel,
  textChars,
} from "@voltip/shared";
import {
  Button,
  EmptyState,
  Progress,
  Segmented,
  Select,
  exportName,
  isLongEntry,
  useBackend,
  useHistoryProcess,
  useI18n,
  useNow,
  useUiState,
} from "@voltip/ui";
import { type ReactNode, useEffect, useState } from "react";
import { useMobileShell } from "../app/shell";

type View = "polished" | "raw" | "processed";

/** The entry `id` from the core (`history_entry`), asked again on every history event (a star, a
 *  deletion): `undefined` until the answer, `null` once it is gone. */
function useEntry(id: string): Entry | null | undefined {
  const { backend } = useBackend();
  const { history_recent: revision } = useUiState();
  const [answer, setAnswer] = useState<{ id: string; entry: Entry | null } | undefined>(undefined);
  useEffect(() => {
    let live = true;
    backend.historyEntry(id).then(
      (entry) => {
        if (live) setAnswer({ id, entry });
      },
      () => {
        if (live) setAnswer({ id, entry: null });
      },
    );
    return () => {
      live = false;
    };
    // A history event (a star, a deletion, a processed text) is a reason to ask again.
    // oxlint-disable-next-line react/exhaustive-effect-dependencies
  }, [backend, id, revision]);
  return answer?.id === id ? answer.entry : undefined;
}

function Fact({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-start justify-between gap-3 py-2">
      <dt className="shrink-0 text-[12px] text-fg-muted">{label}</dt>
      <dd className="min-w-0 text-right text-[12px] break-words text-fg">{children}</dd>
    </div>
  );
}

/** 用 AI 预设处理 and the two exports of a long entry (docs/dictation.md §22), the desktop's tools
 *  on the phone: the exports go to the share sheet as files. */
function LongTools({
  entry,
  process,
}: {
  entry: Entry;
  process: ReturnType<typeof useHistoryProcess>;
}) {
  const { backend } = useBackend();
  const shell = useMobileShell();
  const { t, locale } = useI18n();
  const { settings, presets } = useUiState();
  const [preset, setPreset] = useState<string>(settings.engines.refine_preset);
  const view = process.view;
  const running = view.state === "running";
  const processed = view.state === "done" ? view.processed : entry.processed;
  const hasSegments = (entry.segments ?? []).some((s) => s.text.trim().length > 0);
  const share = async (format: ExportFormat) => {
    const outcome = await backend.historyExport(entry.id, format, exportName(entry.at_ms));
    if (outcome.kind === "failed")
      shell.toast(
        t(`history.long.exportFailed.${outcome.code}`, { detail: outcome.detail }),
        "danger",
      );
  };
  return (
    <section
      className="flex flex-col gap-3 rounded-10 bg-surface p-3 hairline"
      aria-label={t("history.long.title")}
      data-testid="phone-entry-long">
      <span className="text-[13px] font-medium text-fg">{t("history.long.title")}</span>
      <div className="flex items-center gap-2">
        <Select
          aria-label={t("history.long.preset")}
          size="sm"
          className="flex-1"
          value={preset}
          disabled={running}
          options={[
            ...BUILTIN_PRESETS.map((id) => ({
              value: id,
              label: presetLabel(id, presets, locale),
            })),
            ...presets.map((p) => ({ value: p.id, label: p.name })),
          ]}
          onChange={setPreset}
        />
        {running ? (
          <Button size="sm" variant="outline" onClick={process.cancel}>
            {t("common.cancel")}
          </Button>
        ) : (
          <Button
            size="sm"
            variant="primary"
            onClick={() => {
              process.start(preset);
            }}>
            {processed === undefined ? t("history.long.start") : t("history.long.again")}
          </Button>
        )}
      </div>
      {running && (
        <div className="flex items-center gap-3">
          <Progress
            value={view.total === 0 ? 0 : view.done / view.total}
            indeterminate={view.total === 0}
            size={2}
            className="flex-1"
          />
          <span className="mono text-[11px] text-fg-muted">
            {t("history.long.running", { done: view.done, total: view.total })}
          </span>
        </div>
      )}
      {view.state === "failed" && (
        <p className="text-[12px] text-danger">
          {t("history.long.failed", { reason: view.reason })}
        </p>
      )}
      {view.state === "cancelled" && (
        <p className="text-[12px] text-fg-muted">{t("history.long.cancelled")}</p>
      )}
      <p className="text-[11px] leading-4 text-fg-subtle">{t("history.long.note")}</p>
      <div className="flex flex-wrap gap-2 border-t border-border pt-3">
        <Button
          size="sm"
          icon="share"
          disabled={!hasSegments}
          onClick={() => {
            void share("srt");
          }}>
          {t("mobile.entry.shareSrt")}
        </Button>
        <Button
          size="sm"
          icon="share"
          onClick={() => {
            void share("txt");
          }}>
          {t("mobile.entry.shareTxt")}
        </Button>
      </div>
      {processed !== undefined && (
        <span className="text-[11px] text-fg-muted">{t("history.long.txtUsesProcessed")}</span>
      )}
    </section>
  );
}

/** One entry of the phone's history (docs/dictation.md §20.7): its text as polished, as
 *  recognised and, after 用 AI 预设处理, as processed; when and how it was made; copy, share,
 *  star and delete (after a confirmation); and for a long entry the processing and the exports. */
export function HistoryEntry() {
  const shell = useMobileShell();
  const { backend } = useBackend();
  const { t, locale } = useI18n();
  const now = useNow();
  const id = shell.param ?? "";
  const entry = useEntry(id);
  const tooLarge = useUiState().phone_outbox_too_large.includes(id);
  const process = useHistoryProcess(id);
  const [view, setView] = useState<View>("polished");

  if (entry === undefined) return null;
  if (entry === null)
    return (
      <div className="p-4">
        <EmptyState compact title={t("history.long.exportFailed.gone")}>
          {t("mobile.entry.goneBody")}
        </EmptyState>
      </div>
    );

  const processed = process.view.state === "done" ? process.view.processed : entry.processed;
  const shown: View = view === "processed" && processed === undefined ? "polished" : view;
  const text =
    shown === "raw" ? entry.raw_text : shown === "processed" ? (processed?.text ?? "") : entry.text;
  const outcome = outcomeLabel(entry.outcome, locale);
  const fail = (e: unknown) => {
    shell.toast(t("mobile.toast.error", { message: errorText(e) }), "danger");
  };
  const copy = () => {
    void backend.pasteText(text).then(
      (answer) => {
        if (answer.kind === "failed") shell.toast(t("mobile.recent.copyFailed"), "danger");
        else shell.toast(t("history.detail.copied", { n: textChars(text) }));
      },
      () => {
        shell.toast(t("mobile.recent.copyFailed"), "danger");
      },
    );
  };
  const remove = () => {
    shell.confirm({
      title: t("history.confirm.deleteTitle"),
      body: t("history.confirm.deleteBody", {
        when: `${dayLabel(entry.at_ms, now, locale)} ${shortClockLabel(entry.at_ms)}`,
        excerpt: `${entry.text.slice(0, 40)}${entry.text.length > 40 ? "…" : ""}`,
      }),
      confirmLabel: t("common.delete"),
      onConfirm: () => {
        backend.invoke("history_delete", { id: entry.id }).then(shell.back, fail);
      },
    });
  };
  const views: { value: View; label: string }[] = [
    { value: "polished", label: t("history.view.polished") },
    { value: "raw", label: t("history.view.raw") },
    ...(processed === undefined
      ? []
      : [{ value: "processed" as const, label: t("history.view.processed") }]),
  ];

  return (
    <div className="flex flex-col gap-3 p-4" data-testid="phone-entry">
      <div className="flex items-center gap-2 text-[12px] text-fg-muted">
        <span>{`${dayLabel(entry.at_ms, now, locale)} ${shortClockLabel(entry.at_ms)}`}</span>
        <span>{outcome.text}</span>
      </div>
      {entry.refined && (
        <Segmented
          label={t("history.view.label")}
          size="sm"
          value={shown}
          onChange={setView}
          options={views}
        />
      )}
      {shown === "processed" && processed !== undefined && (
        <p className="text-[12px] text-fg-muted" data-user-text>
          {t("history.view.processedBy", { preset: processed.preset.name })}
        </p>
      )}
      <p
        className="rounded-10 bg-surface p-4 text-[15px] leading-7 whitespace-pre-wrap break-words text-fg hairline"
        data-testid="phone-entry-text"
        data-user-text>
        {text}
      </p>
      <div className="flex flex-wrap gap-2">
        <Button size="sm" icon="copy" onClick={copy}>
          {t("mobile.recent.copy")}
        </Button>
        <Button
          size="sm"
          icon="share"
          onClick={() => {
            backend.invoke("phone_share_text", { text }).catch(fail);
          }}>
          {t("mobile.recent.share")}
        </Button>
        <Button
          size="sm"
          icon="star"
          aria-pressed={entry.starred}
          onClick={() => {
            backend.invoke("history_star", { id: entry.id, starred: !entry.starred }).catch(fail);
          }}>
          {entry.starred ? t("history.unstar") : t("history.star")}
        </Button>
        <Button size="sm" variant="text-danger" className="ml-auto" onClick={remove}>
          {t("common.delete")}
        </Button>
      </div>
      {tooLarge && (
        <p className="text-[12px] text-fg-muted" data-testid="phone-entry-too-large">
          {t("mobile.entry.tooLarge")}
        </p>
      )}
      {isLongEntry(entry) && <LongTools entry={entry} process={process} />}
      <dl className="flex flex-col divide-y divide-border rounded-10 bg-surface px-4 hairline">
        <Fact label={t("history.detail.duration")}>
          {formatDuration(entry.duration_ms, locale)}
        </Fact>
        <Fact label={t("history.detail.chars")}>{textChars(entry.text)}</Fact>
        <Fact label={t("history.detail.asrModel")}>
          <span className="mono">{entry.asr_model}</span>
        </Fact>
        <Fact label={t("history.detail.refineModel")}>
          {entry.refine_model === undefined ? (
            t("history.detail.notRefined")
          ) : (
            <span className="mono">{entry.refine_model}</span>
          )}
        </Fact>
        {entry.preset !== undefined && (
          <Fact label={t("history.detail.preset")}>
            <span {...(isBuiltinPreset(entry.preset.id) ? {} : { "data-user-text": "" })}>
              {presetRefLabel(entry.preset, locale)}
            </span>
          </Fact>
        )}
        {entry.scene !== undefined && (
          <Fact label={t("history.context.scene")}>
            <span data-user-text>{sceneLabel(entry.scene, locale)}</span>
          </Fact>
        )}
        <Fact label={t("mobile.entry.time")}>
          {t("history.timing.total", { n: formatCount(entry.asr_ms + (entry.refine_ms ?? 0)) })}
        </Fact>
      </dl>
    </div>
  );
}
