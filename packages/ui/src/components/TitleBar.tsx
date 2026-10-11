import type { PointerEvent, ReactNode } from "react";
import { cx } from "../cx";
import { useT } from "../i18n/I18nProvider";
import { Icon } from "./Icon";
import { Keycaps } from "./Keycap";

/** Accessible name (and tooltip) of the search field in the default locale; components read it
 *  through `useT()`, this constant stays for callers and tests. */
export const TITLE_BAR_SEARCH_LABEL = "搜索或输入命令 · Ctrl K";

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
  /** The line under the title (Penpot 「澄空 Glass · Voltip」 board 01): a summary of the page, or
   *  the speech model and microphone menus. The box does not clip, so a menu that opens from it is
   *  not cut off; the buttons truncate their own text. */
  subtitle?: ReactNode;
  /** Names the subtitle as a group when it holds controls (the switch menus). */
  subtitleLabel?: string;
  /** The `Ctrl K` command field: a pill that opens the command palette. */
  onSearch?: () => void;
  /** Right of the search field (the AI 润色 chip, the update badge); left of the window controls. */
  right?: ReactNode;
  /** Before the title: the button that brings a hidden sidebar back. */
  left?: ReactNode;
  /** The bar starts at the window's left edge on macOS (no sidebar beside it): its content starts
   *  `TRAFFIC_LIGHTS_CLEARANCE` (100) px in, clear of the traffic lights. */
  trafficLights?: boolean;
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
  "inline-flex h-full w-10 items-center justify-center text-fg-subtle outline-none transition-colors hover:bg-field hover:text-fg focus-visible:bg-field focus-visible:text-fg";

/** The page header that *is* the window title bar (board 01): the page title over a one-line
 *  subtitle, the `Ctrl K` command field, the right slot (AI 润色) and, on Windows / Linux under
 *  Tauri, the window controls in the top-right corner.
 *
 *  `data-tauri-drag-region="deep"` on the root makes the whole header draggable, title and subtitle
 *  included; Tauri's injected script already excludes buttons and inputs and toggles maximize on
 *  double-click by itself, so there is deliberately no `onDoubleClick` here (a second handler
 *  would flip the window straight back). macOS keeps its native traffic lights
 *  (`titleBarStyle: "Overlay"`), so the controls are hidden there. */
export function TitleBar({
  title,
  subtitle,
  subtitleLabel,
  onSearch,
  right,
  left,
  trafficLights = false,
  platform = "unknown",
  controls = null,
  maximized = false,
  className,
}: TitleBarProps) {
  const t = useT();
  const showControls = platform !== "macos" && controls !== null;

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
        "relative flex shrink-0 items-start gap-4 pt-[38px] pr-6 select-none",
        trafficLights ? "pl-[100px]" : left !== undefined ? "pl-3" : "pl-6",
        className,
      )}>
      {left !== undefined && <div className="mt-1 shrink-0">{left}</div>}
      <div className="min-w-0 flex-1">
        <h1 className="truncate text-[26px] leading-[34px] font-bold text-fg">{title}</h1>
        {subtitle !== undefined && (
          <div
            data-testid="title-bar-subtitle"
            role={subtitleLabel === undefined ? undefined : "group"}
            aria-label={subtitleLabel}
            className="mt-1 flex min-w-0 items-center gap-1.5 text-[13px] whitespace-nowrap text-fg-muted">
            {subtitle}
          </div>
        )}
      </div>
      {onSearch && (
        <button
          type="button"
          data-testid="title-bar-search"
          aria-label={t("ui.titleBar.search")}
          title={t("ui.titleBar.search")}
          onClick={onSearch}
          className="mt-0.5 flex h-12 w-[clamp(180px,26vw,372px)] min-w-0 shrink items-center gap-2.5 rounded-pill border border-field-edge bg-field pr-3 pl-[18px] text-left text-[14px] text-fg-subtle shadow-field transition-colors hover:border-fg-subtle">
          <Icon name="search" size={18} className="shrink-0" />
          <span className="min-w-0 flex-1 truncate">{t("ui.palette.placeholder")}</span>
          <Keycaps keys="Ctrl K" />
        </button>
      )}
      {right !== undefined && (
        <div className="mt-0.5 flex h-12 shrink-0 items-center gap-3">{right}</div>
      )}
      {showControls && (
        <div
          data-testid="window-controls"
          className="absolute top-0 right-0 flex h-9 items-stretch">
          <button
            type="button"
            aria-label={t("ui.titleBar.minimize")}
            title={t("ui.titleBar.minimize")}
            onClick={controls.minimize}
            className={CONTROL}>
            <Icon name="minimize" size={14} />
          </button>
          <button
            type="button"
            data-state={maximized ? "maximized" : "normal"}
            aria-label={maximized ? t("ui.titleBar.restore") : t("ui.titleBar.maximize")}
            title={maximized ? t("ui.titleBar.restore") : t("ui.titleBar.maximize")}
            onClick={controls.toggleMaximize}
            className={CONTROL}>
            <Icon name={maximized ? "restore" : "maximize"} size={14} />
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
            <Icon name="close" size={14} />
          </button>
        </div>
      )}
    </header>
  );
}
