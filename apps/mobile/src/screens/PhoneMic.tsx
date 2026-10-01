import {
  type DeviceView,
  type DictationPhase,
  type LevelFrame,
  type PhoneTakeState,
  type TFunction,
  coreMessageText,
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

/** The line under the button for a take the phone recognises itself (docs/dictation.md §20.7). */
export function localTakeLine(phase: DictationPhase, elapsedMs: number, t: TFunction): string {
  switch (phase.phase) {
    case "idle":
      return "";
    case "listening":
      return t("mobile.mic.listening", { elapsed: formatElapsed(elapsedMs) });
    case "processing":
      if (phase.stage === "transcribing") return t("mobile.mic.local.transcribing");
      if (phase.stage === "refining") return t("mobile.mic.local.refining");
      return t("mobile.mic.local.processing");
    case "done":
      return t("mobile.mic.local.copied", { text: phase.text });
    case "failed":
      return phase.code === "no_speech"
        ? t("mobile.mic.failed.no_speech")
        : t("mobile.mic.local.failed", { message: coreMessageText(phase.message) });
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

function localTone(phase: DictationPhase): "ok" | "accent" | "danger" | "idle" {
  switch (phase.phase) {
    case "listening":
      return "ok";
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

/** The phone's talk card (docs/dictation.md §20, §20.7): hold to talk, release to finish, slide
 *  off the button before releasing to cancel. With a paired computer online the take streams to
 *  it, and the computer recognises and inserts the text; with none online the phone recognises
 *  it itself through the built-in service and copies the result. A take keeps its route until it
 *  ends: a computer coming online or going away in the middle moves nothing. */
export function PhoneMic({ desktops }: { desktops: readonly DeviceView[] }) {
  const { phone_take: take, dictation } = useUiState();
  const online = desktops.filter((d) => d.connection.state === "online");
  const toComputer = take !== undefined && !phoneTakeFinal(take.state);
  const onPhone = dictation.phase.phase === "listening" || dictation.phase.phase === "processing";
  const route = toComputer ? "computer" : onPhone || online.length === 0 ? "phone" : "computer";
  return route === "computer" ? (
    <ComputerTalk desktops={desktops} online={online} />
  ) : (
    <PhoneTalk paired={desktops.length > 0} />
  );
}

/** The hold-to-talk button both routes share. `start` resolves whether the take started; a
 *  release waits for it, so a quick tap still stops the take it started (the core keeps the
 *  order: start, then stop). */
function HoldButton({
  busy,
  sublabel,
  releaseLabel,
  start,
  stop,
  cancel,
}: {
  busy: boolean;
  sublabel: string;
  releaseLabel: string;
  start: () => Promise<boolean>;
  stop: () => void;
  cancel: () => void;
}) {
  const { t } = useI18n();
  const [held, setHeld] = useState(false);
  const [offButton, setOffButton] = useState(false);
  const button = useRef<HTMLButtonElement>(null);
  const started = useRef<Promise<boolean> | null>(null);

  const begin = () => {
    if (busy || started.current !== null) return;
    setHeld(true);
    setOffButton(false);
    started.current = start().then((ok) => {
      if (!ok) {
        setHeld(false);
        setOffButton(false);
      }
      return ok;
    });
  };
  const finish = (cancelled: boolean) => {
    const pending = started.current;
    started.current = null;
    setHeld(false);
    setOffButton(false);
    if (pending === null) return;
    void pending.then((ok) => {
      if (ok) (cancelled ? cancel : stop)();
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
      : releaseLabel;
  return (
    <button
      ref={button}
      type="button"
      aria-pressed={held}
      data-testid="phone-mic-hold"
      data-cancel={offButton || undefined}
      disabled={busy && !held}
      className={`h-24 w-full touch-none select-none rounded-14 text-[17px] font-semibold transition-colors ${
        offButton
          ? "bg-danger-soft text-danger"
          : held
            ? "bg-accent text-accent-fg"
            : "bg-inset text-fg hairline"
      }`}
      onPointerDown={(e) => {
        e.currentTarget.setPointerCapture(e.pointerId);
        begin();
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
          begin();
        }
      }}
      onKeyUp={(e) => {
        if (e.key === " " || e.key === "Enter") {
          e.preventDefault();
          finish(false);
        }
      }}>
      {label}
      <span className="mt-1 block text-[12px] font-normal opacity-80" data-testid="phone-mic-route">
        {sublabel}
      </span>
    </button>
  );
}

/** A start the shell refused before the core saw it (Android: the microphone permission was
 *  denied): a toast, and the button lets go. */
function useStart(): (run: () => Promise<void>) => Promise<boolean> {
  const shell = useMobileShell();
  const { t } = useI18n();
  return (run) =>
    run().then(
      () => true,
      (e: unknown) => {
        shell.toast(
          t("mobile.toast.error", {
            message: coreMessageText(e instanceof Error ? e.message : String(e)),
          }),
          "danger",
        );
        return false;
      },
    );
}

/** The take streams to a paired computer, which recognises and inserts the text; the line below
 *  follows the computer. */
function ComputerTalk({
  desktops,
  online,
}: {
  desktops: readonly DeviceView[];
  online: readonly DeviceView[];
}) {
  const { backend } = useBackend();
  const { t } = useI18n();
  const { phone_take: take } = useUiState();
  const begin = useStart();
  const [picked, setPicked] = useState<string | undefined>(undefined);
  const running = take !== undefined && !phoneTakeFinal(take.state);
  // A running take stays with its computer even when that one went offline meanwhile.
  const target =
    (running ? desktops.find((d) => d.device.public_key === take.device) : undefined) ??
    online.find((d) => d.device.public_key === picked) ??
    online[0];
  const listening = take?.state.state === "listening";
  const now = useTicking(listening);
  const level = useTakeLevel(listening);
  if (target === undefined) return null;

  return (
    <Card className="flex flex-col gap-3" data-testid="phone-mic" data-route="computer">
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
      <HoldButton
        busy={running}
        sublabel={t("mobile.mic.toDesktop", { name: target.device.name })}
        releaseLabel={t("mobile.mic.release")}
        start={() =>
          begin(() => backend.invoke("phone_take_start", { publicKey: target.device.public_key }))
        }
        stop={() => void backend.invoke("phone_take_stop")}
        cancel={() => void backend.invoke("phone_take_cancel")}
      />
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

/** The phone recognises the take itself (docs/dictation.md §20.7): the built-in service
 *  transcribes and polishes it, and the result lands on the phone's clipboard. */
function PhoneTalk({ paired }: { paired: boolean }) {
  const { backend } = useBackend();
  const { t } = useI18n();
  const { dictation } = useUiState();
  const begin = useStart();
  const phase = dictation.phase;
  const running = phase.phase === "listening" || phase.phase === "processing";
  const listening = phase.phase === "listening";
  const now = useTicking(listening);
  const level = useTakeLevel(listening);
  const line = localTakeLine(phase, listening && phase.ready ? now - phase.started_at : 0, t);

  return (
    <Card className="flex flex-col gap-3" data-testid="phone-mic" data-route="phone">
      <div className="flex flex-col gap-1">
        <span className="text-[15px] font-semibold text-fg">{t("mobile.mic.title")}</span>
        <p className="text-[12px] text-fg-muted">{t("mobile.mic.localBody")}</p>
        {paired && (
          <p className="text-[12px] text-fg-muted" data-testid="phone-mic-offline">
            {t("mobile.mic.offline")}
          </p>
        )}
      </div>
      <HoldButton
        busy={running}
        sublabel={t("mobile.mic.onPhone")}
        releaseLabel={t("mobile.mic.releaseLocal")}
        start={() => begin(() => backend.invoke("dictation_start"))}
        stop={() => void backend.invoke("dictation_stop")}
        cancel={() => void backend.invoke("dictation_cancel")}
      />
      {line !== "" && (
        <div data-testid="phone-mic-state" data-state={phase.phase}>
          <LampText tone={localTone(phase)} pulse={listening}>
            {line}
          </LampText>
          {listening && (
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
