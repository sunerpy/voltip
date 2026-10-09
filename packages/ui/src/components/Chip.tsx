import type { ReactNode } from "react";
import { cx } from "../cx";
import { Lamp, type LampTone } from "./Lamp";

export interface ChipProps {
  children: ReactNode;
  lamp?: LampTone;
  /** Mono count shown after the label, e.g. `×14`. */
  count?: string;
  active?: boolean;
  disabled?: boolean;
  onClick?: () => void;
  title?: string;
  className?: string;
  round?: boolean;
}

/** Hairline pill (h 28). Renders a button when clickable, a span otherwise. */
export function Chip({
  children,
  lamp,
  count,
  active = false,
  disabled = false,
  onClick,
  title,
  className,
  round = false,
}: ChipProps) {
  const classes = cx(
    "inline-flex h-7 items-center gap-1.5 bg-surface px-2.5 text-[12px] whitespace-nowrap hairline",
    round ? "rounded-pill" : "rounded-6",
    active && "border-primary bg-primary text-primary-fg",
    onClick && !disabled && "cursor-pointer hover:border-fg-subtle",
    disabled && "opacity-50",
    className,
  );
  const body = (
    <>
      {lamp && <Lamp tone={lamp} size={6} />}
      <span>{children}</span>
      {count !== undefined && <span className="mono text-[11px] text-fg-muted">{count}</span>}
    </>
  );
  if (onClick) {
    return (
      <button
        type="button"
        className={classes}
        onClick={onClick}
        disabled={disabled}
        title={title}
        aria-pressed={active}>
        {body}
      </button>
    );
  }
  return (
    <span className={classes} title={title}>
      {body}
    </span>
  );
}
