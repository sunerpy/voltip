import { cx } from "../cx";

export interface KeycapProps {
  children: string;
  /** `danger`: outlined and lettered in the danger colour (the pill's Esc-to-cancel hint). */
  tone?: "default" | "danger";
  className?: string;
}

/** `<kbd>`: mono 11 px, keycap surface, radius 6. */
export function Keycap({ children, tone = "default", className }: KeycapProps) {
  return (
    <kbd
      className={cx(
        "mono inline-flex h-5 min-w-5 items-center justify-center rounded-6 border bg-keycap-bg px-1.5 text-[11px] leading-none",
        tone === "danger" ? "border-danger text-danger" : "border-keycap-border text-fg-muted",
        className,
      )}>
      {children}
    </kbd>
  );
}

export interface KeycapsProps {
  /** `Ctrl+Alt+Space` or `Ctrl Alt Space`. */
  keys: string;
  /** Render `+` between caps (the home status row) instead of a 6 px gap. */
  plus?: boolean;
  className?: string;
}

export function splitKeys(keys: string): string[] {
  return keys
    .split(/\s*\+\s*|\s+/)
    .map((k) => k.trim())
    .filter((k) => k.length > 0);
}

export function Keycaps({ keys, plus = false, className }: KeycapsProps) {
  const parts = splitKeys(keys);
  return (
    <span
      className={cx("inline-flex items-center gap-1.5", className)}
      aria-label={parts.join(" ")}>
      {parts.map((part, i) => (
        <span key={`${part}-${i}`} className="inline-flex items-center gap-1.5">
          {plus && i > 0 && <span className="text-[11px] text-fg-subtle">+</span>}
          <Keycap>{part}</Keycap>
        </span>
      ))}
    </span>
  );
}
