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
  refineModelText,
} from "@voltip/shared";
import {
  Button,
  Card,
  EmptyState,
  Eyebrow,
  LampText,
  Panel,
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
import { useEffect, useState } from "react";
import { Fact, Facts, PAGE, TOUCH } from "../app/phone-ui";
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
    <section aria-label={t("history.long.title")} data-testid="phone-entry-long">
      <Panel eyebrow={t("history.long.title")} bodyClassName="flex flex-col gap-3">
        <div className="flex items-center gap-2">
          {/* The select takes the rest of the row and may shrink below its longest preset. */}
          <div className="min-w-0 flex-1">
            <Select
              aria-label={t("history.long.preset")}
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
          </div>
          {running ? (
            <Button variant="outline" className={TOUCH} onClick={process.cancel}>
              {t("common.cancel")}
            </Button>
          ) : (
            <Button
              variant="primary"
              className={TOUCH}
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
          <p className="text-[12px] leading-5 text-danger">
            {t("history.long.failed", { reason: view.reason })}
          </p>
        )}
        {view.state === "cancelled" && (
          <p className="text-[12px] leading-5 text-fg-muted">{t("history.long.cancelled")}</p>
        )}
        <p className="text-[12px] leading-5 text-fg-subtle">{t("history.long.note")}</p>
        <div className="flex flex-wrap gap-2 border-t border-border pt-3">
          <Button
            icon="share"
            className={TOUCH}
            disabled={!hasSegments}
            onClick={() => {
              void share("srt");
            }}>
            {t("mobile.entry.shareSrt")}
          </Button>
          <Button
            icon="share"
            className={TOUCH}
            onClick={() => {
              void share("txt");
            }}>
            {t("mobile.entry.shareTxt")}
          </Button>
        </div>
        {processed !== undefined && (
          <span className="text-[12px] text-fg-muted">{t("history.long.txtUsesProcessed")}</span>
        )}
      </Panel>
    </section>
  );
}

/** The day, the time and how the take ended, above its text: the desktop's detail header (the
 *  eyebrow left, the outcome's lamp right). */
export function EntryHeader({
  when,
  outcome,
}: {
  when: string;
  outcome: ReturnType<typeof outcomeLabel>;
}) {
  return (
    <Eyebrow
      className="px-1"
      right={
        <LampText tone={outcome.tone} size="sm">
          {outcome.text}
        </LampText>
      }>
      {when}
    </Eyebrow>
  );
}

/** The text of an entry as the chosen view shows it: 15 px, selectable, in a hairline card. */
export function EntryText({ text, testId }: { text: string; testId: string }) {
  return (
    <Card>
      <p
        className="text-[15px] leading-7 break-words whitespace-pre-wrap text-fg select-text"
        data-testid={testId}
        data-user-text>
        {text}
      </p>
    </Card>
  );
}

/** One entry of the phone's history (docs/dictation.md §20.7): its text as polished, as
 *  recognised and, after 用 AI 预设处理, as processed; when and how it was made; copy, share,
 *  star and delete (after a confirmation); and for a long entry the processing and the exports. */
export function HistoryEntry() {
  const shell = useMobileShell();
  const { backend } = useBackend();
  const { t, locale } = useI18n();
  // Milliseconds for `dayLabel` (`useNow` is Unix seconds).
  const now = useNow() * 1000;
  const id = shell.param ?? "";
  const entry = useEntry(id);
  const tooLarge = useUiState().phone_outbox_too_large.includes(id);
  const process = useHistoryProcess(id);
  const [view, setView] = useState<View>("polished");

  if (entry === undefined) return null;
  if (entry === null)
    return (
      <div className={PAGE}>
        <Card padding="none">
          <EmptyState compact title={t("history.long.exportFailed.gone")}>
            {t("mobile.entry.goneBody")}
          </EmptyState>
        </Card>
      </div>
    );

  const processed = process.view.state === "done" ? process.view.processed : entry.processed;
  const shown: View = view === "processed" && processed === undefined ? "polished" : view;
  const text =
    shown === "raw" ? entry.raw_text : shown === "processed" ? (processed?.text ?? "") : entry.text;
  const outcome = outcomeLabel(entry.outcome, locale);
  // A take a computer delivered (docs/dictation.md §20.7): the phone has the text the computer
  // reported and the length of the audio; the models and timings are in the computer's history.
  const sent = entry.origin?.kind === "sent";
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
    <div className={PAGE} data-testid="phone-entry">
      <EntryHeader
        when={`${dayLabel(entry.at_ms, now, locale)} ${shortClockLabel(entry.at_ms)}`}
        outcome={outcome}
      />
      {entry.refined && (
        <Segmented
          label={t("history.view.label")}
          value={shown}
          onChange={setView}
          options={views}
          className="h-11 w-full [&>button]:flex-1"
        />
      )}
      {shown === "processed" && processed !== undefined && (
        <p className="px-1 text-[12px] leading-5 text-fg-muted" data-user-text>
          {t("history.view.processedBy", { preset: processed.preset.name })}
        </p>
      )}
      <EntryText text={text} testId="phone-entry-text" />
      <div className="flex flex-wrap items-center gap-2">
        <Button icon="copy" className={TOUCH} onClick={copy}>
          {t("mobile.recent.copy")}
        </Button>
        <Button
          icon="share"
          className={TOUCH}
          onClick={() => {
            backend.invoke("phone_share_text", { text }).catch(fail);
          }}>
          {t("mobile.recent.share")}
        </Button>
        <Button
          icon="star"
          className={TOUCH}
          aria-pressed={entry.starred}
          onClick={() => {
            backend.invoke("history_star", { id: entry.id, starred: !entry.starred }).catch(fail);
          }}>
          {entry.starred ? t("history.unstar") : t("history.star")}
        </Button>
        <Button variant="text-danger" className={`${TOUCH} ml-auto`} onClick={remove}>
          {t("common.delete")}
        </Button>
      </div>
      {tooLarge && (
        <p className="px-1 text-[12px] leading-5 text-fg-muted" data-testid="phone-entry-too-large">
          {t("mobile.entry.tooLarge")}
        </p>
      )}
      {isLongEntry(entry) && <LongTools entry={entry} process={process} />}
      <Facts>
        {sent && (
          <Fact label={t("history.detail.origin")}>
            <span data-user-text data-origin="sent">
              {t("history.origin.sent", { device: entry.origin?.device ?? "" })}
            </span>
          </Fact>
        )}
        <Fact label={t("history.detail.duration")}>
          {formatDuration(entry.duration_ms, locale)}
        </Fact>
        <Fact label={t("history.detail.chars")}>
          <span className="mono">{textChars(entry.text)}</span>
        </Fact>
        {!sent && (
          <>
            <Fact label={t("history.detail.asrModel")}>
              <span className="mono">{entry.asr_model}</span>
            </Fact>
            <Fact label={t("history.detail.refineModel")}>
              {entry.refine_model === undefined ? (
                refineModelText(entry, locale)
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
          </>
        )}
      </Facts>
      {sent && (
        <p className="px-1 text-[12px] leading-5 text-fg-muted" data-testid="phone-entry-sent-note">
          {t("mobile.entry.sentNote")}
        </p>
      )}
    </div>
  );
}
