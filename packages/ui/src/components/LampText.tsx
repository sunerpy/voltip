import type { ReactNode } from "react";
import { cx } from "../cx";
import { Lamp, type LampTone } from "./Lamp";

export interface LampTextProps {
  tone: LampTone;
  children: ReactNode;
  /** Mono readout appended after the label. */
  readout?: ReactNode;
  mono?: boolean;
  pulse?: boolean;
  size?: "sm" | "md";
  className?: string;
}

/** `● 运行中 · 127.0.0.1:47823` — the design's standard status readout. */
export function LampText({
  tone,
  children,
  readout,
  mono = false,
  pulse = false,
  size = "md",
  className,
}: LampTextProps) {
  return (
    <span
      className={cx(
        "inline-flex items-center gap-1.5 whitespace-nowrap",
        size === "sm" ? "text-[11px]" : "text-[12px]",
        mono && "mono",
        className,
      )}>
      <Lamp tone={tone} size={size === "sm" ? 6 : 8} pulse={pulse} />
      <span className="text-fg">{children}</span>
      {readout !== undefined && <span className="mono text-fg-muted">{readout}</span>}
    </span>
  );
}
