import { type ReactNode, useId } from "react";
import { cx } from "../cx";
import { Icon, type IconName } from "./Icon";

export interface SettingsPaneProps {
  /** Pane title (the group's name). */
  title: ReactNode;
  /** One sentence under the title. */
  lede?: ReactNode;
  /** Right side of the title row (a pane-wide action). */
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
  "data-testid"?: string;
}

/** One settings group: title + lede, then its sections 24 px apart. Every pane of the settings
 *  dialog uses it, so titles, spacing and section rhythm are the same everywhere. */
export function SettingsPane({
  title,
  lede,
  actions,
  children,
  className,
  ...rest
}: SettingsPaneProps) {
  return (
    <div className={cx("flex flex-col gap-6", className)} data-testid={rest["data-testid"]}>
      <header className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <h2 className="text-[18px] font-semibold text-fg">{title}</h2>
          {lede !== undefined && <p className="mt-1 text-[13px] leading-5 text-fg-muted">{lede}</p>}
        </div>
        {actions !== undefined && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
      </header>
      {children}
    </div>
  );
}

export interface SettingsSectionProps {
  /** Section label (rendered in the eyebrow style). */
  title: ReactNode;
  /** A sentence under the label. */
  description?: ReactNode;
  /** Right side of the label row: a status lamp, a count, a small action. */
  aside?: ReactNode;
  children?: ReactNode;
  className?: string;
  "data-testid"?: string;
  /** Extra attributes some tests read (`data-state`, `data-mode`, …). */
  data?: Readonly<Record<`data-${string}`, string | undefined>>;
}

/** A labelled block inside a pane: eyebrow label + aside, optional description, content 12 px below.
 *  The label is a real heading, so screen readers can jump between sections. */
export function SettingsSection({
  title,
  description,
  aside,
  children,
  className,
  data,
  ...rest
}: SettingsSectionProps) {
  const headingId = useId();
  return (
    <section
      aria-labelledby={headingId}
      className={cx("flex flex-col gap-3", className)}
      data-testid={rest["data-testid"]}
      {...data}>
      <div className="flex min-h-5 items-center justify-between gap-3">
        <h3 id={headingId} className="eyebrow">
          {title}
        </h3>
        {aside !== undefined && (
          <div className="flex min-w-0 items-center gap-2 text-[11px]">{aside}</div>
        )}
      </div>
      {description !== undefined && (
        <p className="text-[12px] leading-4 text-fg-muted">{description}</p>
      )}
      {children}
    </section>
  );
}

/** The hairline-topped list `StatusRow`s sit in. */
export function SettingsRows({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={cx("border-t border-border", className)}>{children}</div>;
}

export interface CardGridProps {
  children: ReactNode;
  /** Smallest card width; columns follow the pane width. */
  min?: number;
  role?: "list" | "listbox";
  "aria-label"?: string;
  className?: string;
}

/** Fluid card grid: as many columns as fit at `min` px, never a fixed panel width. */
export function CardGrid({ children, min = 300, role, className, ...rest }: CardGridProps) {
  return (
    <div
      role={role}
      aria-label={rest["aria-label"]}
      className={cx("grid gap-4", className)}
      style={{ gridTemplateColumns: `repeat(auto-fill, minmax(${min}px, 1fr))` }}>
      {children}
    </div>
  );
}

export interface DisclosureCardProps {
  /** Expanded. */
  open: boolean;
  onToggle: (open: boolean) => void;
  icon?: IconName;
  title: ReactNode;
  /** Mono line under the title. */
  subtitle?: ReactNode;
  /** Status badges next to the title (inside the toggle; no controls here). */
  badge?: ReactNode;
  /** Controls on the right of the header, outside the toggle button. */
  actions?: ReactNode;
  /** Accent ring (the provider in use). */
  selected?: boolean;
  /** The expanded body. */
  children?: ReactNode;
  className?: string;
  "aria-label"?: string;
  "data-testid"?: string;
}

/** A card whose header toggles a body: the provider cards of the engines pane. The header is one
 *  button (`aria-expanded` / `aria-controls`); controls that must stay reachable while collapsed go
 *  in `actions`, beside it, never inside it. */
export function DisclosureCard({
  open,
  onToggle,
  icon,
  title,
  subtitle,
  badge,
  actions,
  selected = false,
  children,
  className,
  ...rest
}: DisclosureCardProps) {
  const panelId = useId();
  const headerId = useId();
  return (
    <article
      aria-label={rest["aria-label"]}
      data-testid={rest["data-testid"]}
      data-open={open || undefined}
      data-selected={selected || undefined}
      className={cx(
        "flex flex-col rounded-10 hairline transition-colors",
        selected ? "bg-accent-soft/40 inset-ring-2 inset-ring-accent" : "bg-surface",
        className,
      )}>
      <div className="flex items-center gap-2 pr-3">
        <button
          type="button"
          id={headerId}
          aria-expanded={open}
          aria-controls={panelId}
          onClick={() => {
            onToggle(!open);
          }}
          className="flex min-w-0 flex-1 items-center gap-3 rounded-10 p-4 text-left outline-none transition-colors hover:bg-fg/5 focus-visible:bg-nav-active">
          {icon !== undefined && (
            <span
              className={cx(
                "flex h-8 w-8 shrink-0 items-center justify-center rounded-6",
                selected ? "bg-accent-soft text-accent-text" : "bg-inset text-fg-muted",
              )}>
              <Icon name={icon} size={16} />
            </span>
          )}
          <span className="flex min-w-0 flex-1 flex-col gap-0.5">
            <span className="truncate text-[14px] leading-5 font-semibold text-fg">{title}</span>
            {subtitle !== undefined && (
              <span className="mono truncate text-[11px] leading-4 text-fg-muted">{subtitle}</span>
            )}
          </span>
          {badge !== undefined && (
            <span className="flex shrink-0 flex-wrap items-center justify-end gap-1">{badge}</span>
          )}
          <Icon
            name={open ? "chevronUp" : "chevronDown"}
            size={16}
            className="shrink-0 text-fg-subtle"
          />
        </button>
        {actions !== undefined && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
      </div>
      <div
        id={panelId}
        role="region"
        aria-labelledby={headerId}
        hidden={!open}
        className="flex flex-col gap-4 border-t border-border p-4">
        {open ? children : null}
      </div>
    </article>
  );
}
