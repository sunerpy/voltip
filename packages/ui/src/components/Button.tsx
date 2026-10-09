import type { ButtonHTMLAttributes, ReactNode } from "react";
import { cx } from "../cx";
import { usePresentation } from "../presentation/PresentationProvider";
import { TOUCH_CONTROL, TOUCH_TARGET } from "../presentation/touch";
import { Icon, type IconName } from "./Icon";
import { Keycaps } from "./Keycap";

export type ButtonVariant =
  | "primary"
  | "outline"
  | "ghost"
  | "danger"
  | "text"
  | "text-danger"
  | "text-muted";
export type ButtonSize = "sm" | "md";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  icon?: IconName;
  /** Keyboard hint rendered as keycaps after the label, e.g. `Ctrl N`. */
  keys?: string;
  loading?: boolean;
  children?: ReactNode;
}

const VARIANT_CLASS: Record<ButtonVariant, string> = {
  primary: "bg-primary text-primary-fg hover:opacity-90",
  outline: "bg-surface text-fg hairline hover:bg-inset",
  ghost: "bg-transparent text-fg hover:bg-inset",
  danger: "bg-danger text-surface hover:opacity-90",
  // Links as Codex draws them: the accent text colour, no underline, a stronger shade on hover.
  text: "bg-transparent px-0 text-accent-text hover:text-accent-text-hover",
  // A destructive or secondary action in the same place: its own colour, and its own hover.
  "text-danger": "bg-transparent px-0 text-danger hover:underline",
  "text-muted": "bg-transparent px-0 text-fg-muted hover:text-fg",
};

const SIZE_CLASS: Record<ButtonSize, string> = {
  sm: "h-7 px-2.5 text-[12px] gap-1.5",
  md: "h-8 px-3 text-[13px] gap-2",
};

/** The pressed state under the touch presentation, where no hover comes before the tap: a step
 *  past the hover colour on surfaces, a little lighter for fills and links. */
const PRESSED_CLASS: Record<ButtonVariant, string> = {
  primary: "active:opacity-80",
  outline: "active:bg-inset2",
  ghost: "active:bg-inset2",
  danger: "active:opacity-80",
  text: "active:opacity-60",
  "text-danger": "active:opacity-60",
  "text-muted": "active:opacity-60",
};

export function Button({
  variant = "outline",
  size = "md",
  icon,
  keys,
  loading = false,
  children,
  className,
  disabled,
  type = "button",
  ...rest
}: ButtonProps) {
  const touch = usePresentation() === "touch";
  const off = disabled === true || loading;
  return (
    <button
      type={type}
      disabled={disabled || loading}
      aria-busy={loading || undefined}
      data-variant={variant}
      className={cx(
        "inline-flex shrink-0 items-center justify-center rounded-6 font-medium whitespace-nowrap transition-colors select-none",
        "disabled:cursor-not-allowed disabled:opacity-50 disabled:hover:opacity-50",
        VARIANT_CLASS[variant],
        SIZE_CLASS[size],
        // The phone: the same size, a 44 px target around it, and a pressed state.
        touch && TOUCH_CONTROL,
        touch && TOUCH_TARGET,
        touch && !off && PRESSED_CLASS[variant],
        className,
      )}
      {...rest}>
      {icon && <Icon name={icon} size={size === "sm" ? 13 : 14} />}
      {children}
      {keys && <Keycaps keys={keys} className="ml-1" />}
    </button>
  );
}
