import { Card, Icon, type IconName, Lamp, type LampTone, cx } from "@voltip/ui";
import type { ReactNode } from "react";

/** The phone's size for a `Button` of either size: 44 px tall (the smallest touch target) with
 *  14 px text. Each class is larger than the button's own, so it wins wherever it is added. */
export const TOUCH = "h-11 px-4 text-[14px]";

/** An `IconButton`'s touch target: the drawing keeps its size, the button grows to 44 × 44. */
export const TOUCH_ICON = "min-h-11 min-w-11";

/** A `Toggle`'s touch target: the switch keeps Codex's 32 × 19, and its label takes the taps
 *  within 12 px of it. */
export const TOUCH_TOGGLE = "-m-3 p-3";

/** A `Textarea` (on its wrapper): typed text at 15 px, as the phone's `Input size="lg"` takes it. */
export const TOUCH_TEXTAREA = "[&_textarea]:text-[15px] [&_textarea]:leading-6";

/** A screen: 16 px from the edges, its blocks 16 px apart. */
export const PAGE = "flex flex-col gap-4 p-4";

/** The sentence under a screen's title, before its first block. */
export function Lede({ children }: { children: ReactNode }) {
  return <p className="px-1 text-[13px] leading-5 text-fg-muted">{children}</p>;
}

/** A hairline card of rows that divide themselves (the settings lists). */
export function RowList({ children }: { children: ReactNode }) {
  return (
    <Card padding="none" className="overflow-hidden">
      <ul className="divide-y divide-border">{children}</ul>
    </Card>
  );
}

/** A row that opens a page, in the desktop's settings style: a 14 px medium label, a 12 px muted
 *  description, a chevron at the end; the whole row is the target. */
export function NavRow({
  icon,
  title,
  detail,
  onOpen,
  ...rest
}: {
  icon?: IconName;
  title: ReactNode;
  detail?: ReactNode;
  onOpen: () => void;
  "data-testid"?: string;
}) {
  return (
    <li>
      <button
        type="button"
        data-testid={rest["data-testid"]}
        onClick={onOpen}
        className="flex min-h-14 w-full items-center gap-3 px-4 py-3 text-left transition-colors hover:bg-inset active:bg-inset">
        {icon !== undefined && <Icon name={icon} size={18} className="shrink-0 text-fg-muted" />}
        <span className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span className="text-[14px] font-medium text-fg">{title}</span>
          {detail !== undefined && (
            <span className="truncate text-[12px] leading-4 text-fg-muted">{detail}</span>
          )}
        </span>
        <Icon name="chevronRight" size={16} className="shrink-0 text-fg-subtle" />
      </button>
    </li>
  );
}

/** Read-only facts in a hairline card: the label left, the value right, a hairline between. */
export function Facts({ children }: { children: ReactNode }) {
  return (
    <Card padding="none" className="px-4">
      <dl>{children}</dl>
    </Card>
  );
}

/** One fact of `Facts`: `<dt>` and `<dd>` side by side. */
export function Fact({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-start justify-between gap-4 border-b border-border py-3 last:border-b-0">
      <dt className="shrink-0 text-[13px] text-fg-muted">{label}</dt>
      <dd className="min-w-0 text-right text-[13px] break-words text-fg">{children}</dd>
    </div>
  );
}

/** A status line that may wrap (a take's result, an update's failure): the lamp beside the first
 *  line, 8 px with 13 px text or 6 px with 12 px (`sm`). `LampText` is the one-line form. */
export function StateLine({
  tone,
  pulse = false,
  size = "md",
  children,
  className,
}: {
  tone: LampTone;
  pulse?: boolean;
  size?: "sm" | "md";
  children: ReactNode;
  className?: string;
}) {
  return (
    <span
      className={cx(
        "flex items-start text-fg",
        size === "sm" ? "gap-1.5 text-[12px] leading-4" : "gap-2 text-[13px] leading-5",
        className,
      )}>
      <Lamp
        tone={tone}
        pulse={pulse}
        size={size === "sm" ? 6 : 8}
        className={size === "sm" ? "mt-[5px]" : "mt-1.5"}
      />
      <span className="min-w-0 break-words">{children}</span>
    </span>
  );
}
