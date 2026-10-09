import { cx } from "../cx";
import { useT } from "../i18n/I18nProvider";

export interface SparklineProps {
  values: readonly number[];
  width?: number;
  height?: number;
  label?: string;
  className?: string;
}

export function sparklinePoints(values: readonly number[], width: number, height: number): string {
  if (values.length === 0) return "";
  const max = Math.max(...values, 1);
  const min = Math.min(...values, 0);
  const span = max - min || 1;
  const stepX = values.length > 1 ? width / (values.length - 1) : 0;
  return values
    .map((v, i) => {
      const x = (i * stepX).toFixed(1);
      const y = (height - ((v - min) / span) * (height - 2) - 1).toFixed(1);
      return `${x},${y}`;
    })
    .join(" ");
}

/** Tiny accent polyline; used for latency trends in detail sheets. */
export function Sparkline({ values, width = 80, height = 20, label, className }: SparklineProps) {
  const t = useT();
  return (
    <svg
      role="img"
      aria-label={label ?? t("ui.a11y.sparkline")}
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      className={cx("text-accent", className)}>
      <polyline
        fill="none"
        stroke="currentColor"
        strokeWidth={1.5}
        strokeLinejoin="round"
        strokeLinecap="round"
        points={sparklinePoints(values, width, height)}
      />
    </svg>
  );
}
