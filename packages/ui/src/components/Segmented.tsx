import { cx } from "../cx";

export interface SegmentedOption<V extends string> {
  value: V;
  label: string;
  disabled?: boolean;
  /** Tooltip explaining why a segment is unavailable. */
  reason?: string;
}

export interface SegmentedProps<V extends string> {
  options: readonly SegmentedOption<V>[];
  value: V;
  onChange: (value: V) => void;
  /** `soft`: selected = surface + hairline (本地 | 云端). `ink`: selected = ink fill (全部 · 已生效). */
  variant?: "soft" | "ink";
  size?: "sm" | "md";
  mono?: boolean;
  label?: string;
  className?: string;
}

export function Segmented<V extends string>({
  options,
  value,
  onChange,
  variant = "soft",
  size = "md",
  mono = false,
  label,
  className,
}: SegmentedProps<V>) {
  return (
    <div
      role="radiogroup"
      aria-label={label}
      className={cx(
        "inline-flex items-center rounded-6 bg-inset p-0.5 hairline",
        size === "sm" ? "h-7" : "h-8",
        className,
      )}>
      {options.map((opt) => {
        const selected = opt.value === value;
        return (
          <button
            key={opt.value}
            type="button"
            role="radio"
            aria-checked={selected}
            disabled={opt.disabled}
            title={opt.reason}
            onClick={() => {
              onChange(opt.value);
            }}
            className={cx(
              "h-full rounded-[5px] px-3 whitespace-nowrap transition-colors",
              size === "sm" ? "text-[11px]" : "text-[12px]",
              mono && "mono",
              selected &&
                variant === "soft" &&
                "bg-surface text-fg shadow-[0_0_0_1px_var(--border)]",
              selected && variant === "ink" && "bg-primary text-primary-fg",
              !selected && "text-fg-muted hover:text-fg",
              opt.disabled && "cursor-not-allowed opacity-40 hover:text-fg-muted",
            )}>
            {opt.label}
          </button>
        );
      })}
    </div>
  );
}
