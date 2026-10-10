import { type ReactNode, useId } from "react";
import { cx } from "../cx";

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
