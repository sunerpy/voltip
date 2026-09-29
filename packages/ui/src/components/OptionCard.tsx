import type { KeyboardEvent, MouseEvent, ReactNode } from "react";
import { cx } from "../cx";
import { Icon, type IconName } from "./Icon";

export interface OptionCardProps {
  /** Accent ring + soft tint; reported as `aria-selected` when the card is selectable. */
  selected?: boolean;
  /** Makes the card an interactive `option` (click / Enter / Space). Wrap the group in a
   *  `role="listbox"` at the call site. Without it the card is a plain `article`. */
  onSelect?: () => void;
  disabled?: boolean;
  /** 32 px icon tile at the left of the header. */
  icon?: IconName;
  title: ReactNode;
  /** Mono line under the title (id, endpoint…). */
  subtitle?: ReactNode;
  /** Right side of the header: status badges. */
  badge?: ReactNode;
  children?: ReactNode;
  /** Actions row under a hairline; clicks and keys inside never select the card. */
  footer?: ReactNode;
  className?: string;
  "aria-label"?: string;
}

function stop(e: MouseEvent | KeyboardEvent) {
  e.stopPropagation();
}

/** Selectable card (hairline, radius 10, no shadow): header with icon tile · title · badge, free
 *  body, optional footer. Hover lifts the border, selection is the accent ring; green never fills. */
export function OptionCard({
  selected = false,
  onSelect,
  disabled = false,
  icon,
  title,
  subtitle,
  badge,
  children,
  footer,
  className,
  "aria-label": ariaLabel,
}: OptionCardProps) {
  const selectable = onSelect !== undefined;
  // `ring-inset` is unusable here: the palette has an `inset` colour token, so Tailwind also emits
  // `ring-inset` as a ring *colour* (var(--inset)) that beats `ring-accent`. The inset ring
  // utilities carry their own colour variable.
  const classes = cx(
    "flex flex-col gap-3 rounded-10 p-4 text-left hairline transition-colors",
    selected ? "bg-accent-soft/40 inset-ring-2 inset-ring-accent" : "bg-surface",
    selectable && !disabled && "cursor-pointer",
    selectable && !disabled && !selected && "hover:border-border-strong",
    disabled && "cursor-not-allowed opacity-60",
    className,
  );
  const body = (
    <>
      <header className="flex items-start gap-3">
        {icon && (
          <span
            className={cx(
              "flex h-8 w-8 shrink-0 items-center justify-center rounded-6",
              selected ? "bg-accent-soft text-accent-text" : "bg-inset text-fg-muted",
            )}>
            <Icon name={icon} size={16} />
          </span>
        )}
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          {/* One line; cut only when the card is too narrow, the whole title on hover. */}
          <div
            className="truncate text-[14px] leading-5 font-semibold text-fg"
            title={typeof title === "string" ? title : undefined}>
            {title}
          </div>
          {subtitle !== undefined && (
            <div
              className="mono truncate text-[11px] leading-4 text-fg-muted"
              title={typeof subtitle === "string" ? subtitle : undefined}>
              {subtitle}
            </div>
          )}
        </div>
        {badge !== undefined && (
          <div className="flex shrink-0 flex-wrap items-center justify-end gap-1">{badge}</div>
        )}
      </header>
      {children !== undefined && (
        <div className="flex flex-col gap-2 text-[13px] leading-5 text-fg">{children}</div>
      )}
      {footer !== undefined && (
        <div
          className="mt-auto flex items-center gap-2 border-t border-border pt-3"
          onClick={stop}
          onKeyDown={stop}>
          {footer}
        </div>
      )}
    </>
  );

  if (!selectable) {
    return (
      <article aria-label={ariaLabel} data-selected={selected || undefined} className={classes}>
        {body}
      </article>
    );
  }

  const select = () => {
    if (!disabled) onSelect();
  };
  return (
    <div
      role="option"
      aria-selected={selected}
      aria-disabled={disabled || undefined}
      aria-label={ariaLabel}
      data-selected={selected || undefined}
      tabIndex={disabled ? -1 : 0}
      onClick={select}
      onKeyDown={(e) => {
        if (e.target !== e.currentTarget) return;
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          select();
        }
      }}
      className={classes}>
      {body}
    </div>
  );
}
