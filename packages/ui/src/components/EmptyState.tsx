import type { ReactNode } from "react";
import { cx } from "../cx";
import { Icon, type IconName } from "./Icon";
import { Lamp } from "./Lamp";

export interface EmptyStateProps {
  title: ReactNode;
  children?: ReactNode;
  /** Mono facts line (`history.sqlite3 · 未创建 · 默认 500 条 / 30 d`). */
  mono?: ReactNode;
  icon?: IconName;
  actions?: ReactNode;
  compact?: boolean;
  className?: string;
}

export function EmptyState({
  title,
  children,
  mono,
  icon,
  actions,
  compact = false,
  className,
}: EmptyStateProps) {
  return (
    <div
      role="status"
      className={cx(
        "flex flex-col items-center justify-center text-center",
        compact ? "gap-2 py-6" : "gap-3 py-12",
        className,
      )}>
      {icon ? (
        <Icon name={icon} size={compact ? 20 : 24} className="text-fg-subtle" />
      ) : (
        <Lamp tone="off" size={10} />
      )}
      <div className={cx("font-medium text-fg", compact ? "text-[14px]" : "text-[18px]")}>
        {title}
      </div>
      {children !== undefined && (
        <div className="max-w-[640px] text-[13px] leading-5 text-fg-muted">{children}</div>
      )}
      {mono !== undefined && <div className="mono text-[11px] text-fg-subtle">{mono}</div>}
      {actions !== undefined && <div className="mt-1 flex items-center gap-2">{actions}</div>}
    </div>
  );
}
