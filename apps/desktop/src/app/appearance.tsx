import {
  type AccentId,
  type Appearance,
  type Density,
  FONT_SIZE_DEFAULT,
  FONT_SIZE_MAX,
  FONT_SIZE_MIN,
  applyAppearance,
  isAccentId,
  resolveTheme,
  systemPrefersDark,
  systemPrefersReducedMotion,
  useUiState,
} from "@voltip/ui";
import {
  type ReactNode,
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
} from "react";

/** The knobs that stay in this webview (the pill's placement is a core setting the shell reads). */
export interface LocalAppearance {
  /** 强调色 (ChatGPT's accent design, user decision 2026-09-29). */
  accent: AccentId;
  density: Density;
  fontSizePx: number;
  reduceMotion: boolean;
}

export const APPEARANCE_STORAGE_KEY = "voltip.appearance";

export const DEFAULT_LOCAL_APPEARANCE: LocalAppearance = {
  accent: "default",
  density: "default",
  fontSizePx: FONT_SIZE_DEFAULT,
  reduceMotion: false,
};

export function readLocalAppearance(
  storage: Pick<Storage, "getItem"> = window.localStorage,
): LocalAppearance {
  try {
    const raw = storage.getItem(APPEARANCE_STORAGE_KEY);
    if (!raw) return DEFAULT_LOCAL_APPEARANCE;
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null) return DEFAULT_LOCAL_APPEARANCE;
    const p: Partial<Record<keyof LocalAppearance, unknown>> = parsed;
    return {
      accent: isAccentId(p.accent) ? p.accent : "default",
      density: p.density === "compact" ? "compact" : "default",
      fontSizePx:
        typeof p.fontSizePx === "number"
          ? Math.min(FONT_SIZE_MAX, Math.max(FONT_SIZE_MIN, p.fontSizePx))
          : FONT_SIZE_DEFAULT,
      reduceMotion: p.reduceMotion === true,
    };
  } catch (_error) {
    return DEFAULT_LOCAL_APPEARANCE;
  }
}

export interface AppearanceValue {
  local: LocalAppearance;
  setLocal: (patch: Partial<LocalAppearance>) => void;
  /** Theme currently painted (follow-system already resolved). */
  resolvedTheme: Appearance["theme"];
  systemDark: boolean;
  systemReducedMotion: boolean;
  /** Temporarily paint another theme (command palette preview); `undefined` restores. */
  preview: (theme: Appearance["theme"] | undefined) => void;
}

const AppearanceContext = createContext<AppearanceValue | undefined>(undefined);

/** Owns the local knobs (density / font size / motion) and paints theme + knobs on <html>. */
export function AppearanceProvider({ children }: { children: ReactNode }) {
  const { settings } = useUiState();
  const [local, setLocalState] = useState<LocalAppearance>(() => readLocalAppearance());
  const [previewTheme, setPreviewTheme] = useState<Appearance["theme"] | undefined>(undefined);
  const systemDark = systemPrefersDark();
  const systemReducedMotion = systemPrefersReducedMotion();
  const resolvedTheme = previewTheme ?? resolveTheme(settings, systemDark);

  useEffect(() => {
    applyAppearance({
      theme: resolvedTheme,
      accent: local.accent,
      density: local.density,
      fontSizePx: local.fontSizePx,
      reduceMotion: local.reduceMotion || systemReducedMotion,
    });
  }, [resolvedTheme, local, systemReducedMotion]);

  // Another window of the app (the pill window, a second main webview) changed the knobs: follow.
  useEffect(() => {
    const onStorage = (event: StorageEvent) => {
      if (event.key === APPEARANCE_STORAGE_KEY) setLocalState(readLocalAppearance());
    };
    window.addEventListener("storage", onStorage);
    return () => {
      window.removeEventListener("storage", onStorage);
    };
  }, []);

  const setLocal = useCallback((patch: Partial<LocalAppearance>) => {
    setLocalState((prev) => {
      const next = { ...prev, ...patch };
      window.localStorage.setItem(APPEARANCE_STORAGE_KEY, JSON.stringify(next));
      return next;
    });
  }, []);

  const value = useMemo<AppearanceValue>(
    () => ({
      local,
      setLocal,
      resolvedTheme,
      systemDark,
      systemReducedMotion,
      preview: setPreviewTheme,
    }),
    [local, setLocal, resolvedTheme, systemDark, systemReducedMotion],
  );
  return <AppearanceContext.Provider value={value}>{children}</AppearanceContext.Provider>;
}

export function useAppearance(): AppearanceValue {
  const ctx = useContext(AppearanceContext);
  if (!ctx) throw new Error("useAppearance must be used inside <AppearanceProvider>");
  return ctx;
}
