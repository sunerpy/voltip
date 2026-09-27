import { cx } from "../cx";
import { useT } from "../i18n/I18nProvider";
import { clamp01 } from "./LedMeter";

export type WaveformState = "live" | "frozen" | "collapsed" | "idle";
export type WaveformTone = "accent" | "danger" | "ok";

export interface WaveformProps {
  /** 0..1 per bar, oldest first. */
  levels: readonly number[];
  state?: WaveformState;
  tone?: WaveformTone;
  /** Bar height in px at level 1. */
  height?: number;
  bars?: number;
  label?: string;
  className?: string;
}

const TONE_CLASS: Record<WaveformTone, string> = {
  accent: "bg-accent",
  danger: "bg-danger",
  ok: "bg-ok",
};

/** Dynamic-Island style waveform: 2 px bars, 2 px gaps, round caps, mirrored around the midline.
 *  The newest 45 % of bars use the state colour; the tail is `wave` at 55 % opacity. */
export function Waveform({
  levels,
  state = "live",
  tone = "accent",
  height = 16,
  bars = 40,
  label,
  className,
}: WaveformProps) {
  const t = useT();
  const visible = levels.slice(-bars);
  const padded =
    visible.length < bars
      ? [...Array.from({ length: bars - visible.length }, () => 0), ...visible]
      : visible;
  const recentFrom = Math.floor(bars * 0.55);
  return (
    <div
      role="img"
      aria-label={label ?? t("ui.a11y.waveform")}
      data-state={state}
      className={cx(
        "flex items-center gap-[2px]",
        state === "frozen" && "[animation:vt-pulse_1.4s_ease-in-out_infinite]",
        className,
      )}
      style={{ height }}>
      {padded.map((raw, i) => {
        const level = state === "collapsed" ? 0 : clamp01(raw);
        const h = state === "collapsed" ? 2 : Math.max(2, Math.round(level * height));
        const recent = i >= recentFrom;
        return (
          <span
            key={i}
            className={cx(
              "w-[2px] shrink-0 rounded-full",
              state === "frozen" || state === "idle"
                ? "bg-led-off"
                : recent
                  ? TONE_CLASS[tone]
                  : "bg-wave opacity-55",
            )}
            style={{ height: h }}
          />
        );
      })}
    </div>
  );
}
