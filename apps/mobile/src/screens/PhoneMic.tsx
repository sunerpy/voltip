import {
  type DeviceView,
  type LevelFrame,
  type PhoneTakeState,
  type TFunction,
  formatElapsed,
  phoneTakeFinal,
} from "@voltip/shared";
import { Card, LampText, LedMeter, useBackend, useI18n, useUiState } from "@voltip/ui";
import { type PointerEvent, useEffect, useRef, useState } from "react";
import { useMobileShell } from "../app/shell";

/** How often the listening timer redraws. */
const TICK_MS = 250;

/** The line under the button: where the take is. */
export function phoneTakeLine(state: PhoneTakeState, elapsedMs: number, t: TFunction): string {
  switch (state.state) {
    case "starting":
      return t("mobile.mic.starting");
    case "listening":
      return t("mobile.mic.listening", { elapsed: formatElapsed(elapsedMs) });
    case "processing":
      return t("mobile.mic.processing");
    case "done":
      return t(state.pasted ? "mobile.mic.donePasted" : "mobile.mic.doneClipboard", {
        text: state.text,
      });
    case "failed":
      return t(`mobile.mic.failed.${state.code}`, { message: state.message });
    case "cancelled":
      return t("mobile.mic.cancelled");
  }
}

function tone(state: PhoneTakeState): "ok" | "accent" | "danger" | "idle" {
  switch (state.state) {
    case "listening":
      return "ok";
    case "starting":
    case "processing":
      return "accent";
    case "failed":
      return "danger";
    default:
      return "idle";
  }
}

/** dBFS → 0…1 for the meter (−60 dBFS is silence, 0 dBFS full scale). */
export function levelFraction(dbfs: number): number {
  return Math.min(1, Math.max(0, (dbfs + 60) / 60));
}

/** The level of the phone's own take while it records (`Backend.meter`): the take's capture feeds
 *  it, so the meter never opens the microphone by itself. */
function useTakeLevel(active: boolean): LevelFrame | undefined {
  const { backend } = useBackend();
  const [frame, setFrame] = useState<LevelFrame | undefined>(undefined);
  useEffect(() => {
    if (!active) return;
    let alive = true;
    let stop: (() => void) | undefined;
    backend
      .meter(undefined, (f) => {
        if (alive) setFrame(f);
      })
      .then(
        (unsubscribe) => {
          if (alive) stop = unsubscribe;
          else unsubscribe();
        },
        // No meter is no reason to stop the take; the line below still follows it.
        () => undefined,
      );
    return () => {
      alive = false;
      stop?.();
      setFrame(undefined);
    };
  }, [backend, active]);
  return frame;
}

function useTicking(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    const id = setInterval(() => {
      setNow(Date.now());
    }, TICK_MS);
    return () => {
      clearInterval(id);
    };
  }, [active]);
  return now;
}

/** The phone as the desktop's microphone (docs/dictation.md §20): hold to talk, release to send,
 *  slide off the button before releasing to cancel. The phone streams its microphone to the chosen
 *  paired desktop, which recognises and inserts the text; the line below follows the desktop. */
export function PhoneMic({ desktops }: { desktops: readonly DeviceView[] }) {
  const { backend } = useBackend();
  const shell = useMobileShell();
  const { t } = useI18n();
  const { phone_take: take } = useUiState();
  const online = desktops.filter((d) => d.connection.state === "online");
  const [picked, setPicked] = useState<string | undefined>(undefined);
  const target = online.find((d) => d.device.public_key === picked) ?? online[0];
  const [held, setHeld] = useState(false);
  const [offButton, setOffButton] = useState(false);
  const button = useRef<HTMLButtonElement>(null);
  // The start's invoke, while the button is held: a release waits for it, so a quick tap still
  // stops the take it started (the core keeps the order: start, then stop).
  const started = useRef<Promise<boolean> | null>(null);
  const running = take !== undefined && !phoneTakeFinal(take.state);
  const listening = take?.state.state === "listening";
  const now = useTicking(listening);
  const level = useTakeLevel(listening);

  if (target === undefined) {
    return (
      <Card className="flex flex-col gap-2" data-testid="phone-mic">
        <span className="text-[15px] font-semibold text-fg">{t("mobile.mic.title")}</span>
        <p className="text-[12px] text-fg-muted">{t("mobile.mic.noDesktop")}</p>
      </Card>
    );
  }

  const start = () => {
    if (running || started.current !== null) return;
    setHeld(true);
    setOffButton(false);
    started.current = backend
      .invoke("phone_take_start", { publicKey: target.device.public_key })
      .then(
        () => true,
        (e: unknown) => {
          // Refused before the core saw it (Android: the microphone permission was denied).
          setHeld(false);
          setOffButton(false);
          shell.toast(
            t("mobile.toast.error", { message: e instanceof Error ? e.message : String(e) }),
            "danger",
          );
          return false;
        },
      );
  };
  const finish = (cancel: boolean) => {
    const pending = started.current;
    started.current = null;
    setHeld(false);
    setOffButton(false);
    if (pending === null) return;
    void pending.then((ok) => {
      if (ok) void backend.invoke(cancel ? "phone_take_cancel" : "phone_take_stop");
    });
  };
  const outside = (e: PointerEvent) => {
    const rect = button.current?.getBoundingClientRect();
    if (rect === undefined) return false;
    return (
      e.clientX < rect.left ||
      e.clientX > rect.right ||
      e.clientY < rect.top ||
      e.clientY > rect.bottom
    );
  };

  const label = !held
    ? t("mobile.mic.hold")
    : offButton
      ? t("mobile.mic.releaseCancel")
      : t("mobile.mic.release");
  return (
    <Card className="flex flex-col gap-3" data-testid="phone-mic">
      <div className="flex flex-col gap-1">
        <span className="text-[15px] font-semibold text-fg">{t("mobile.mic.title")}</span>
        <p className="text-[12px] text-fg-muted">{t("mobile.mic.body")}</p>
      </div>
      {online.length > 1 && (
        <label className="flex items-center justify-between gap-3 text-[12px] text-fg-muted">
          {t("mobile.mic.target")}
          <select
            className="rounded-6 bg-surface px-2 py-1 text-[13px] text-fg hairline"
            value={target.device.public_key}
            disabled={running}
            onChange={(e) => {
              setPicked(e.target.value);
            }}>
            {online.map((d) => (
              <option key={d.device.public_key} value={d.device.public_key}>
                {d.device.name}
              </option>
            ))}
          </select>
        </label>
      )}
      <button
        ref={button}
        type="button"
        aria-pressed={held}
        data-testid="phone-mic-hold"
        data-cancel={offButton || undefined}
        disabled={running && !held}
        className={`h-24 w-full touch-none select-none rounded-14 text-[17px] font-semibold transition-colors ${
          offButton
            ? "bg-danger-soft text-danger"
            : held
              ? "bg-accent text-accent-fg"
              : "bg-inset text-fg hairline"
        }`}
        onPointerDown={(e) => {
          e.currentTarget.setPointerCapture(e.pointerId);
          start();
        }}
        onPointerMove={(e) => {
          if (held) setOffButton(outside(e));
        }}
        onPointerUp={(e) => {
          finish(outside(e));
        }}
        onPointerCancel={() => {
          finish(true);
        }}
        onKeyDown={(e) => {
          if ((e.key === " " || e.key === "Enter") && !e.repeat) {
            e.preventDefault();
            start();
          }
        }}
        onKeyUp={(e) => {
          if (e.key === " " || e.key === "Enter") {
            e.preventDefault();
            finish(false);
          }
        }}>
        {label}
        <span className="mt-1 block text-[12px] font-normal opacity-80">{target.device.name}</span>
      </button>
      {take !== undefined && take.device === target.device.public_key && (
        <div data-testid="phone-mic-state" data-state={take.state.state}>
          <LampText tone={tone(take.state)} pulse={take.state.state === "listening"}>
            {phoneTakeLine(take.state, now - take.started_at, t)}
          </LampText>
          {take.opus === true && !phoneTakeFinal(take.state) && (
            <span
              className="mono mt-1 block text-[11px] text-fg-subtle"
              data-testid="phone-mic-codec">
              {t("mobile.mic.codecOpus")}
            </span>
          )}
          {take.state.state === "listening" && (
            <LedMeter
              className="mt-2"
              size="sm"
              segments={24}
              label={t("mobile.mic.level")}
              level={level === undefined ? 0 : levelFraction(level.rms_dbfs)}
              peak={level === undefined ? undefined : levelFraction(level.peak_dbfs)}
            />
          )}
        </div>
      )}
    </Card>
  );
}
