import { LOGO_BARS, fitLogo } from "@voltip/shared";
import { type RefObject, useEffect, useId, useLayoutEffect, useRef, useState } from "react";

/** The app mark 「声波光标」 (2026-10-08), as in `apps/desktop/src-tauri/icons/icon.png` (1024 px):
 *  on a deep ink rounded square, three white sound bars run into a cyan text cursor, speech that
 *  becomes text at the cursor. Drawn as SVG on whole device pixels (see `Logo`), so it stays
 *  crisp at 22–24 px in the sidebar and the phone's header as well as larger. Colours are the
 *  icon's own, not theme tokens: the mark must look the same in every theme, like the window
 *  icon does. */
export const LOGO_INK = "#0B1220";
/** The tile's gradient runs from ink (top left) to this (bottom right). */
export const LOGO_INK_END = "#1B2A4A";
export const LOGO_WAVE = "#FFFFFF";
/** The cursor's gradient runs from this (top) to cyan (bottom). */
export const LOGO_CURSOR_TOP = "#38BDF8";
export const LOGO_CURSOR = "#22D3EE";
export { LOGO_BARS };

export interface LogoProps {
  /** Rendered width and height in px. */
  size?: number;
  className?: string;
  /** Accessible name; omit for a purely decorative mark next to the product name. */
  label?: string;
}

function currentRatio(): number {
  return typeof window === "undefined" ? 1 : window.devicePixelRatio || 1;
}

/** The device pixel ratio, updated when the window moves to a screen with another one. */
function useDevicePixelRatio(): number {
  const [ratio, setRatio] = useState(currentRatio);
  useEffect(() => {
    if (typeof window.matchMedia !== "function") return undefined;
    const query = window.matchMedia(`(resolution: ${ratio}dppx)`);
    const update = () => setRatio(currentRatio());
    query.addEventListener("change", update);
    return () => query.removeEventListener("change", update);
  }, [ratio]);
  return ratio;
}

/** Moves the mark by less than a device pixel each way so that its top-left corner is on a whole
 *  device pixel. The layout is in rem, so at the default 14 px text size the sidebar puts the mark
 *  at x 17.5, y 5.5, and the browser smears every fitted edge across two pixels. The shift is a
 *  relative offset, not a transform: Chromium draws a fractional transform with the same smear.
 *  It is written to the element itself (no render depends on it), and only on a mark the page
 *  has not positioned itself. */
function useWholePixelPosition(ref: RefObject<SVGSVGElement | null>, ratio: number): void {
  useLayoutEffect(() => {
    const svg = ref.current;
    if (!svg) return undefined;
    const position = getComputedStyle(svg).position;
    if (position !== "" && position !== "static") return undefined;
    let shift = [0, 0];
    const snap = () => {
      const rect = svg.getBoundingClientRect();
      // Where the mark sits without the shift already applied.
      const x = rect.left - (shift[0] ?? 0);
      const y = rect.top - (shift[1] ?? 0);
      const next = [Math.round(x * ratio) / ratio - x, Math.round(y * ratio) / ratio - y];
      if (next.some((value, i) => Math.abs(value - (shift[i] ?? 0)) > 1e-3)) {
        shift = next;
        const moved = next.some((value) => Math.abs(value) > 1e-3);
        svg.style.position = moved ? "relative" : "";
        svg.style.left = moved ? `${next[0]}px` : "";
        svg.style.top = moved ? `${next[1]}px` : "";
      }
    };
    snap();
    // The layout around the mark moves it: a text size or density change, a collapsed sidebar, a
    // resized window.
    const observer =
      typeof ResizeObserver === "undefined" || !svg.parentElement
        ? undefined
        : new ResizeObserver(snap);
    if (observer && svg.parentElement) observer.observe(svg.parentElement);
    return () => {
      observer?.disconnect();
      svg.style.position = "";
      svg.style.left = "";
      svg.style.top = "";
    };
  }, [ref, ratio]);
}

/** The mark, `size` px square. It is fitted to the device pixels it covers (`fitLogo` at `size`
 *  × the pixel ratio, so a 24 px mark is fitted at 36 px on a 150 % screen) and moved by less than
 *  a pixel onto whole device pixels. */
export function Logo({ size = 24, className, label }: LogoProps) {
  // Gradient ids are document-wide: each mark on the page needs its own.
  const id = `voltip-logo-${useId().replace(/[^A-Za-z0-9_-]/g, "")}`;
  const ref = useRef<SVGSVGElement>(null);
  const ratio = useDevicePixelRatio();
  useWholePixelPosition(ref, ratio);
  const pixels = Math.round(size * ratio);
  const { bars, cursor } = fitLogo(pixels);
  return (
    <svg
      ref={ref}
      data-testid="app-logo"
      width={size}
      height={size}
      viewBox={`0 0 ${pixels} ${pixels}`}
      role={label ? "img" : "presentation"}
      aria-label={label}
      aria-hidden={label ? undefined : true}
      className={className}
      focusable="false">
      <defs>
        <linearGradient id={`${id}-tile`} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor={LOGO_INK} />
          <stop offset="1" stopColor={LOGO_INK_END} />
        </linearGradient>
        <linearGradient id={`${id}-cursor`} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor={LOGO_CURSOR_TOP} />
          <stop offset="1" stopColor={LOGO_CURSOR} />
        </linearGradient>
      </defs>
      <rect width={pixels} height={pixels} rx={(232 / 1024) * pixels} fill={`url(#${id}-tile)`} />
      {bars.map((bar) => (
        <rect
          key={bar.x}
          x={bar.x}
          y={bar.y}
          width={bar.width}
          height={bar.height}
          rx={bar.width / 2}
          fill={LOGO_WAVE}
        />
      ))}
      <rect
        x={cursor.x}
        y={cursor.y}
        width={cursor.width}
        height={cursor.height}
        rx={cursor.width / 2}
        fill={`url(#${id}-cursor)`}
      />
    </svg>
  );
}
