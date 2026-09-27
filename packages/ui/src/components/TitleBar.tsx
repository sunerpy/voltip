import type { PointerEvent, ReactNode } from "react";
import { cx } from "../cx";
import { useT } from "../i18n/I18nProvider";
import { Icon } from "./Icon";
import { IconButton } from "./IconButton";
import { Lamp } from "./Lamp";
import type { ToolbarReadout } from "./Toolbar";

/** Accessible name (and tooltip) of the search icon button in the default locale; components read
 *  it through `useT()`, this constant stays for callers and tests. */
export const TITLE_BAR_SEARCH_LABEL = "搜索或输入命令 · Ctrl K";

/** Height of the title bar and of the sidebar brand row, in px: together they form one
 *  continuous drag strip across the top of the window. */
export const TITLE_BAR_HEIGHT = 40;

/** The compact readout shows at most this many items (engine, microphone). */
export const TITLE_BAR_READOUT_MAX = 2;

/** Host platform as the desktop app resolves it (user agent first, core identity as the hint). */
export type TitleBarPlatform = "macos" | "windows" | "linux" | "unknown";

export interface TitleBarControls {
  minimize: () => void;
  toggleMaximize: () => void;
  close: () => void;
  /** Touch / pen drag fallback: Tauri's injected drag script only listens to the mouse. */
  startDragging?: () => void;
}

export interface TitleBarProps {
  title: ReactNode;
  /** Compact inline readout after the title (engine model with its lamp, microphone short name):
   *  mono 11 px, subtle, ` · ` separated, truncated when narrow, hidden below `md`. At most
   *  `TITLE_BAR_READOUT_MAX` items are drawn. */
  readouts?: readonly ToolbarReadout[];
  /** Renders the `Ctrl K` search as a single icon button (the 220 px field is gone). */
  onSearch?: () => void;
  /** Right-most content slot (the 润色 toggle); sits left of the window controls. */
  right?: ReactNode;
  platform?: TitleBarPlatform;
  /** Window controls bound to the Tauri window. `null` / omitted (a plain browser: Vite dev,
   *  vitest) hides the buttons instead of drawing three dead ones. */
  controls?: TitleBarControls | null;
  /** Drives the maximize / restore icon and label; synced by the caller from the window. */
  maximized?: boolean;
  className?: string;
}

/** Elements the touch / pen drag fallback must leave alone (mirrors Tauri's drag.js exclusions). */
const INTERACTIVE = "a, button, input, select, textarea, label, summary, [role='button']";

const CONTROL =
  "inline-flex h-full w-11.5 items-center justify-center text-fg-muted outline-none transition-colors hover:bg-nav-active hover:text-fg focus-visible:bg-nav-active focus-visible:text-fg";

/** The 40 px bar that *is* the window title bar: the page title (drag region), a compact
 *  engine · microphone readout right after it (user feedback 2026-09-25: keep the facts, but on the
 *  title bar itself, not on a second row), the `Ctrl K` search icon, the right slot (润色 toggle)
 *  and, on Windows / Linux under Tauri, the window controls.
 *
 *  `data-tauri-drag-region="deep"` on the root makes the whole strip draggable, title and readout
 *  included; Tauri's injected script already excludes buttons and inputs and toggles maximize on
 *  double-click by itself, so there is deliberately no `onDoubleClick` here (a second handler
 *  would flip the window straight back). macOS keeps its native traffic lights
 *  (`titleBarStyle: "Overlay"`), so the in-bar controls are hidden there. */
export function TitleBar({
  title,
  readouts = [],
  onSearch,
  right,
  platform = "unknown",
  controls = null,
  maximized = false,
  className,
}: TitleBarProps) {
  const t = useT();
  const showControls = platform !== "macos" && controls !== null;
  const shown = readouts.slice(0, TITLE_BAR_READOUT_MAX);

  const onPointerDown = (e: PointerEvent<HTMLElement>) => {
    // Mouse drags stay with Tauri's native script; only touch and pen need the fallback.
    if (e.pointerType === "mouse" || !controls?.startDragging) return;
    if (e.target instanceof Element && e.target.closest(INTERACTIVE)) return;
    controls.startDragging();
  };

  return (
    <header
      data-tauri-drag-region="deep"
      data-testid="title-bar"
      data-platform={platform}
      onPointerDown={onPointerDown}
      className={cx(
        "flex h-10 shrink-0 items-center border-b border-border bg-surface select-none",
        className,
      )}>
      <div className="flex min-w-0 flex-1 items-center gap-3 px-6">
        <h1 className="shrink-0 truncate text-[14px] font-semibold text-fg">{title}</h1>
        {shown.length > 0 && (
          <div
            data-testid="title-bar-readout"
            aria-label={t("ui.titleBar.readout")}
            className="mono hidden min-w-0 flex-1 items-center gap-1.5 truncate text-[11px] text-fg-subtle md:flex">
            {shown.map((r, i) => (
              <span
                key={`${r.label}-${i}`}
                title={r.title ?? `${r.label} · ${r.value}`}
                className="flex min-w-0 items-center gap-1.5 whitespace-nowrap">
                {i > 0 && <span aria-hidden>·</span>}
                {r.lamp && <Lamp tone={r.lamp} size={6} />}
                <span className="truncate">{r.value}</span>
                {r.badge !== undefined && (
                  <span
                    data-testid="title-bar-readout-badge"
                    className="rounded-6 bg-inset px-1 text-[10px] leading-4 text-fg-muted">
                    {r.badge}
                  </span>
                )}
              </span>
            ))}
          </div>
        )}
        {shown.length === 0 && <span className="min-w-0 flex-1" />}
        {onSearch && (
          <IconButton
            icon="search"
            label={t("ui.titleBar.search")}
            size={28}
            onClick={onSearch}
            data-testid="title-bar-search"
          />
        )}
        {right !== undefined && <div className="flex shrink-0 items-center gap-1">{right}</div>}
      </div>
      {showControls && (
        <div data-testid="window-controls" className="flex h-full shrink-0 items-stretch">
          <button
            type="button"
            aria-label={t("ui.titleBar.minimize")}
            title={t("ui.titleBar.minimize")}
            onClick={controls.minimize}
            className={CONTROL}>
            <Icon name="minimize" size={16} />
          </button>
          <button
            type="button"
            data-state={maximized ? "maximized" : "normal"}
            aria-label={maximized ? t("ui.titleBar.restore") : t("ui.titleBar.maximize")}
            title={maximized ? t("ui.titleBar.restore") : t("ui.titleBar.maximize")}
            onClick={controls.toggleMaximize}
            className={CONTROL}>
            <Icon name={maximized ? "restore" : "maximize"} size={16} />
          </button>
          <button
            type="button"
            aria-label={t("ui.titleBar.close")}
            title={t("ui.titleBar.close")}
            onClick={controls.close}
            className={cx(
              CONTROL,
              "hover:bg-danger hover:text-primary-fg focus-visible:bg-danger focus-visible:text-primary-fg",
            )}>
            <Icon name="close" size={16} />
          </button>
        </div>
      )}
    </header>
  );
}
