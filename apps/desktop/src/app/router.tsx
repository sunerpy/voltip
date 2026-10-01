import {
  type ReactNode,
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
} from "react";

export type SettingsSection =
  | "general"
  | "hotkey"
  | "dictation"
  | "microphone"
  | "scene"
  | "privacy"
  | "appearance"
  | "about";
export const SETTINGS_SECTIONS: readonly SettingsSection[] = [
  "general",
  "hotkey",
  "dictation",
  "microphone",
  "scene",
  "privacy",
  "appearance",
  "about",
];

/** The 语音模型 page (recognition providers, local models, recognition options). It was a group of
 *  the settings dialog until 2026-09-28: every sidebar entry but 设置 is a page of the main layout. */
export const SPEECH_ROUTE: Route = { name: "speech" };
/** The AI 模型 page: the LLM providers behind the clean-up and voice edit. */
export const AI_ROUTE: Route = { name: "ai" };

export type Route =
  | { name: "home" }
  | { name: "history"; filter?: string }
  | { name: "dictionary" }
  /** `compose`: open the editor on a new rule (the palette's 新建规则), then drop the flag. */
  | { name: "rules"; compose?: true }
  | { name: "devices" }
  | { name: "speech" }
  /** `section`: scroll to that part of the page (the preset menus' 管理预设…). */
  | { name: "ai"; section?: "presets" }
  | { name: "feedback" }
  | { name: "settings"; section: SettingsSection }
  | { name: "onboarding"; step: number }
  | { name: "overlay"; state?: string }
  | { name: "notfound"; path: string };

export function isSettingsSection(value: string): value is SettingsSection {
  return (SETTINGS_SECTIONS as readonly string[]).includes(value);
}

/** `/settings/appearance?x=1` or `#/overlay?state=listening` → Route. `/engines`,
 *  `/settings/engine` and `/settings/speech` (the former engines page and settings groups) land on
 *  the 语音模型 page, `/settings/refine` and `/settings/ai` on AI 模型, so old links keep working. */
export function parseRoute(path: string): Route {
  const clean = path.startsWith("#") ? path.slice(1) : path;
  const [pathname = "/", query = ""] = clean.split("?");
  const params = new URLSearchParams(query);
  const segments = pathname.split("/").filter((s) => s.length > 0);
  const [head, second] = segments;
  switch (head) {
    case undefined:
      return { name: "home" };
    case "history": {
      const filter = params.get("filter");
      return filter === null ? { name: "history" } : { name: "history", filter };
    }
    case "dictionary":
      return { name: "dictionary" };
    case "rules":
      return params.get("new") === "1" ? { name: "rules", compose: true } : { name: "rules" };
    case "engines":
    case "speech":
      return SPEECH_ROUTE;
    case "ai":
      return second === "presets" ? { name: "ai", section: "presets" } : AI_ROUTE;
    case "feedback":
      return { name: "feedback" };
    case "devices":
      return { name: "devices" };
    case "settings":
      if (second === "engine" || second === "speech") return SPEECH_ROUTE;
      if (second === "refine" || second === "ai") return AI_ROUTE;
      return {
        name: "settings",
        section: second !== undefined && isSettingsSection(second) ? second : "appearance",
      };
    case "onboarding": {
      const step = Number.parseInt(params.get("step") ?? "1", 10);
      return {
        name: "onboarding",
        step: Number.isFinite(step) && step >= 1 && step <= 4 ? step : 1,
      };
    }
    case "overlay": {
      const state = params.get("state");
      return state === null ? { name: "overlay" } : { name: "overlay", state };
    }
    default:
      return { name: "notfound", path: pathname };
  }
}

export function routePath(route: Route): string {
  switch (route.name) {
    case "home":
      return "/";
    case "history":
      return route.filter === undefined
        ? "/history"
        : `/history?filter=${encodeURIComponent(route.filter)}`;
    case "settings":
      return `/settings/${route.section}`;
    case "onboarding":
      return route.step === 1 ? "/onboarding" : `/onboarding?step=${route.step}`;
    case "overlay":
      return route.state === undefined
        ? "/overlay"
        : `/overlay?state=${encodeURIComponent(route.state)}`;
    case "notfound":
      return route.path;
    case "rules":
      return route.compose === true ? "/rules?new=1" : "/rules";
    case "ai":
      return route.section === undefined ? "/ai" : `/ai/${route.section}`;
    case "dictionary":
    case "devices":
    case "speech":
    case "feedback":
      return `/${route.name}`;
  }
}

/** A page the settings and feedback dialogs can float over: never a dialog itself, nor a
 *  chrome-less route. */
export type BackgroundRoute = Exclude<
  Route,
  { name: "settings" | "feedback" | "onboarding" | "overlay" | "notfound" }
>;

/** The routes that are modal dialogs over `background` (user decision 2026-09-28: 设置 and 反馈
 *  stay dialogs; every other sidebar entry is a page). */
export type DialogRoute = Extract<Route, { name: "settings" | "feedback" }>;

export function isDialogRoute(route: Route): route is DialogRoute {
  return route.name === "settings" || route.name === "feedback";
}

export const HOME_ROUTE: BackgroundRoute = { name: "home" };

export function isBackgroundRoute(route: Route): route is BackgroundRoute {
  return (
    !isDialogRoute(route) &&
    route.name !== "onboarding" &&
    route.name !== "overlay" &&
    route.name !== "notfound"
  );
}

export interface RouterValue {
  path: string;
  route: Route;
  /** The last page that was not a dialog; `home` until one has been visited. */
  background: BackgroundRoute;
  /** `replace` swaps the current history entry instead of pushing one (a one-shot flag). */
  navigate: (to: string | Route, options?: { replace?: boolean }) => void;
}

const RouterContext = createContext<RouterValue | undefined>(undefined);

/** Reads `#/path` first (overlay window), then `pathname` (SPA fallback serves index.html). */
export function currentLocationPath(
  loc: Pick<Location, "hash" | "pathname" | "search"> = window.location,
): string {
  if (loc.hash.startsWith("#/")) return loc.hash.slice(1);
  return `${loc.pathname}${loc.search}`;
}

export interface RouterProviderProps {
  children: ReactNode;
  /** Start here and never touch `window.history` (tests, storybook-style previews). */
  initialPath?: string;
}

interface NavState {
  path: string;
  background: BackgroundRoute;
}

function navStateFor(path: string, previous: BackgroundRoute): NavState {
  const route = parseRoute(path);
  return { path, background: isBackgroundRoute(route) ? route : previous };
}

export function RouterProvider({ children, initialPath }: RouterProviderProps) {
  const memory = initialPath !== undefined;
  const [nav, setNav] = useState<NavState>(() =>
    navStateFor(initialPath ?? currentLocationPath(), HOME_ROUTE),
  );
  const { path, background } = nav;
  const setPath = useCallback((next: string) => {
    setNav((prev) => (prev.path === next ? prev : navStateFor(next, prev.background)));
  }, []);

  useEffect(() => {
    if (memory) return;
    const onPop = () => {
      setPath(currentLocationPath());
    };
    window.addEventListener("popstate", onPop);
    window.addEventListener("hashchange", onPop);
    return () => {
      window.removeEventListener("popstate", onPop);
      window.removeEventListener("hashchange", onPop);
    };
  }, [memory, setPath]);

  const navigate = useCallback(
    (to: string | Route, options?: { replace?: boolean }) => {
      const next = typeof to === "string" ? to : routePath(to);
      setPath(next);
      if (!memory) {
        const hash = window.location.hash.startsWith("#/");
        if (options?.replace === true) {
          if (hash) window.location.replace(`#${next}`);
          else window.history.replaceState(null, "", next);
        } else if (hash) window.location.hash = `#${next}`;
        else window.history.pushState(null, "", next);
      }
    },
    [memory, setPath],
  );

  const value = useMemo<RouterValue>(
    () => ({ path, route: parseRoute(path), background, navigate }),
    [path, background, navigate],
  );
  return <RouterContext.Provider value={value}>{children}</RouterContext.Provider>;
}

export function useRouter(): RouterValue {
  const ctx = useContext(RouterContext);
  if (!ctx) throw new Error("useRouter must be used inside <RouterProvider>");
  return ctx;
}
