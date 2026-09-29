import {
  Button,
  Card,
  Eyebrow,
  LiveCaption,
  PILL_STATES,
  Pill,
  type PillState,
  Toast,
  pillCaption,
  useBackend,
  useI18n,
} from "@voltip/ui";
import { useEffect, useState } from "react";
import { useRouter } from "../app/router";
import { copyWithToast, useShell } from "../app/shell-context";
import { isPillState } from "./Overlay";

// `pnpm dev` only: `App` imports this module behind `import.meta.env.DEV`, so a release build
// carries neither the spec sheet nor its sample values (`scripts/check-web-bundle.sh`).

export interface OverlaySheetProps {
  /** A pill state (`/overlay?state=listening`) draws that one pill over sample values, chromeless,
   *  as the pill window would; anything else draws the whole sheet. */
  state?: string;
}

/** Failure causes: dictionary keys of the cause and its destination, plus the code. */
const FAILURE_CAUSES = [
  ["uipi", "uipi_denied"],
  ["secure", "secure_field"],
  ["focus", "focus_lost"],
  ["paste", "paste_timeout"],
  ["ax", "accessibility_denied"],
  ["wayland", "wayland_tool_missing"],
] as const;

/** Deterministic pseudo-waveform so tests and screenshots are stable. */
export function fakeLevels(tick: number, bars = 40): number[] {
  return Array.from({ length: bars }, (_, i) => {
    const phase = (i + tick) * 0.7;
    return 0.25 + 0.35 * Math.abs(Math.sin(phase)) + 0.2 * Math.abs(Math.sin(phase * 2.3));
  });
}

function useTicker(active: boolean, intervalMs = 85): number {
  const [tick, setTick] = useState(0);
  useEffect(() => {
    if (!active) return;
    const id = setInterval(() => {
      setTick((t) => t + 1);
    }, intervalMs);
    return () => {
      clearInterval(id);
    };
  }, [active, intervalMs]);
  return tick;
}

const READOUTS: Partial<Record<PillState, string>> = {
  listening: "00:07",
  locked: "01:24",
  processing: "0.9 s",
};

/** The pill spec sheet: the eight pill states, live caption, toasts, fallback card and failure causes. A
 *  component showcase over sample values: copying and navigation are real, the locked pill's
 *  结束收音 is the real `dictation_stop`, and the demo toasts' undo buttons do nothing. */
export default function OverlaySheet({ state }: OverlaySheetProps) {
  const shell = useShell();
  const { backend } = useBackend();
  const { navigate } = useRouter();
  const { t } = useI18n();
  const single = state !== undefined && isPillState(state) ? state : undefined;
  const tick = useTicker(single !== undefined);
  const levels = fakeLevels(tick);

  const fallbackText = t("overlay.fallbackText");
  const sampleLabels: Partial<Record<PillState, string>> = {
    processing: t("overlay.label.processing"),
    inserted: t("overlay.label.inserted"),
    error: t("overlay.label.error"),
  };
  const copyFallback = () => {
    void copyWithToast(shell, fallbackText, t("overlay.copied", { n: fallbackText.length }));
  };

  if (single !== undefined) {
    return (
      <div
        className="flex h-full items-start justify-center bg-transparent pt-2"
        data-testid="overlay-window">
        <Pill
          state={single}
          levels={levels}
          readout={READOUTS[single]}
          label={sampleLabels[single]}
          via={single === "inserted" ? "VS Code" : undefined}
          mode={single === "processing" ? t("overlay.live.refineTag") : undefined}
          onCopy={copyFallback}
        />
      </div>
    );
  }

  return (
    <div
      className="mx-auto flex w-full max-w-[1440px] flex-col gap-4 p-6"
      data-testid="page-overlay">
      <Eyebrow>{t("overlay.heading")}</Eyebrow>
      {/* The 3×3 state grid: two columns below `lg`, three fluid columns above. The pills
          inside keep their real overlay widths (52 / 260–340 / 420), the tiles stretch. */}
      <div className="grid grid-cols-2 gap-4 lg:grid-cols-3">
        {PILL_STATES.map((s) => (
          <div key={s} className="flex min-w-0 flex-col gap-1.5">
            <div className="flex h-[100px] flex-col items-center justify-between rounded-14 bg-desktop px-3 pt-3 pb-4">
              {/* The target application's text field the pill floats over (the card anatomy of the design). */}
              <span
                className="hairline flex h-6 w-full max-w-[240px] items-center rounded-6 bg-surface px-2"
                aria-hidden>
                <span className="h-3 w-px bg-fg-muted" />
              </span>
              <Pill
                state={s}
                levels={levels}
                readout={READOUTS[s]}
                label={sampleLabels[s]}
                via={s === "inserted" ? "VS Code" : undefined}
                mode={s === "processing" ? t("overlay.live.refineTag") : undefined}
                onCopy={copyFallback}
                onStop={() => {
                  void backend.invoke("dictation_stop");
                }}
              />
            </div>
            <span className="mono text-[11px] text-fg-subtle">{pillCaption(s, t)}</span>
          </div>
        ))}
        <div className="flex min-w-0 flex-col gap-1.5">
          <div className="flex h-[100px] items-center justify-center rounded-14 bg-desktop p-3">
            <LiveCaption
              committed={t("overlay.captionCommitted")}
              tail={t("overlay.captionTail")}
              tier="preview"
              elapsed="00:12"
              engine={t("overlay.captionEngine")}
              levels={levels}
            />
          </div>
          <span className="mono text-[11px] text-fg-subtle">{t("overlay.expanded")}</span>
        </div>
      </div>

      <Eyebrow>{t("overlay.anatomy")}</Eyebrow>
      <p className="mono text-[11px] leading-5 text-fg-muted">{t("overlay.anatomyBody")}</p>

      {/* bottom band: TOAST stack and FALLBACK card share the left column (320–380 px, the
          toast's own width), FAILURE CAUSES takes the rest so its three columns stay single-line;
          below `lg` the two stack. */}
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-[minmax(320px,380px)_minmax(0,1fr)]">
        <div className="flex flex-col gap-4">
          <div className="flex flex-col gap-2">
            <Eyebrow>{t("overlay.toastTitle")}</Eyebrow>
            <Toast
              toast={{
                id: "t1",
                message: t("overlay.toast.cancelled"),
                duration: 5000,
                action: { label: t("overlay.toast.undo"), keys: "Z", onClick: () => undefined },
              }}
              onDismiss={() => undefined}
            />
            <Toast
              toast={{
                id: "t2",
                message: t("overlay.toast.theme"),
                duration: 5000,
                action: { label: t("overlay.toast.undo"), onClick: () => undefined },
              }}
              onDismiss={() => undefined}
            />
            <Toast
              toast={{ id: "t3", message: t("overlay.toast.copied"), duration: 2000 }}
              onDismiss={() => undefined}
            />
          </div>
          <Card radius={14} className="flex flex-col gap-2">
            <div className="eyebrow">{t("overlay.fallback.title")}</div>
            <div className="flex items-center gap-2 text-[14px] font-semibold text-fg">
              <span className="text-danger">⚠</span> {t("overlay.fallback.heading")}
            </div>
            <p className="text-[12px] text-fg-muted">{t("overlay.fallback.body")}</p>
            <div className="mono truncate rounded-6 bg-inset px-2 py-1.5 text-[12px] text-fg">
              {fallbackText}
            </div>
            <div className="flex items-center justify-between">
              <span className="mono text-[10px] text-fg-subtle">
                {t("overlay.fallback.stored")}
              </span>
              <div className="flex gap-2">
                <Button size="sm" variant="primary" onClick={copyFallback}>
                  {t("overlay.fallback.copy")}
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => {
                    navigate({ name: "history" });
                  }}>
                  {t("overlay.fallback.openHistory")}
                </Button>
              </div>
            </div>
          </Card>
        </div>
        <Card padding="sm" className="flex flex-col gap-1 self-start">
          <div className="eyebrow mb-1">{t("overlay.failures.title")}</div>
          <table className="w-full text-[12px]">
            <thead>
              <tr className="eyebrow text-left">
                <th className="pb-1 font-normal">{t("overlay.failures.cause")}</th>
                <th className="pb-1 font-normal">{t("overlay.failures.code")}</th>
                <th className="pb-1 text-right font-normal">{t("overlay.failures.dest")}</th>
              </tr>
            </thead>
            <tbody>
              {FAILURE_CAUSES.map(([key, code]) => (
                // one line per cause; the free-text column truncates, code and destination never wrap.
                <tr key={code} className="border-t border-border whitespace-nowrap">
                  <td className="w-full py-1 text-fg">{t(`overlay.causes.${key}`)}</td>
                  <td className="mono py-1 pr-3 text-fg-muted">{code}</td>
                  <td
                    className={`mono py-1 text-right ${key === "secure" ? "text-danger" : "text-fg-muted"}`}>
                    {t(`overlay.causes.${key}Dest`)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </Card>
      </div>
    </div>
  );
}
