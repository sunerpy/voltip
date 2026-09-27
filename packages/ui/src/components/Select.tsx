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

/** Native `<select>` with the design's chrome: hairline, radius 6, chevron, optional mono value. */
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
  return (
    <div className={cx("flex flex-col gap-1", className)}>
      {label && (
        <label htmlFor={selectId} className="text-[12px] text-fg-muted">
          {label}
        </label>
      )}
      <div className="relative">
        <select
          id={selectId}
          value={value}
          onChange={(e) => {
            const next = options.find((o) => o.value === e.target.value);
            if (next) onChange(next.value);
          }}
          className={cx(
            "w-full appearance-none rounded-6 bg-surface pr-7 pl-2.5 hairline outline-none focus:border-fg",
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
