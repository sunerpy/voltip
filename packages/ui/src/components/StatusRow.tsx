import type { ReactNode } from "react";
import { cx } from "../cx";

export interface StatusRowProps {
  label: ReactNode;
  help?: ReactNode;
  /** Right column: the control or readout. */
  children?: ReactNode;
  /** Extra line under the control (mono readout such as `prefers-color-scheme: light`). */
  note?: ReactNode;
  className?: string;
  "data-testid"?: string;
}

/** Settings form row: label + help on the left, taking the width the control leaves (no fixed cap:
 *  help that fits the row stays on one line, user feedback 2026-09-29), control right-aligned,
 *  hairline below. */
export function StatusRow({ label, help, children, note, className, ...rest }: StatusRowProps) {
  return (
    <div
      data-testid={rest["data-testid"]}
      className={cx(
        "flex min-h-[52px] items-center justify-between gap-4 border-b border-border py-3 last:border-b-0",
        className,
      )}>
      <div className="min-w-0 flex-1">
        <div className="text-[14px] text-fg">{label}</div>
        {help !== undefined && (
          <div className="mt-0.5 text-[12px] leading-4 text-fg-muted">{help}</div>
        )}
      </div>
      {/* The control keeps its size up to 60 % of the row; long readouts (commands, paths)
          truncate inside it instead of squeezing the label away. */}
      <div className="flex min-w-0 max-w-[60%] flex-col items-end gap-1">
        {children}
        {note !== undefined && <span className="mono text-[11px] text-fg-subtle">{note}</span>}
      </div>
    </div>
  );
}
