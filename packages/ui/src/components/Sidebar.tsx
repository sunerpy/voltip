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

/** Width of the expanded sidebar and of the collapsed icon rail, in px. */
export const SIDEBAR_WIDTH = 224;
export const SIDEBAR_RAIL_WIDTH = 56;

export interface SidebarProps {
  groups: readonly SidebarGroup[];
  activeId: string;
  onNavigate: (id: string) => void;
  /** Items greyed out (onboarding keeps only the home item live). */
  disabledIds?: readonly string[];
  /** Brand lamp: green when the core is ready. */
  statusTone?: LampTone;
  /** The 56 px icon rail: glyphs only; every entry keeps its label as its name and tooltip. */
  collapsed?: boolean;
  /** Layout controls (collapse, hide, pin): in the brand row, or stacked under it in the rail. */
  controls?: ReactNode;
  /** Entries pinned to the bottom (feedback, the theme switch, settings), drawn with
   *  {@link SidebarEntry} so they follow `collapsed` like the nav items. */
  footer?: ReactNode;
  /** Drawn over the page (the edge preview of a hidden sidebar): a shadow instead of the border. */
  floating?: boolean;
  brand?: string;
  /** macOS keeps its native traffic lights over the top-left corner (`titleBarStyle: "Overlay"`);
   *  reserve their 72 px (x 12 + 2 × 20 pitch + 12 button + 8 breathing) so the brand does not
   *  sit under them. Keep in sync with `trafficLightPosition` in tauri.macos.conf.json. */
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

/** One 36 px sidebar row: a glyph and, while expanded, the label. Collapsed, the label stays as
 *  the accessible name and the tooltip, since nothing else on screen says what the glyph is. */
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
        "flex h-9 w-full items-center rounded-6 text-[13px] transition-colors",
        collapsed ? "justify-center" : "gap-2.5 px-2",
        current
          ? "bg-nav-active font-medium text-fg"
          : "text-fg-muted hover:bg-nav-active hover:text-fg",
        disabled && "cursor-not-allowed opacity-40 hover:bg-transparent hover:text-fg-muted",
      )}>
      <Icon name={icon} size={16} className={current ? "text-fg" : "text-fg-subtle"} />
      {!collapsed && (
        <>
          <span className="flex-1 truncate text-left">{label}</span>
          {trailing !== undefined && (
            <span className="mono text-[11px] text-fg-subtle">{trailing}</span>
          )}
        </>
      )}
    </button>
  );
}

/** The nav rail: a 40 px brand row (part of the window drag strip) with the layout controls,
 *  the grouped items (36 px) and the footer entries pinned to the bottom. 224 px wide, or the
 *  56 px icon rail when `collapsed`. */
export function Sidebar({
  groups,
  activeId,
  onNavigate,
  disabledIds = [],
  statusTone = "ok",
  collapsed = false,
  controls,
  footer,
  floating = false,
  brand = "Voltip",
  trafficLights = false,
  className,
}: SidebarProps) {
  const t = useT();
  const width = collapsed ? SIDEBAR_RAIL_WIDTH : SIDEBAR_WIDTH;
  return (
    <nav
      aria-label={t("ui.sidebar.nav")}
      data-collapsed={collapsed}
      data-floating={floating}
      // A flex item's automatic minimum is its content: pin all three so a long label can never
      // hold the rail open wider than 56 px.
      style={{ width, minWidth: width, maxWidth: width }}
      className={cx(
        "flex h-full shrink-0 flex-col bg-nav pb-4",
        collapsed ? "px-2" : "px-3",
        floating ? "shadow-win" : "border-r border-border",
        className,
      )}>
      {/* Brand row: exactly 40 px, flush with the top edge, so it and the TitleBar form one
          continuous drag strip. `deep` lets the brand text drag too (Tauri's drag script leaves
          buttons alone); the lamp is not clickable. */}
      <div
        data-tauri-drag-region="deep"
        data-testid="sidebar-brand"
        className={cx(
          "flex h-10 shrink-0 items-center gap-2.5 select-none",
          collapsed ? "justify-center" : trafficLights ? "pl-15 pr-2" : "px-2",
          collapsed && trafficLights && "invisible",
        )}>
        <Logo size={24} className="shrink-0" />
        {!collapsed && (
          <>
            <span className="text-[15px] font-semibold text-fg">{brand}</span>
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
      <div
        className={cx("flex flex-1 flex-col gap-5 overflow-y-auto", collapsed ? "mt-3" : "mt-6")}>
        {groups.map((g) => (
          <div key={g.title} role="group" aria-label={g.title}>
            {collapsed ? (
              <div aria-hidden className="mx-2 mb-1 h-px bg-border first:hidden" />
            ) : (
              <div className="mb-1 px-2 text-[11px] text-fg-subtle">{g.title}</div>
            )}
            <ul className="flex flex-col gap-0.5">
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
      {footer !== undefined && (
        <div data-testid="sidebar-footer" className="mt-4 flex flex-col gap-0.5">
          {footer}
        </div>
      )}
    </nav>
  );
}
