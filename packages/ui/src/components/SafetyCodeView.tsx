import type { SafetyCode } from "@voltip/shared";
import { cx } from "../cx";
import { useT } from "../i18n/I18nProvider";

export interface SafetyCodeViewProps {
  code: SafetyCode;
  size?: "sm" | "md";
  className?: string;
}

/** Four words in mono boxes plus the fingerprint; identical on both devices when the channel is honest. */
export function SafetyCodeView({ code, size = "md", className }: SafetyCodeViewProps) {
  const t = useT();
  return (
    <div className={cx("flex flex-col items-center gap-3", className)}>
      <ol aria-label={t("ui.a11y.safetyCode")} className="grid grid-cols-4 gap-2">
        {code.words.map((w, i) => (
          <li
            key={`${w}-${i}`}
            className={cx(
              "mono flex items-center justify-center rounded-10 bg-inset text-fg hairline",
              size === "md"
                ? "h-12 min-w-[84px] px-3 text-[15px]"
                : "h-9 min-w-[64px] px-2 text-[12px]",
            )}>
            {w}
          </li>
        ))}
      </ol>
      <div
        className="mono text-[12px] tracking-wider text-fg-muted"
        aria-label={t("ui.a11y.fingerprint")}>
        {code.fingerprint}
      </div>
    </div>
  );
}
