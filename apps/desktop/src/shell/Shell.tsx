import { formatCount, formatDuration, isBuiltinPreset, presetLabel } from "@voltip/shared";
import {
  Button,
  CommandPalette,
  cx,
  Dialog,
  floatingTheme,
  Icon,
  isThemeId,
  Lamp,
  Popover,
  type ThemeChoice,
  TitleBar,
  ToastViewport,
  useBackend,
  useHomeStats,
  useI18n,
  useNow,
  useRelayRecheck,
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
import { PresetMenu } from "../features/presets/PresetMenu";
import {
  MicrophoneMenu,
  PolishModelMenu,
  SpeechModelMenu,
} from "../features/switchers/SwitcherMenus";
import { UpdateBadge, UpdateDialog } from "../features/update/UpdateDialog";
import { pageMeta } from "./page-meta";
import { RevealSidebarButton, ShellSidebar } from "./ShellSidebar";
import { useSidebarLayout } from "./sidebar-layout";

/** The 润色 switch on the title bar: wand icon + a short text label (`AI 润色` / `AI Polish`) +
 *  lamp (user feedback 2026-09-25: the icon alone did not read as a switch). It is the real LLM
 *  pass: a click writes `settings_set_engines { refine_enabled }` with the rest of the current
 *  engine settings, and `aria-pressed` follows what the core reports back. Next to it, the current
 *  preset opens the same preset menu as the home page (docs/dictation.md §21), and the model the
 *  clean-up runs on is shown and switched in place (user request 2026-09-30, option A). */
export function PolishToggle({ className }: { className?: string }) {
  const { backend } = useBackend();
  const { t, locale } = useI18n();
  const state = useUiState();
  const on = state.engines.refine_enabled;
  const tooltip = on ? t("shell.polish.tooltipOn") : t("shell.polish.tooltipOff");
  const preset = state.settings.engines.refine_preset;
  return (
    // A pill on the header's line (Penpot 「澄空 Glass · Voltip」 board 01): the switch, a rule, the
    // preset and, where the window is wide enough, the model.
    <div
      data-testid="polish-chip"
      className={cx(
        "inline-flex h-12 items-center gap-1 rounded-pill border border-field-edge bg-field px-3 shadow-field",
        className,
      )}>
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
        className="inline-flex h-9 items-center gap-2 rounded-pill px-2 text-fg-muted transition-colors hover:bg-inset hover:text-fg">
        <Icon name="wand" size={18} className={on ? "text-primary" : "text-fg-subtle"} />
        <span
          data-testid="polish-toggle-label"
          className={cx(
            "text-[14px] font-medium whitespace-nowrap",
            on ? "text-fg" : "text-fg-muted",
          )}>
          {t("shell.polish.label")}
        </span>
        <Lamp tone={on ? "ok" : "idle"} size={6} />
      </button>
      <span aria-hidden className="mx-1 h-5 w-px bg-border" />
      <PresetMenu
        align="end"
        data-testid="polish-preset"
        triggerClassName={cx(
          "inline-flex h-9 items-center gap-1 rounded-pill px-2 text-[13px] font-medium whitespace-nowrap transition-colors hover:bg-inset hover:text-fg",
          on ? "text-fg" : "text-fg-muted",
        )}
        trigger={
          <>
            <span
              data-testid="polish-preset-name"
              {...(isBuiltinPreset(preset) ? {} : { "data-user-text": "" })}>
              {presetLabel(preset, state.presets, locale)}
            </span>
            <Icon name="chevronDown" size={12} className="text-fg-subtle" />
          </>
        }
      />
      {/* Only where the window is wide enough (xl): the menu's own `inline-flex` would win over a
          `hidden` beside it, so a wrapper hides it (1152 px window, 2026-10-11). */}
      <span className="hidden xl:contents">
        <PolishModelMenu
          align="end"
          data-testid="polish-model"
          triggerClassName={cx(
            "mono inline-flex h-9 max-w-[160px] min-w-0 items-center gap-1 rounded-pill px-2 text-[11px] transition-colors hover:bg-inset hover:text-fg",
            on ? "text-fg-muted" : "text-fg-subtle",
          )}
        />
      </span>
    </div>
  );
}

/** The floating sidebar (232, inset 16) + the page header + content on the canvas, plus the global
 *  palette, confirm dialog and toasts (Penpot 「澄空 Glass · Voltip」 board 01, every theme). The
 *  main window is frameless (tauri.conf.json `decorations: false`; macOS keeps its traffic lights
 *  via tauri.macos.conf.json), so the sidebar's brand row and the page header *are* the window
 *  title bar: drag regions, with the window controls in the top-right corner on Windows / Linux.
 *  The header carries the title over its subtitle (home: today's summary; other pages: the speech
 *  model and microphone menus, user request 2026-09-30), the `Ctrl K` command field and the AI 润色
 *  chip with its preset and model. */
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
  // The main window only (the pill shows and hides with every take).
  useRelayRecheck(backend);
  const { t } = i18n;
  const appearance = useAppearance();
  const microphone = useMicrophoneReadout();
  const chrome = useWindowChrome(state.identity?.platform);
  const dictation = useDictation();
  const sidebar = useSidebarLayout();
  // The home page's subtitle counts today's takes (the same query as the page's tiles); `useNow`
  // is in seconds.
  const today = useHomeStats(useNow() * 1000);

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

  const setTheme = useCallback(
    (
      theme: Parameters<typeof backend.invoke<"settings_set_theme">>[1]["theme"],
      followSystem: boolean,
    ) => {
      void backend.invoke("settings_set_theme", { theme, followSystem });
    },
    [backend],
  );

  const lastText = state.history_recent[0]?.text;
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
  const historyCount = state.history_total;
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
    <div className="app-canvas flex h-full min-h-0 text-fg">
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
            subtitle={
              route.name === "home" ? (
                // Board 01: today's takes and time saved, and the size of the history; 依据 says
                // how the time saved is estimated (docs/dictation.md §4.5).
                <span className="flex min-w-0 items-baseline gap-1.5">
                  <span data-testid="title-bar-summary" className="truncate">
                    {t("shell.subtitle.home", {
                      n: formatCount(today.today.count),
                      saved: formatDuration(today.today.savedMs, i18n.locale),
                      total: formatCount(state.history_total),
                    })}
                  </span>
                  <Popover
                    trigger={t("home.session.basis")}
                    label={t("home.session.basisLabel")}
                    data-testid="home-saved-basis"
                    triggerClassName="shrink-0 text-accent-text underline decoration-accent/40 underline-offset-2 hover:decoration-accent">
                    {t("home.session.basisText")}
                  </Popover>
                </span>
              ) : (
                // Every other page keeps the speech model (with its lamp) and the microphone, each
                // a menu that switches it (user request 2026-09-30).
                <span className="mono flex min-w-0 items-center gap-1.5 text-[11px]">
                  <SpeechModelMenu data-testid="title-bar-speech" />
                  <span aria-hidden>·</span>
                  <MicrophoneMenu withSource data-testid="title-bar-mic" />
                </span>
              )
            }
            subtitleLabel={route.name === "home" ? undefined : t("ui.titleBar.readout")}
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
      </div>
      <CommandPalette
        open={shell.paletteOpen}
        items={commands}
        onClose={closePalette}
        onHighlight={(item) => {
          if (item?.id.startsWith("theme-") && item.id !== "theme-system") {
            const theme = item.id.slice("theme-".length);
            if (isThemeId(theme)) appearance.preview(theme);
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
      <ToastViewport
        toasts={shell.toasts.toasts}
        onDismiss={shell.toasts.dismiss}
        theme={floatingTheme(appearance.resolvedTheme)}
      />
    </div>
  );
}
