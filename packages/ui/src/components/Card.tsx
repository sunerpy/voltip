import type { HTMLAttributes, ReactNode } from "react";
import { cx } from "../cx";

export interface CardProps extends HTMLAttributes<HTMLDivElement> {
  children: ReactNode;
  padding?: "none" | "sm" | "md";
  radius?: 10 | 14;
  /** Hover affordance for clickable cards (border darkens). */
  interactive?: boolean;
  selected?: boolean;
}

/** Surface + 1 px hairline + radius 10, no shadow. The only container in the app. */
export function Card({
  children,
  padding = "md",
  radius = 10,
  interactive = false,
  selected = false,
  className,
  ...rest
}: CardProps) {
  return (
    <div
      className={cx(
        "bg-surface hairline",
        radius === 10 ? "rounded-10" : "rounded-14",
        padding === "md" && "p-[var(--card-pad)]",
        padding === "sm" && "p-3",
        interactive && "transition-colors hover:border-fg-subtle",
        // Not `ring-inset`: the palette's `inset` colour token makes Tailwind emit `ring-inset` as a
        // ring colour too, which beats `ring-primary`; the inset-ring utilities have their own var.
        selected && "inset-ring-2 inset-ring-primary",
        className,
      )}
      {...rest}>
      {children}
    </div>
  );
}
