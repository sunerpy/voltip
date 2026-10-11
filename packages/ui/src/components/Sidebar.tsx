import type { ReactNode } from "react";
import { cx } from "../cx";
import { useT } from "../i18n/I18nProvider";
import { Logo } from "./Logo";
import { Icon, type IconName } from "./Icon";
import { Lamp, type LampTone } from "./Lamp";

export interface SidebarItem {
  id: string;
  label: string;
  icon: IconName;
  count?: number | string;
}

export interface SidebarGroup {
  title: string;
  items: SidebarItem[];
}

/** Width of the expanded sidebar and of the collapsed icon rail, in px (Penpot 「澄空 Glass ·
 *  Voltip」 board 01: a 232 px panel floating 16 px from the window's edges). */
export const SIDEBAR_WIDTH = 232;
export const SIDEBAR_RAIL_WIDTH = 72;

/** macOS draws its native traffic lights at `trafficLightPosition` x 32, y 42 (tauri.macos.conf.json):
 *  inside the floating sidebar's brand row, three 12 px buttons 20 px apart, ending 84 px from the
 *  window's left edge. Whatever sits next to them starts 16 px later, in px so the 字号 setting
 *  (which scales rem) cannot slide it back under them (user feedback 2026-09-29: with 8 px of air
 *  the lights crowded the app mark). */
export const TRAFFIC_LIGHTS_CLEARANCE = 100;
/** The brand row's left padding beside the traffic lights: the clearance less the sidebar's 16 px
 *  margin and its 12 px padding. */
export const TRAFFIC_LIGHTS_BRAND_INSET = "pl-[72px]";
/** The collapsed rail on macOS holds the lights with 16 px on either side. */
export const SIDEBAR_RAIL_WIDTH_TRAFFIC_LIGHTS = 84;

/** One 44 px sidebar row, shared by the nav items so their glyphs and labels line up. */
export const SIDEBAR_ROW_CLASS =
  "flex h-11 w-full items-center rounded-row text-[14px] font-medium transition-colors";
/** An expanded row: the glyph 14 px from the edge, the label 12 px after the 18 px glyph. */
export const SIDEBAR_ROW_EXPANDED_CLASS = "gap-3 px-3.5";
/** The square buttons under the nav (反馈, the theme menu, 设置): a third of the row each, or the
 *  rail's width when collapsed. */
export const SIDEBAR_FOOTER_BUTTON_CLASS =
  "flex h-11 w-full items-center justify-center rounded-row border border-field-edge bg-field text-fg-muted transition-colors hover:bg-nav-active hover:text-fg disabled:cursor-not-allowed disabled:opacity-40";

export interface SidebarProps {
  groups: readonly SidebarGroup[];
  activeId: string;
  onNavigate: (id: string) => void;
  /** Items greyed out (onboarding keeps only the home item live). */
  disabledIds?: readonly string[];
  /** Brand lamp: green when the core is ready. */
  statusTone?: LampTone;
  /** The icon rail: glyphs only; every entry keeps its label as its name and tooltip. */
  collapsed?: boolean;
  /** Layout controls (collapse, hide, pin): in the brand row, or stacked under it in the rail. */
  controls?: ReactNode;
  /** A card over the footer (the relay and the phones); the rail leaves it out. */
  status?: ReactNode;
  /** The square buttons at the bottom (feedback, the theme menu, settings), drawn with
   *  {@link SidebarButton} so they follow `collapsed`. */
  footer?: ReactNode;
  /** Drawn over the page (the edge preview of a hidden sidebar): the window shadow. */
  floating?: boolean;
  brand?: string;
  /** macOS keeps its native traffic lights in the brand row (`titleBarStyle: "Overlay"`): the row
   *  starts clear of them and shows the wordmark without the mark (the Dock, the menu bar and the
   *  tray already show it), and the collapsed rail widens to
   *  {@link SIDEBAR_RAIL_WIDTH_TRAFFIC_LIGHTS} px. Keep in sync with `trafficLightPosition` in
   *  tauri.macos.conf.json. */
  trafficLights?: boolean;
  className?: string;
}

export interface SidebarEntryProps {
  icon: IconName;
  label: string;
  onClick: () => void;
  collapsed?: boolean;
  /** The current page (nav items only): `aria-current="page"` and the active surface. */
  current?: boolean;
  disabled?: boolean;
  /** Right-aligned text while expanded (a count). */
  trailing?: ReactNode;
  /** The entry opens a dialog rather than navigating (settings, feedback). */
  opensDialog?: boolean;
  "data-testid"?: string;
}

/** One 44 px nav row: a glyph and, while expanded, the label. Collapsed, the label stays as the
 *  accessible name and the tooltip, since nothing else on screen says what the glyph is. */
export function SidebarEntry({
  icon,
  label,
  onClick,
  collapsed = false,
  current = false,
  disabled = false,
  trailing,
  opensDialog = false,
  "data-testid": testId,
}: SidebarEntryProps) {
  return (
    <button
      type="button"
      aria-current={current ? "page" : undefined}
      aria-haspopup={opensDialog ? "dialog" : undefined}
      aria-label={collapsed ? label : undefined}
      title={collapsed ? label : undefined}
      disabled={disabled}
      onClick={onClick}
      data-testid={testId}
      className={cx(
        SIDEBAR_ROW_CLASS,
        collapsed ? "justify-center" : SIDEBAR_ROW_EXPANDED_CLASS,
        current
          ? "bg-nav-active text-fg shadow-row"
          : "text-fg-muted hover:bg-nav-active hover:text-fg",
        disabled && "cursor-not-allowed opacity-40 hover:bg-transparent hover:text-fg-muted",
      )}>
      <Icon name={icon} size={18} className={current ? "text-primary" : "text-fg-subtle"} />
      {!collapsed && (
        <>
          <span className="flex-1 truncate text-left">{label}</span>
          {trailing !== undefined && (
            <span className="text-[12px] font-medium text-fg-subtle">{trailing}</span>
          )}
        </>
      )}
    </button>
  );
}

export interface SidebarButtonProps {
  icon: IconName;
  label: string;
  onClick: () => void;
  disabled?: boolean;
  /** The button opens a dialog (settings, feedback). */
  opensDialog?: boolean;
  "data-testid"?: string;
}

/** A square footer button: its label is its accessible name and tooltip. */
export function SidebarButton({
  icon,
  label,
  onClick,
  disabled = false,
  opensDialog = false,
  "data-testid": testId,
}: SidebarButtonProps) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      aria-haspopup={opensDialog ? "dialog" : undefined}
      disabled={disabled}
      onClick={onClick}
      data-testid={testId}
      className={SIDEBAR_FOOTER_BUTTON_CLASS}>
      <Icon name={icon} size={18} />
    </button>
  );
}

/** The nav panel, floating 16 px from the window's edges (board 01): a brand row (part of the
 *  window's drag area) with the layout controls, the grouped 44 px items, an optional status card
 *  and the square footer buttons. 232 px wide, or the 72 px icon rail when `collapsed`. */
export function Sidebar({
  groups,
  activeId,
  onNavigate,
  disabledIds = [],
  statusTone = "ok",
  collapsed = false,
  controls,
  status,
  footer,
  floating = false,
  brand = "Voltip",
  trafficLights = false,
  className,
}: SidebarProps) {
  const t = useT();
  const rail = trafficLights ? SIDEBAR_RAIL_WIDTH_TRAFFIC_LIGHTS : SIDEBAR_RAIL_WIDTH;
  const width = collapsed ? rail : SIDEBAR_WIDTH;
  return (
    <nav
      aria-label={t("ui.sidebar.nav")}
      data-collapsed={collapsed}
      data-floating={floating}
      // A flex item's automatic minimum is its content: pin all three so a long label can never
      // hold the rail open wider than it is.
      style={{ width, minWidth: width, maxWidth: width }}
      className={cx(
        "flex h-full shrink-0 flex-col rounded-panel border border-sidebar-edge bg-sidebar pb-3",
        collapsed ? "px-2" : "px-3",
        floating ? "shadow-win glass-blur" : "shadow-sidebar",
        className,
      )}>
      {/* Brand row: `deep` lets the brand text drag the window too (Tauri's drag script leaves
          buttons alone); the lamp is not clickable. */}
      <div
        data-tauri-drag-region="deep"
        data-testid="sidebar-brand"
        className={cx(
          "flex h-16 shrink-0 items-center gap-2.5 select-none",
          collapsed
            ? "justify-center"
            : trafficLights
              ? cx(TRAFFIC_LIGHTS_BRAND_INSET, "pr-1")
              : "px-2",
          collapsed && trafficLights && "invisible",
        )}>
        {!trafficLights && <Logo size={28} className="shrink-0" />}
        {!collapsed && (
          <>
            <span className="text-[17px] font-semibold text-fg">{brand}</span>
            <Lamp tone={statusTone} size={6} label={t("ui.sidebar.coreStatus")} />
            {controls !== undefined && (
              <div
                role="group"
                aria-label={t("ui.sidebar.layout")}
                data-testid="sidebar-controls"
                className="ml-auto flex items-center gap-0.5">
                {controls}
              </div>
            )}
          </>
        )}
      </div>
      {collapsed && controls !== undefined && (
        <div
          role="group"
          aria-label={t("ui.sidebar.layout")}
          data-testid="sidebar-controls"
          className="flex flex-col items-center gap-0.5 border-b border-border pb-2">
          {controls}
        </div>
      )}
      <div className="mt-2 flex flex-1 flex-col gap-5 overflow-y-auto">
        {groups.map((g) => (
          <div key={g.title} role="group" aria-label={g.title}>
            {collapsed ? (
              <div aria-hidden className="mx-2 mb-1 h-px bg-border first:hidden" />
            ) : (
              <div className="mb-1.5 px-2 text-[11px] font-medium text-fg-subtle">{g.title}</div>
            )}
            <ul className="flex flex-col gap-1">
              {g.items.map((item) => (
                <li key={item.id}>
                  <SidebarEntry
                    icon={item.icon}
                    label={
                      collapsed && item.count !== undefined
                        ? `${item.label} · ${item.count}`
                        : item.label
                    }
                    collapsed={collapsed}
                    current={item.id === activeId}
                    disabled={disabledIds.includes(item.id)}
                    trailing={item.count}
                    onClick={() => {
                      onNavigate(item.id);
                    }}
                  />
                </li>
              ))}
            </ul>
          </div>
        ))}
      </div>
      {status !== undefined && !collapsed && (
        <div data-testid="sidebar-status" className="mt-3">
          {status}
        </div>
      )}
      {footer !== undefined && (
        <div
          data-testid="sidebar-footer"
          className={cx("mt-3 grid gap-2", collapsed ? "grid-cols-1" : "grid-cols-3")}>
          {footer}
        </div>
      )}
    </nav>
  );
}
