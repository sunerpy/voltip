import type { ReactNode } from "react";
import { cx } from "../cx";
import { Lamp } from "./Lamp";

export type BadgeTone = "neutral" | "ok" | "accent" | "danger" | "warn" | "info" | "ink";

export interface BadgeProps {
  tone?: BadgeTone;
  children: ReactNode;
  mono?: boolean;
  className?: string;
  /** Native tooltip, for a badge that shortens what it shows. */
  title?: string;
}

const TONE_CLASS: Record<BadgeTone, string> = {
  neutral: "bg-inset text-fg-muted hairline",
  ok: "bg-surface text-fg hairline",
  accent: "bg-accent-soft text-accent-text",
  danger: "bg-danger-soft text-danger",
  warn: "bg-warning-soft text-warning",
  info: "bg-info-soft text-info",
  ink: "bg-primary text-primary-fg",
};

/** Small status label. `ok` is neutral surface + green dot, never a green fill. */
export function Badge({ tone = "neutral", children, mono = false, className, title }: BadgeProps) {
  return (
    <span
      data-tone={tone}
      title={title}
      className={cx(
        "inline-flex h-5 items-center gap-1.5 rounded-6 px-1.5 text-[11px] leading-none whitespace-nowrap",
        mono && "mono",
        TONE_CLASS[tone],
        className,
      )}>
      {tone === "ok" && <Lamp tone="ok" size={6} />}
      {children}
    </span>
  );
}
