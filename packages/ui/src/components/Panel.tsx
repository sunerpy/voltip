import type { ReactNode } from "react";
import { Card, type CardProps } from "./Card";
import { cx } from "../cx";

export interface PanelProps extends Omit<CardProps, "title"> {
  eyebrow: ReactNode;
  title?: ReactNode;
  /** Right side of the header: readouts, lamps, buttons. */
  right?: ReactNode;
  children: ReactNode;
  bodyClassName?: string;
}

/** Card with an eyebrow header row: `眉题 · 标题 ........ readout`. The eyebrow is localised copy
 *  (one language per locale, docs/frontend.md §6.4): pages pass `t(...)`, never a Latin caps label. */
export function Panel({
  eyebrow,
  title,
  right,
  children,
  bodyClassName,
  className,
  ...rest
}: PanelProps) {
  return (
    <Card className={cx("flex flex-col", className)} {...rest}>
      {/* A right side that does not fit next to the eyebrow moves to its own line; the eyebrow
          itself never breaks (it read "MICROPHONE / INPUT" at 1152 px). */}
      <header className="mb-3 flex min-h-5 flex-wrap items-center justify-between gap-x-3 gap-y-1.5">
        <div className="flex min-w-0 items-baseline gap-2">
          <span className="eyebrow whitespace-nowrap">{eyebrow}</span>
          {title !== undefined && (
            <>
              <span className="eyebrow">·</span>
              <span className="text-[12px] text-fg-muted">{title}</span>
            </>
          )}
        </div>
        {right !== undefined && (
          <div className="ml-auto flex min-w-0 flex-wrap items-center justify-end gap-2 text-[11px]">
            {right}
          </div>
        )}
      </header>
      <div className={cx("flex-1", bodyClassName)}>{children}</div>
    </Card>
  );
}
