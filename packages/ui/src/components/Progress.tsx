import { cx } from "../cx";
import { useT } from "../i18n/I18nProvider";
import { clamp01 } from "./LedMeter";

export interface ProgressProps {
  /** 0..1; ignored when `indeterminate`. */
  value?: number;
  indeterminate?: boolean;
  size?: 2 | 4 | 6 | 8;
  tone?: "accent" | "ink" | "ok" | "danger";
  /** Render as N discrete segments (download rows) instead of a continuous bar. */
  segments?: number;
  label?: string;
  className?: string;
}

const TONE_CLASS = {
  accent: "bg-accent",
  ink: "bg-primary",
  ok: "bg-ok",
  danger: "bg-danger",
} as const;

export function Progress({
  value = 0,
  indeterminate = false,
  size = 4,
  tone = "accent",
  segments,
  label,
  className,
}: ProgressProps) {
  const t = useT();
  const pct = clamp01(value) * 100;
  const aria = {
    role: "progressbar" as const,
    "aria-label": label ?? t("ui.a11y.progress"),
    "aria-valuemin": 0,
    "aria-valuemax": 100,
    "aria-valuenow": indeterminate ? undefined : Math.round(pct),
  };
  if (segments !== undefined) {
    const lit = Math.round(clamp01(value) * segments);
    return (
      <div {...aria} className={cx("flex gap-[2px]", className)} style={{ height: size }}>
        {Array.from({ length: segments }, (_, i) => (
          <span
            key={i}
            className={cx("flex-1 rounded-[1px]", i < lit ? TONE_CLASS[tone] : "bg-led-off")}
          />
        ))}
      </div>
    );
  }
  return (
    <div
      {...aria}
      className={cx("relative overflow-hidden rounded-pill bg-led-off", className ?? "w-full")}
      style={{ height: size }}>
      <span
        className={cx(
          "absolute inset-y-0 left-0 rounded-pill",
          TONE_CLASS[tone],
          indeterminate && "w-1/3 [animation:vt-indeterminate_1.2s_ease-in-out_infinite]",
        )}
        style={indeterminate ? undefined : { width: `${pct}%` }}
      />
    </div>
  );
}
