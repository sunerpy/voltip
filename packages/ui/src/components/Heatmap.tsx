import { cx } from "../cx";
import { useT } from "../i18n/I18nProvider";

export interface HeatmapProps {
  /** `values[col][row]`, each 0..3. */
  values: readonly (readonly number[])[];
  columnLabels?: readonly string[];
  legend?: boolean;
  cell?: number;
  gap?: number;
  label?: string;
  className?: string;
}

const LEVEL_CLASS = ["bg-inset2", "bg-accent-soft", "bg-accent opacity-60", "bg-accent"] as const;

export function levelClass(level: number): string {
  const idx = Math.min(3, Math.max(0, Math.round(level)));
  return LEVEL_CLASS[idx] ?? "bg-inset2";
}

/** 6 weeks × 7 days activity grid; four accent levels, never green. */
export function Heatmap({
  values,
  columnLabels,
  legend = true,
  cell = 10,
  gap = 3,
  label,
  className,
}: HeatmapProps) {
  const t = useT();
  return (
    <div
      className={cx("inline-flex flex-col gap-1.5", className)}
      role="img"
      aria-label={label ?? t("ui.a11y.heatmap")}>
      {columnLabels && (
        <div className="flex" style={{ gap }}>
          {columnLabels.map((c) => (
            <span
              key={c}
              className="mono text-center text-[9px] leading-none text-fg-subtle"
              style={{ width: cell }}>
              {c}
            </span>
          ))}
        </div>
      )}
      <div className="flex" style={{ gap }}>
        {values.map((col, ci) => (
          <div key={ci} className="flex flex-col" style={{ gap }}>
            {col.map((v, ri) => (
              <span
                key={ri}
                data-level={v}
                className={cx("rounded-[2px]", levelClass(v))}
                style={{ width: cell, height: cell }}
              />
            ))}
          </div>
        ))}
      </div>
      {legend && (
        <div className="mono flex items-center gap-1 text-[10px] text-fg-subtle">
          <span>{t("ui.a11y.less")}</span>
          {LEVEL_CLASS.map((c) => (
            <span
              key={c}
              className={cx("inline-block rounded-[2px]", c)}
              style={{ width: 8, height: 8 }}
            />
          ))}
          <span>{t("ui.a11y.more")}</span>
        </div>
      )}
    </div>
  );
}
