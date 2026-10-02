import {
  MAX_PASTE_TEXT_CHARS,
  type MirrorEntry as Entry,
  dayLabel,
  errorText,
  formatCount,
  formatDuration,
  isBuiltinPreset,
  outcomeLabel,
  presetRefLabel,
  sceneLabel,
  shortClockLabel,
  textChars,
} from "@voltip/shared";
import { Button, EmptyState, Segmented, useBackend, useI18n, useNow, useUiState } from "@voltip/ui";
import { type ReactNode, useEffect, useState } from "react";
import { useMobileShell } from "../app/shell";

type View = "polished" | "raw" | "processed";

/** `desktop/id`, as the 记录 tab opens this screen. */
export function mirrorEntryParam(desktop: string, id: string): string {
  return `${desktop}/${id}`;
}

function parseParam(param: string | undefined): { desktop: string; id: string } {
  const [desktop = "", id = ""] = (param ?? "").split("/");
  return { desktop, id };
}

/** The entry `id` of the copy of `desktop` (`mirror_history_entry`), asked again whenever the copy
 *  changes: `undefined` until the answer, `null` once it is gone. */
function useMirrorEntry(desktop: string, id: string): Entry | null | undefined {
  const { backend } = useBackend();
  const copy = useUiState().mirrors.find((m) => m.desktop === desktop);
  const revision = `${copy?.state}:${copy?.entries}:${copy?.synced_at_ms}`;
  const [answer, setAnswer] = useState<{ key: string; entry: Entry | null } | undefined>(undefined);
  const key = `${desktop}/${id}`;
  useEffect(() => {
    let live = true;
    backend.mirrorHistoryEntry(desktop, id).then(
      (entry) => {
        if (live) setAnswer({ key, entry });
      },
      () => {
        if (live) setAnswer({ key, entry: null });
      },
    );
    return () => {
      live = false;
    };
    // The copy changed (a batch arrived, the computer deleted it): ask again.
    // oxlint-disable-next-line react/exhaustive-effect-dependencies
  }, [backend, desktop, id, key, revision]);
  return answer?.key === key ? answer.entry : undefined;
}

function Fact({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-start justify-between gap-3 py-2">
      <dt className="shrink-0 text-[12px] text-fg-muted">{label}</dt>
      <dd className="min-w-0 text-right text-[12px] break-words text-fg">{children}</dd>
    </div>
  );
}

/** One entry of a computer's history on the phone (docs/dictation.md §20.8; user decision
 *  2026-10-02): read-only — the text as polished, as recognised and as processed, when and how it
 *  was made and where it came from; copy and share, which a text over `MAX_PASTE_TEXT_CHARS` cannot
 *  take whole. An entry that arrived shortened says so. */
export function MirrorEntry() {
  const shell = useMobileShell();
  const { backend } = useBackend();
  const { t, locale } = useI18n();
  const now = useNow();
  const { desktop, id } = parseParam(shell.param);
  const found = useMirrorEntry(desktop, id);
  const [view, setView] = useState<View>("polished");

  if (found === undefined) return null;
  if (found === null)
    return (
      <div className="p-4">
        <EmptyState compact title={t("history.long.exportFailed.gone")}>
          {t("mobile.entry.goneBody")}
        </EmptyState>
      </div>
    );

  const { entry, shortened } = found;
  const processed = entry.processed;
  const shown: View = view === "processed" && processed === undefined ? "polished" : view;
  const text =
    shown === "raw" ? entry.raw_text : shown === "processed" ? (processed?.text ?? "") : entry.text;
  const tooLong = textChars(text) > MAX_PASTE_TEXT_CHARS;
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
  const views: { value: View; label: string }[] = [
    { value: "polished", label: t("history.view.polished") },
    { value: "raw", label: t("history.view.raw") },
    ...(processed === undefined
      ? []
      : [{ value: "processed" as const, label: t("history.view.processed") }]),
  ];

  return (
    <div className="flex flex-col gap-3 p-4" data-testid="phone-mirror-entry">
      <div className="flex items-center gap-2 text-[12px] text-fg-muted">
        <span>{`${dayLabel(entry.at_ms, now, locale)} ${shortClockLabel(entry.at_ms)}`}</span>
        <span>{outcome.text}</span>
      </div>
      {(entry.refined || processed !== undefined) && (
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
      {shortened && (
        <p
          role="note"
          className="rounded-6 bg-inset px-3 py-2 text-[12px] leading-5 text-fg-muted"
          data-testid="phone-mirror-entry-shortened">
          {t("mirror.shortened")}
        </p>
      )}
      <p
        className="rounded-10 bg-surface p-4 text-[15px] leading-7 whitespace-pre-wrap break-words text-fg select-text hairline"
        data-testid="phone-mirror-entry-text"
        data-user-text>
        {text}
      </p>
      <div className="flex flex-wrap gap-2">
        <Button size="sm" icon="copy" disabled={tooLong} onClick={copy}>
          {t("mobile.recent.copy")}
        </Button>
        <Button
          size="sm"
          icon="share"
          disabled={tooLong}
          onClick={() => {
            backend.invoke("phone_share_text", { text }).catch(fail);
          }}>
          {t("mobile.recent.share")}
        </Button>
      </div>
      {tooLong && (
        <p className="text-[12px] text-fg-muted" data-testid="phone-mirror-entry-too-long">
          {t("mirror.tooLong")}
        </p>
      )}
      <p className="text-[11px] leading-4 text-fg-subtle">{t("mirror.readOnly")}</p>
      <dl className="flex flex-col divide-y divide-border rounded-10 bg-surface px-4 hairline">
        {entry.origin !== undefined && (
          <Fact label={t("history.detail.origin")}>
            <span data-user-text data-origin={entry.origin.kind}>
              {t(`history.origin.${entry.origin.kind}`, { device: entry.origin.device })}
            </span>
          </Fact>
        )}
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
        {entry.app !== undefined && (
          <Fact label={t("history.context.app")}>
            <span data-user-text>{entry.app.name}</span>
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
