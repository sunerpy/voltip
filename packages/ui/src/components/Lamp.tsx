import { cx } from "../cx";

export type LampTone = "ok" | "danger" | "warn" | "accent" | "neutral" | "idle" | "off";

export interface LampProps {
  tone?: LampTone;
  /** Diameter in px (6 / 8 / 10). */
  size?: 6 | 8 | 10;
  pulse?: boolean;
  label?: string;
  className?: string;
}

const TONE_CLASS: Record<LampTone, string> = {
  ok: "bg-ok",
  danger: "bg-danger",
  warn: "bg-warning",
  accent: "bg-accent",
  neutral: "bg-fg",
  idle: "bg-transparent border-[1.5px] border-fg-subtle",
  off: "bg-border",
};

/** 8 px status dot. Green is reserved for this component (R-UI-4). */
export function Lamp({ tone = "ok", size = 8, pulse = false, label, className }: LampProps) {
  return (
    <span
      role={label ? "img" : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
      data-tone={tone}
      className={cx(
        "inline-block shrink-0 rounded-full",
        TONE_CLASS[tone],
        pulse && "[animation:vt-pulse_1.6s_ease-in-out_infinite]",
        className,
      )}
      style={{ width: size, height: size }}
    />
  );
}
