import type { ButtonHTMLAttributes } from "react";
import { cx } from "../cx";
import { Icon, type IconName } from "./Icon";

export interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  icon: IconName;
  /** Required: the icon alone is not a name. */
  label: string;
  size?: 24 | 28;
  tone?: "default" | "danger";
  bordered?: boolean;
}

export function IconButton({
  icon,
  label,
  size = 24,
  tone = "default",
  bordered = false,
  className,
  type = "button",
  ...rest
}: IconButtonProps) {
  return (
    <button
      type={type}
      aria-label={label}
      title={label}
      className={cx(
        "inline-flex shrink-0 items-center justify-center rounded-6 transition-colors",
        "text-fg-muted hover:bg-inset hover:text-fg disabled:cursor-not-allowed disabled:opacity-50",
        tone === "danger" && "hover:text-danger",
        bordered && "bg-surface hairline",
        className,
      )}
      style={{ width: size, height: size }}
      {...rest}>
      <Icon name={icon} size={size === 24 ? 14 : 16} />
    </button>
  );
}
