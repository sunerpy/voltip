import {
  Button,
  CommandPalette,
  Dialog,
  Icon,
  Keycaps,
  Lamp,
  type ThemeChoice,
  TitleBar,
  ToastViewport,
  cx,
  useBackend,
  useI18n,
  useUiState,
} from "@voltip/ui";
import { type ReactNode, useCallback, useEffect, useMemo } from "react";
import { useAppearance } from "../app/appearance";
import { buildCommands } from "../app/commands";
import { useRouter } from "../app/router";
import { copyWithToast, useShell } from "../app/shell-context";
import { type TrayRequest, type TrayRequestSource, useTrayRequests } from "../app/tray-requests";
import { useWindowChrome } from "../app/window";
import { microphoneReadoutValue, useMicrophoneReadout } from "../features/audio/mic-store";
import { useDictation } from "../features/dictation/useDictation";
import { UpdateBadge, UpdateDialog } from "../features/update/UpdateDialog";
import { engineReadout, microphoneReadout, pageMeta } from "./page-meta";
import { RevealSidebarButton, ShellSidebar } from "./ShellSidebar";
import { useSidebarLayout } from "./sidebar-layout";

/** The 润色 switch on the title bar: wand icon + a short text label (`AI 润色` / `AI Polish`) +
 *  lamp (user feedback 2026-09-25: the icon alone did not read as a switch). It is the real LLM
 *  pass: a click writes `settings_set_engines { refine_enabled }` with the rest of the current
 *  engine settings, and `aria-pressed` follows what the core reports back. */
export function PolishToggle({ className }: { className?: string }) {
  const { backend } = useBackend();
  const { t } = useI18n();
  const state = useUiState();
  const on = state.engines.refine_enabled;
  const tooltip = on ? t("shell.polish.tooltipOn") : t("shell.polish.tooltipOff");
  return (
    <button
      type="button"
      data-testid="polish-toggle"
      aria-label={t("shell.polish.aria")}
      aria-pressed={on}
      title={tooltip}
      onClick={() => {
        void backend.invoke("settings_set_engines", {
          engines: { ...state.settings.engines, refine_enabled: !on },
        });
      }}
      className={cx(
        "inline-flex h-7 items-center gap-1.5 rounded-6 px-1.5 text-fg-muted transition-colors hover:bg-inset hover:text-fg",
        className,
      )}>
      <Icon name="wand" size={16} className={on ? "text-accent-text" : "text-fg-subtle"} />
      <span
        data-testid="polish-toggle-label"
        className={cx("text-[12px] whitespace-nowrap", on ? "text-fg" : "text-fg-muted")}>
        {t("shell.polish.label")}
      </span>
      <Lamp tone={on ? "ok" : "idle"} size={6} />
    </button>
  );
}

export function FooterShortcuts({ items }: { items: readonly (readonly [string, string])[] }) {
  return (
    <footer className="mono flex h-7 shrink-0 items-center gap-1 overflow-hidden border-t border-border px-6 text-[11px] text-fg-subtle whitespace-nowrap">
      {items.map(([keys, label], i) => (
        <span key={`${keys}-${label}`} className="flex items-center gap-1.5">
          {i > 0 && <span className="px-1">·</span>}
          <Keycaps keys={keys} />
          <span>{label}</span>
        </span>
      ))}
    </footer>
  );
}

/** Sidebar 224 + title bar 40 + content, plus the global palette, confirm dialog and toasts. The
 *  main window is frameless (tauri.conf.json `decorations: false`; macOS keeps its traffic lights
 *  via tauri.macos.conf.json), so the 40 px strip formed by the sidebar brand row and the title
 *  bar *is* the window title bar: one continuous drag region with the window controls at the far
 *  right on Windows / Linux. The title bar carries the title, the compact engine · microphone
 *  readout right after it, the search icon and the AI 润色 toggle; there is no second header row. */
export function Shell({
  children,
  traySource,
}: {
  children: ReactNode;
  /** The tray's requests (tests inject one; the real app listens to the shell's event). */
  traySource?: TrayRequestSource;
}) {
  const { route, background, navigate } = useRouter();
  const state = useUiState();
  const { backend } = useBackend();
  const shell = useShell();
  const i18n = useI18n();
  const { t } = i18n;
  const appearance = useAppearance();
  const microphone = useMicrophoneReadout();
  const chrome = useWindowChrome(state.identity?.platform);
  const dictation = useDictation();
  const sidebar = useSidebarLayout();

  const meta = useMemo(
    () =>
      pageMeta(
        route,
        state,
        { microphone: microphoneReadoutValue(microphone, t) },
        background,
        i18n,
      ),
    [route, background, state, microphone, i18n, t],
  );
  // The compact title-bar readout: the default engine (with its token lamp) and the microphone.
  const barReadouts = useMemo(
    () => [
      engineReadout(state.engines, i18n),
      microphoneReadout(microphoneReadoutValue(microphone, t), i18n),
    ],
    [state.engines, microphone, i18n, t],
  );

  const setTheme = useCallback(
    (
      theme: Parameters<typeof backend.invoke<"settings_set_theme">>[1]["theme"],
      followSystem: boolean,
    ) => {
      void backend.invoke("settings_set_theme", { theme, followSystem });
    },
    [backend],
  );

  const lastText = state.history[0]?.text;
  const copyLast = useCallback(() => {
    if (lastText === undefined) {
      shell.toast({ message: t("shell.toast.historyEmpty"), duration: 3000, tone: "danger" });
      return;
    }
    void copyWithToast(
      shell,
      lastText,
      t("shell.toast.copiedLast", { n: Array.from(lastText).length }),
    );
  }, [lastText, shell, t]);
  const historyCount = state.history.length;
  const clearHistory = useCallback(() => {
    shell.confirm({
      title: t("shell.confirm.clearTitle", { n: historyCount }),
      body: t("shell.confirm.clearBody"),
      confirmLabel: t("shell.confirm.clearConfirm"),
      tone: "danger",
      onConfirm: () => {
        void backend.invoke("history_clear");
      },
    });
  }, [shell, historyCount, backend, t]);

  const commands = useMemo(
    () =>
      buildCommands({
        state,
        resolvedTheme: appearance.resolvedTheme,
        systemDark: appearance.systemDark,
        navigate,
        setTheme,
        toast: (message) => shell.toast({ message, duration: 5000 }),
        dictate: dictation.toggle,
        copyLast,
        clearHistory,
        i18n,
      }),
    [
      state,
      appearance.resolvedTheme,
      appearance.systemDark,
      navigate,
      setTheme,
      shell,
      dictation.toggle,
      copyLast,
      clearHistory,
      i18n,
    ],
  );

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey)) return;
      const key = e.key.toLowerCase();
      if (key === "k") {
        e.preventDefault();
        shell.setPaletteOpen(!shell.paletteOpen);
      } else if (key === ",") {
        e.preventDefault();
        navigate({ name: "settings", section: "appearance" });
      } else if (key === "h" && !e.shiftKey) {
        e.preventDefault();
        navigate({ name: "history" });
      } else if (key === "b" && !e.shiftKey) {
        e.preventDefault();
        sidebar.toggleHidden();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  }, [shell, navigate, sidebar]);

  // The tray's 设置… and 检查更新… (the shell has already brought the window up).
  const updateState = state.update.state;
  const onTray = useCallback(
    (request: TrayRequest) => {
      if (request === "settings") {
        navigate({ name: "settings", section: "general" });
        return;
      }
      shell.setUpdateOpen(true);
      if (updateState === "idle" || updateState === "up_to_date" || updateState === "failed")
        void backend.invoke("update_check");
    },
    [navigate, shell, updateState, backend],
  );
  useTrayRequests(onTray, traySource);

  const onboarding = route.name === "onboarding";
  // The overlay showcase is a chrome-less spec sheet of the pill.
  const sheet = route.name === "overlay";
  const pickTheme = useCallback(
    (choice: ThemeChoice) => {
      if (choice === "system") setTheme(state.settings.theme, true);
      else setTheme(choice, false);
    },
    [setTheme, state.settings.theme],
  );
  const closePalette = useCallback(() => {
    shell.setPaletteOpen(false);
    appearance.preview(undefined);
  }, [shell, appearance]);

  return (
    <div className="flex h-full min-h-0 bg-canvas text-fg">
      {!sheet && (
        <ShellSidebar
          route={route}
          background={background}
          navigate={navigate}
          sidebar={sidebar}
          onboarding={onboarding}
          trafficLights={chrome.platform === "macos"}
          onTheme={pickTheme}
        />
      )}
      <div className="flex min-w-0 flex-1 flex-col">
        {sheet ? (
          // is a chrome-less spec sheet: the showcase gets the full width and a way back,
          // but the strip is still the window's title bar (drag region + window controls).
          <TitleBar
            title={meta.title}
            trafficLights={chrome.platform === "macos"}
            platform={chrome.platform}
            controls={chrome.controls}
            maximized={chrome.maximized}
            right={
              <Button
                size="sm"
                variant="ghost"
                icon="home"
                onClick={() => {
                  navigate({ name: "home" });
                }}>
                {t("shell.backHome")}
              </Button>
            }
          />
        ) : (
          <TitleBar
            title={meta.title}
            readouts={barReadouts}
            left={
              sidebar.layout.hidden ? <RevealSidebarButton onReveal={sidebar.reveal} /> : undefined
            }
            trafficLights={sidebar.layout.hidden && chrome.platform === "macos"}
            onSearch={() => {
              shell.setPaletteOpen(true);
            }}
            platform={chrome.platform}
            controls={chrome.controls}
            maximized={chrome.maximized}
            right={
              <>
                <UpdateBadge
                  onOpen={() => {
                    shell.setUpdateOpen(true);
                  }}
                />
                <PolishToggle />
              </>
            }
          />
        )}
        <main className="min-h-0 flex-1 overflow-auto">{children}</main>
        {!sheet && (
          <FooterShortcuts items={[["Ctrl K", t("shell.footer.command")], ...meta.shortcuts]} />
        )}
      </div>
      <CommandPalette
        open={shell.paletteOpen}
        items={commands}
        onClose={closePalette}
        onHighlight={(item) => {
          if (item?.id.startsWith("theme-") && item.id !== "theme-system") {
            const theme = item.id.slice("theme-".length);
            if (theme === "light" || theme === "dark" || theme === "warm" || theme === "graphite")
              appearance.preview(theme);
          } else appearance.preview(undefined);
        }}
      />
      <Dialog
        open={shell.pending !== undefined}
        title={shell.pending?.title ?? ""}
        facts={shell.pending?.facts}
        onClose={shell.closeConfirm}
        actions={
          <>
            <Button size="sm" variant="ghost" onClick={shell.closeConfirm} data-autofocus>
              {t("common.cancel")}
            </Button>
            <Button
              size="sm"
              variant={shell.pending?.tone === "primary" ? "primary" : "danger"}
              onClick={() => {
                shell.pending?.onConfirm();
                shell.closeConfirm();
              }}>
              {shell.pending?.confirmLabel}
            </Button>
          </>
        }>
        {shell.pending?.body}
      </Dialog>
      <UpdateDialog
        open={shell.updateOpen}
        onClose={() => {
          shell.setUpdateOpen(false);
        }}
      />
      <ToastViewport toasts={shell.toasts.toasts} onDismiss={shell.toasts.dismiss} />
    </div>
  );
}
