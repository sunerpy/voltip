import { create as createQr } from "qrcode";
import type { ReactNode } from "react";
import { cx } from "../cx";
import { useT } from "../i18n/I18nProvider";

export interface QrCodeProps {
  value: string;
  /** Rendered size in px, including the quiet zone. */
  size?: number;
  quietZone?: number;
  /** Fade the code (expired) and show `overlay` centred on top. */
  dimmed?: boolean;
  overlay?: ReactNode;
  label?: string;
  className?: string;
}

export interface QrMatrix {
  size: number;
  dark: (x: number, y: number) => boolean;
}

/** Pure encoding via `qrcode.create`; rendering is our own SVG so it works in jsdom and any theme. */
export function encodeQr(value: string): QrMatrix {
  const { modules } = createQr(value, { errorCorrectionLevel: "M" });
  const size = modules.size;
  return { size, dark: (x, y) => modules.get(y, x) === 1 };
}

export function qrPath(matrix: QrMatrix): string {
  const parts: string[] = [];
  for (let y = 0; y < matrix.size; y += 1) {
    let x = 0;
    while (x < matrix.size) {
      if (!matrix.dark(x, y)) {
        x += 1;
        continue;
      }
      let run = 1;
      while (x + run < matrix.size && matrix.dark(x + run, y)) run += 1;
      parts.push(`M${x} ${y}h${run}v1h-${run}z`);
      x += run;
    }
  }
  return parts.join("");
}

export function QrCode({
  value,
  size = 168,
  quietZone = 2,
  dimmed = false,
  overlay,
  label,
  className,
}: QrCodeProps) {
  const t = useT();
  const matrix = encodeQr(value);
  const total = matrix.size + quietZone * 2;
  return (
    <div
      className={cx("relative inline-block rounded-6 bg-surface hairline", className)}
      style={{ width: size, height: size }}>
      <svg
        role="img"
        aria-label={label ?? t("ui.a11y.qr")}
        viewBox={`0 0 ${total} ${total}`}
        width={size}
        height={size}
        shapeRendering="crispEdges"
        className={cx("block rounded-6 transition-opacity", dimmed && "opacity-40")}>
        <rect width={total} height={total} fill="var(--surface)" />
        <path
          transform={`translate(${quietZone} ${quietZone})`}
          d={qrPath(matrix)}
          fill="var(--fg)"
        />
      </svg>
      {overlay !== undefined && (
        <div className="absolute inset-0 flex items-center justify-center">{overlay}</div>
      )}
    </div>
  );
}
