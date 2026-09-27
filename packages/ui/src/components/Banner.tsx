import type { ReactNode } from "react";
import { cx } from "../cx";
import { useT } from "../i18n/I18nProvider";
import { Icon } from "./Icon";
import { IconButton } from "./IconButton";
import { Lamp, type LampTone } from "./Lamp";

export type BannerTone = "neutral" | "ok" | "danger" | "warn" | "info";

export interface BannerProps {
  tone?: BannerTone;
  title?: ReactNode;
  children?: ReactNode;
  actions?: ReactNode;
  onDismiss?: () => void;
  /** `bar`: 3 px colour bar at the left edge (load errors, conflicts). `lamp`: 8 px dot. `icon`: ⚠. */
  marker?: "bar" | "lamp" | "icon";
  className?: string;
}

const LAMP_TONE: Record<BannerTone, LampTone> = {
  neutral: "idle",
  ok: "ok",
  danger: "danger",
  warn: "warn",
  info: "accent",
};
const BAR_CLASS: Record<BannerTone, string> = {
  neutral: "border-l-fg-subtle",
  ok: "border-l-ok",
  danger: "border-l-danger",
  warn: "border-l-warning",
  info: "border-l-accent",
};
const SOFT_CLASS: Record<BannerTone, string> = {
  neutral: "bg-surface",
  ok: "bg-surface",
  danger: "bg-danger-soft",
  warn: "bg-warning-soft",
  info: "bg-info-soft",
};

export function Banner({
  tone = "neutral",
  title,
  children,
  actions,
  onDismiss,
  marker = "lamp",
  className,
}: BannerProps) {
  const t = useT();
  return (
    <div
      role={tone === "danger" ? "alert" : "status"}
      data-tone={tone}
      className={cx(
        "flex items-start gap-3 rounded-10 px-4 py-3 text-[13px] hairline",
        marker === "bar" ? cx("border-l-[3px]", BAR_CLASS[tone], SOFT_CLASS[tone]) : "bg-surface",
        className,
      )}>
      {marker === "lamp" && <Lamp tone={LAMP_TONE[tone]} className="mt-[6px]" />}
      {marker === "icon" && (
        <Icon
          name="alert"
          size={16}
          className={cx("mt-0.5 shrink-0", tone === "danger" ? "text-danger" : "text-fg-muted")}
        />
      )}
      <div className="min-w-0 flex-1">
        {title !== undefined && <div className="font-medium text-fg">{title}</div>}
        {children !== undefined && (
          <div className="text-[12px] leading-5 text-fg-muted">{children}</div>
        )}
      </div>
      {actions !== undefined && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
      {onDismiss && <IconButton icon="x" label={t("ui.banner.close")} onClick={onDismiss} />}
    </div>
  );
}
