import type { ReactNode } from "react";
import { cx } from "../cx";

export interface ReadoutProps {
  label: ReactNode;
  value: ReactNode;
  unit?: ReactNode;
  size?: "sm" | "md" | "lg";
  align?: "left" | "right";
  /** Muted value (e.g. `—` when history is off). */
  muted?: boolean;
  className?: string;
}

const VALUE_CLASS = { sm: "text-[12px]", md: "text-[13px]", lg: "text-[18px]" } as const;

/** Mono label-over-value readout used in dashboard panels and detail sheets. */
export function Readout({
  label,
  value,
  unit,
  size = "md",
  align = "left",
  muted = false,
  className,
}: ReadoutProps) {
  return (
    <div
      className={cx(
        "flex flex-col gap-0.5",
        align === "right" && "items-end text-right",
        className,
      )}>
      <span className="text-[11px] text-fg-subtle">{label}</span>
      <span
        className={cx(
          "mono leading-tight",
          VALUE_CLASS[size],
          muted ? "text-fg-subtle" : "text-fg",
        )}>
        {value}
        {unit !== undefined && <span className="ml-1 text-[11px] text-fg-muted">{unit}</span>}
      </span>
    </div>
  );
}
