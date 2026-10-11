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

/** The only container in the app: the panel surface with its edge and shadow (opaque, a 1 px
 *  hairline, radius 10 and no shadow in the classic themes; white glass, a white edge, radius 24 and
 *  a soft shadow in 澄空 / 夜空, tokens.css). */
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
        "border border-panel-edge bg-panel shadow-panel",
        radius === 10 ? "rounded-panel" : "rounded-14",
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
