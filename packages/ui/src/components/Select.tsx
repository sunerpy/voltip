import { type SelectHTMLAttributes, useId } from "react";
import { cx } from "../cx";
import { Icon } from "./Icon";

export interface SelectOption<V extends string> {
  value: V;
  label: string;
  disabled?: boolean;
}

export interface SelectProps<V extends string> extends Omit<
  SelectHTMLAttributes<HTMLSelectElement>,
  "onChange" | "value" | "size"
> {
  options: readonly SelectOption<V>[];
  value: V;
  onChange: (value: V) => void;
  mono?: boolean;
  label?: string;
  size?: "sm" | "md";
}

/** Native `<select>` with the design's chrome: hairline, radius 6, chevron, optional mono value.
 *  It is never narrower than its longest option (user feedback 2026-09-29: callers' fixed widths
 *  clipped the chosen label): a hidden copy of every label shares the select's grid cell and sets
 *  the column's width. CSS `field-sizing` would do the same, but WebKitGTK and WKWebView do not
 *  all support it. Callers pass no width; a container narrower than the longest label still wins. */
export function Select<V extends string>({
  options,
  value,
  onChange,
  mono = false,
  label,
  size = "md",
  className,
  id,
  ...rest
}: SelectProps<V>) {
  const autoId = useId();
  const selectId = id ?? autoId;
  // What the options are (a user's device names, endonyms) holds for their hidden copy too, so the
  // language scans treat both alike.
  const markers = Object.fromEntries(
    Object.entries(rest).filter(([key]) => key === "data-user-text" || key === "data-endonyms"),
  );
  return (
    <div className={cx("flex flex-col gap-1", className)}>
      {label && (
        <label htmlFor={selectId} className="text-[12px] text-fg-muted">
          {label}
        </label>
      )}
      <div className="relative grid">
        <span
          aria-hidden
          data-select-sizer=""
          {...markers}
          className={cx(
            "invisible col-start-1 row-start-1 flex h-0 flex-col overflow-hidden border border-transparent pr-7 pl-2.5 whitespace-nowrap",
            size === "sm" ? "text-[12px]" : "text-[13px]",
            mono && "mono",
          )}>
          {options.map((o) => (
            <span key={o.value}>{o.label}</span>
          ))}
        </span>
        <select
          id={selectId}
          value={value}
          onChange={(e) => {
            const next = options.find((o) => o.value === e.target.value);
            if (next) onChange(next.value);
          }}
          className={cx(
            "col-start-1 row-start-1 w-full min-w-0 appearance-none rounded-6 bg-surface pr-7 pl-2.5 hairline outline-none transition-colors hover:border-fg-subtle focus:border-fg disabled:opacity-50 disabled:hover:border-border",
            size === "sm" ? "h-7 text-[12px]" : "h-8 text-[13px]",
            mono && "mono",
          )}
          {...rest}>
          {options.map((o) => (
            <option key={o.value} value={o.value} disabled={o.disabled}>
              {o.label}
            </option>
          ))}
        </select>
        <Icon
          name="chevronDown"
          size={14}
          className="pointer-events-none absolute top-1/2 right-2 -translate-y-1/2 text-fg-subtle"
        />
      </div>
    </div>
  );
}
