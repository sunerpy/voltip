import { type ReactNode, useRef } from "react";
import { cx } from "../cx";
import { SEGMENT_ITEM, SEGMENT_SOFT_SELECTED, SEGMENT_TRACK } from "./Segmented";

export interface TabsItem<V extends string> {
  value: V;
  label: ReactNode;
  /** The tab's element id: its panel names it in `aria-labelledby`. */
  id: string;
  /** The id of the panel the tab shows. */
  panelId: string;
}

export interface TabsProps<V extends string> {
  items: readonly TabsItem<V>[];
  value: V;
  onChange: (value: V) => void;
  /** The list's accessible name. */
  label: string;
  size?: "sm" | "md";
  className?: string;
}

/** A switch between the views of a page, drawn as `Segmented`'s soft track, with tab semantics:
 *  only the selected tab is in the Tab order, and the arrow keys (Home and End too) move to a view
 *  and show it. The page renders the selected view as a `role="tabpanel"` with the item's
 *  `panelId`, labelled by its `id`; the selected tab alone names it in `aria-controls`, so no tab
 *  points at a panel that is not there. Desktop only: it has no touch presentation. */
export function Tabs<V extends string>({
  items,
  value,
  onChange,
  label,
  size = "md",
  className,
}: TabsProps<V>) {
  const buttons = useRef(new Map<V, HTMLButtonElement>());
  const show = (next: V) => {
    onChange(next);
    buttons.current.get(next)?.focus();
  };
  return (
    <div
      role="tablist"
      aria-label={label}
      className={cx(SEGMENT_TRACK, size === "sm" ? "h-7" : "h-8", className)}
      onKeyDown={(e) => {
        const at = items.findIndex((item) => item.value === value);
        const step = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
        const next =
          e.key === "Home"
            ? items[0]
            : e.key === "End"
              ? items.at(-1)
              : step === 0
                ? undefined
                : items[(at + step + items.length) % items.length];
        if (next === undefined) return;
        e.preventDefault();
        show(next.value);
      }}>
      {items.map((item) => {
        const selected = item.value === value;
        return (
          <button
            key={item.value}
            ref={(el) => {
              if (el === null) buttons.current.delete(item.value);
              else buttons.current.set(item.value, el);
            }}
            id={item.id}
            type="button"
            role="tab"
            aria-selected={selected}
            aria-controls={selected ? item.panelId : undefined}
            tabIndex={selected ? 0 : -1}
            onClick={() => {
              onChange(item.value);
            }}
            className={cx(
              SEGMENT_ITEM,
              size === "sm" ? "text-[11px]" : "text-[12px]",
              selected ? SEGMENT_SOFT_SELECTED : "text-fg-muted hover:text-fg",
            )}>
            {item.label}
          </button>
        );
      })}
    </div>
  );
}
