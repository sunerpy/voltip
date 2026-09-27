/** The app mark, traced from `apps/desktop/src-tauri/icons/icon.png` (1024 px): a navy rounded
 *  square with a V whose left arm is pale and right arm is orange. Drawn as SVG so it stays crisp
 *  at 24 px in the sidebar and 56 px on the phone welcome screen. Colours are the icon's own, not
 *  theme tokens: the mark must look the same in every theme, like the window icon does. */
export const LOGO_NAVY = "#0B1220";
export const LOGO_PALE = "#E7EDF5";
export const LOGO_ORANGE = "#F97316";

export interface LogoProps {
  /** Rendered width and height in px. */
  size?: number;
  className?: string;
  /** Accessible name; omit for a purely decorative mark next to the product name. */
  label?: string;
}

export function Logo({ size = 24, className, label }: LogoProps) {
  return (
    <svg
      data-testid="app-logo"
      width={size}
      height={size}
      viewBox="0 0 100 100"
      role={label ? "img" : "presentation"}
      aria-label={label}
      aria-hidden={label ? undefined : true}
      className={className}
      focusable="false">
      <rect width="100" height="100" rx="22" fill={LOGO_NAVY} />
      <polygon points="22.5,22 39.5,22 50,46 50,79" fill={LOGO_PALE} />
      <polygon points="60.5,22 77.5,22 50,79 50,46" fill={LOGO_ORANGE} />
    </svg>
  );
}
