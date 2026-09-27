import type { ReactNode } from "react";
import { cx } from "../cx";

export interface EyebrowProps {
  children: ReactNode;
  /** Optional right-aligned readout (mono). */
  right?: ReactNode;
  className?: string;
}

/** JetBrains Mono 11 px, 0.08 em tracking, subtle colour — the section label of every panel. */
export function Eyebrow({ children, right, className }: EyebrowProps) {
  return (
    <div className={cx("flex items-center justify-between gap-3", className)}>
      <span className="eyebrow">{children}</span>
      {right !== undefined && <span className="mono text-[11px] text-fg-muted">{right}</span>}
    </div>
  );
}
