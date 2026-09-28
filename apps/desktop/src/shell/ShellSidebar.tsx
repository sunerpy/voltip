import {
  IconButton,
  Sidebar,
  SidebarEntry,
  type SidebarGroup,
  ThemeSwitch,
  type ThemeChoice,
  useI18n,
  useUiState,
} from "@voltip/ui";
import type { TFunction } from "@voltip/shared";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { AI_ROUTE, type Route, SPEECH_ROUTE, isDialogRoute } from "../app/router";
import type { SidebarLayoutControls } from "./sidebar-layout";

/** A sidebar count, shown once there is something to count. */
function count(n: number): { count?: number } {
  return n > 0 ? { count: n } : {};
}

/** Sidebar groups: 工作台 (the content) and 语音输入 (where the voice comes from and what turns it
 *  into text). The history, dictionary and rule counts are the core's lists, never a fixture. */
export function navGroups(
  counts: { history: number; dictionary: number; rules: number },
  t: TFunction,
): SidebarGroup[] {
  return [
    {
      title: t("shell.nav.workbench"),
      items: [
        { id: "home", label: t("shell.nav.home"), icon: "home" },
        { id: "history", label: t("shell.nav.history"), icon: "history", ...count(counts.history) },
        {
          id: "dictionary",
          label: t("shell.nav.dictionary"),
          icon: "book",
          ...count(counts.dictionary),
        },
        { id: "rules", label: t("shell.nav.rules"), icon: "sparkles", ...count(counts.rules) },
      ],
    },
    {
      title: t("shell.nav.voice"),
      items: [
        { id: "speech", label: t("shell.nav.speech"), icon: "wave" },
        { id: "ai", label: t("shell.nav.ai"), icon: "wand" },
        { id: "devices", label: t("shell.nav.devices"), icon: "phone" },
      ],
    },
  ];
}

export const ALL_NAV_IDS = ["home", "history", "dictionary", "rules", "speech", "ai", "devices"];

/** The sidebar entry a route lights up: every entry but 设置 and 反馈 is a page of the main layout
 *  (user feedback and decision 2026-09-28), so a page lights up its own entry; 设置 and 反馈 are
 *  dialogs over the page beneath, which keeps its entry lit. */
export function navIdFor(route: Route): string {
  if (route.name === "onboarding" || route.name === "overlay" || route.name === "notfound")
    return "home";
  return route.name;
}

export function routeForNav(id: string): Route {
  switch (id) {
    case "history":
    case "dictionary":
    case "rules":
    case "devices":
      return { name: id };
    case "speech":
      return SPEECH_ROUTE;
    case "ai":
      return AI_ROUTE;
    default:
      return { name: "home" };
  }
}

export interface ShellSidebarProps {
  route: Route;
  /** The page a settings route floats over. */
  background: Route;
  navigate: (route: Route) => void;
  sidebar: SidebarLayoutControls;
  onTheme: (choice: ThemeChoice) => void;
  /** The setup guide: only 首页 stays live. */
  onboarding: boolean;
  trafficLights: boolean;
}

/** The desktop's sidebar in its three layouts (`sidebar-layout.ts`): docked with labels, docked
 *  as the icon rail, or hidden, when a strip on the window's left edge opens it as a floating
 *  preview. The preview closes when the pointer moves past its right edge: `mouseleave` cannot do
 *  it, since the panel appears under a pointer that has not moved and no `mouseenter` ever fired. */
export function ShellSidebar({
  route,
  background,
  navigate,
  sidebar,
  onTheme,
  onboarding,
  trafficLights,
}: ShellSidebarProps) {
  const { t } = useI18n();
  const state = useUiState();
  const { layout, toggleCollapsed, toggleHidden, reveal } = sidebar;
  const [preview, setPreview] = useState(false);
  const panel = useRef<HTMLDivElement>(null);
  const previewing = layout.hidden && preview;

  useEffect(() => {
    if (!previewing) return;
    const onMove = (e: PointerEvent) => {
      const right = panel.current?.getBoundingClientRect().right ?? 0;
      if (e.clientX > right + 8) setPreview(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setPreview(false);
    };
    document.addEventListener("pointermove", onMove);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("pointermove", onMove);
      document.removeEventListener("keydown", onKey);
    };
  }, [previewing]);

  const collapsed = layout.collapsed;
  const collapseControl = (
    <IconButton
      icon={collapsed ? "railExpand" : "railCollapse"}
      label={collapsed ? t("ui.sidebar.expand") : t("ui.sidebar.collapse")}
      size={28}
      aria-pressed={collapsed}
      data-testid="sidebar-collapse"
      onClick={toggleCollapsed}
    />
  );
  const settings = state.settings;
  // A dialog route floats over the page beneath (`background`), whose entry stays lit.
  const activeId = navIdFor(isDialogRoute(route) ? background : route);
  const rail = (floating: boolean): ReactNode => (
    <Sidebar
      groups={navGroups(
        {
          history: state.history.length,
          dictionary: state.dictionary.length,
          rules: state.rules.length,
        },
        t,
      )}
      activeId={activeId}
      disabledIds={onboarding ? ALL_NAV_IDS.filter((id) => id !== "home") : []}
      statusTone={state.identity ? "ok" : "idle"}
      trafficLights={trafficLights}
      collapsed={collapsed}
      floating={floating}
      onNavigate={(id) => {
        navigate(routeForNav(id));
        setPreview(false);
      }}
      controls={
        <>
          {collapseControl}
          {floating ? (
            <IconButton
              icon="pin"
              label={t("ui.sidebar.pin")}
              size={28}
              data-testid="sidebar-pin"
              onClick={() => {
                setPreview(false);
                reveal();
              }}
            />
          ) : (
            <IconButton
              icon="sidebarHide"
              label={`${t("ui.sidebar.hide")} · Ctrl B`}
              size={28}
              data-testid="sidebar-hide"
              onClick={toggleHidden}
            />
          )}
        </>
      }
      footer={
        <>
          <SidebarEntry
            icon="chat"
            label={t("shell.nav.feedback")}
            collapsed={collapsed}
            opensDialog
            disabled={onboarding}
            data-testid="sidebar-feedback"
            onClick={() => {
              navigate({ name: "feedback" });
              setPreview(false);
            }}
          />
          <ThemeSwitch
            value={settings.follow_system_theme ? "system" : settings.theme}
            collapsed={collapsed}
            onChange={onTheme}
          />
          <SidebarEntry
            icon="settings"
            label={t("shell.nav.settings")}
            collapsed={collapsed}
            opensDialog
            disabled={onboarding}
            data-testid="sidebar-settings"
            onClick={() => {
              navigate({ name: "settings", section: "general" });
              setPreview(false);
            }}
          />
        </>
      }
    />
  );

  if (!layout.hidden) return rail(false);
  return (
    <>
      <div
        aria-hidden
        data-testid="sidebar-edge"
        onPointerEnter={() => {
          setPreview(true);
        }}
        className="fixed inset-y-0 left-0 z-40 w-2.5 border-r-2 border-border hover:border-accent"
      />
      {previewing && (
        <div ref={panel} data-testid="sidebar-preview" className="fixed inset-y-0 left-0 z-40">
          {rail(true)}
        </div>
      )}
    </>
  );
}

/** The title bar's way back from a hidden sidebar (keyboard included: the edge strip is a pointer
 *  affordance only). */
export function RevealSidebarButton({ onReveal }: { onReveal: () => void }) {
  const { t } = useI18n();
  return (
    <IconButton
      icon="railExpand"
      label={`${t("ui.sidebar.show")} · Ctrl B`}
      size={28}
      data-testid="sidebar-reveal"
      onClick={onReveal}
    />
  );
}
