import type { ReactNode } from "react";
import { cx } from "../cx";

export interface ToggleProps {
  checked: boolean;
  onChange: (next: boolean) => void;
  label?: ReactNode;
  /** Mono readout after the label (e.g. `PORT 8756`). */
  readout?: ReactNode;
  disabled?: boolean;
  id?: string;
  className?: string;
  /** Accessible name when there is no visible label (table cells). */
  ariaLabel?: string;
}

/** 32×18 switch. On = ink track, off = hairline track; the knob is the surface colour. */
export function Toggle({
  checked,
  onChange,
  label,
  readout,
  disabled = false,
  id,
  className,
  ariaLabel,
}: ToggleProps) {
  return (
    <label
      className={cx(
        "inline-flex items-center gap-2 text-[13px]",
        disabled && "opacity-50",
        className,
      )}>
      <button
        id={id}
        type="button"
        role="switch"
        aria-checked={checked}
        aria-label={ariaLabel}
        disabled={disabled}
        onClick={() => {
          onChange(!checked);
        }}
        className={cx(
          "relative inline-flex h-[18px] w-8 shrink-0 items-center rounded-pill border transition-colors",
          checked ? "border-primary bg-primary" : "border-border-strong bg-surface",
          disabled
            ? "cursor-not-allowed"
            : checked
              ? "cursor-pointer hover:opacity-90"
              : "cursor-pointer hover:border-fg-subtle",
        )}>
        <span
          className={cx(
            "absolute top-[2px] h-3 w-3 rounded-full transition-transform",
            checked ? "translate-x-[16px] bg-primary-fg" : "translate-x-[2px] bg-fg-subtle",
          )}
        />
      </button>
      {label !== undefined && <span>{label}</span>}
      {readout !== undefined && <span className="mono text-[11px] text-fg-muted">{readout}</span>}
    </label>
  );
}
