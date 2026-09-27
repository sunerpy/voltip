import { cx } from "../cx";
import { useT } from "../i18n/I18nProvider";

export interface LedMeterProps {
  /** 0..1 current level. */
  level: number;
  /** 0..1 peak hold marker; omitted when idle. */
  peak?: number;
  segments?: number;
  /** Fraction of the bar (from the top) that reads as clipping and lights danger. */
  clipFrom?: number;
  size?: "xs" | "sm" | "md";
  /** Grey out the whole meter (permission denied / no device). */
  disabled?: boolean;
  label?: string;
  className?: string;
}

const SEGMENT_SIZE = { xs: "h-2.5 w-[3px]", sm: "h-3 w-1.5", md: "h-3.5 w-2" } as const;
const SEGMENT_GAP = { xs: "gap-[2px]", sm: "gap-[2px]", md: "gap-[3px]" } as const;

export function clamp01(n: number): number {
  return Number.isFinite(n) ? Math.min(1, Math.max(0, n)) : 0;
}

/** Segmented LED level meter: lit segments accent, clip zone danger, unlit `ledOff`. */
export function LedMeter({
  level,
  peak,
  segments = 28,
  clipFrom = 1 - 4 / 28,
  size = "md",
  disabled = false,
  label,
  className,
}: LedMeterProps) {
  const t = useT();
  const ariaLabel = label ?? t("ui.a11y.level");
  const lit = Math.round(clamp01(level) * segments);
  const peakIndex =
    peak === undefined ? -1 : Math.min(segments - 1, Math.round(clamp01(peak) * segments) - 1);
  const clipIndex = Math.floor(clipFrom * segments);
  return (
    <div
      role="meter"
      aria-label={ariaLabel}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.round(clamp01(level) * 100)}
      className={cx("flex items-end", SEGMENT_GAP[size], disabled && "opacity-40", className)}>
      {Array.from({ length: segments }, (_, i) => {
        const isLit = i < lit;
        const isClip = i >= clipIndex;
        const isPeak = i === peakIndex;
        return (
          <span
            key={i}
            data-lit={isLit ? "true" : "false"}
            data-peak={isPeak ? "true" : undefined}
            className={cx(
              "rounded-[2px]",
              SEGMENT_SIZE[size],
              isLit ? (isClip ? "bg-danger" : "bg-accent") : "bg-led-off",
              isPeak && !isLit && "bg-led-on",
            )}
          />
        );
      })}
    </div>
  );
}
