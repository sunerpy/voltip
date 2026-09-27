import { THEME_IDS, type ThemeId, type Translator, type UiState, themeName } from "@voltip/shared";
import type { CommandItem } from "@voltip/ui";
import { AI_ROUTE, SPEECH_ROUTE, type Route } from "./router";

export interface CommandDeps {
  state: UiState;
  resolvedTheme: ThemeId;
  systemDark: boolean;
  navigate: (route: Route) => void;
  setTheme: (theme: ThemeId, followSystem: boolean) => void;
  toast: (message: string) => void;
  /** Start dictation, or stop it while listening (`dictation_start` / `dictation_stop`). */
  dictate: () => void;
  /** Copy `state.history[0].text` to the clipboard (with the honest copy toast). */
  copyLast: () => void;
  /** Ask, then `history_clear`. */
  clearHistory: () => void;
  /** The mounted locale's translator. */
  i18n: Translator;
}

/** Ctrl K command list: themes first, then actions and navigation. Every action is real:
 *  dictation, the last result and the history live in the core. */
export function buildCommands(deps: CommandDeps): CommandItem[] {
  const { state, navigate, setTheme, toast, dictate, copyLast, clearHistory, i18n } = deps;
  const { t, locale } = i18n;
  const phase = state.dictation.phase.phase;
  const listening = phase === "listening";
  const processing = phase === "processing";
  const themes: CommandItem[] = THEME_IDS.map((id) => ({
    id: `theme-${id}`,
    group: t("commands.group.theme"),
    label: t("commands.theme", { theme: themeName(id, locale) }),
    icon: "settings",
    hint:
      !state.settings.follow_system_theme && state.settings.theme === id
        ? t("commands.current")
        : undefined,
    run: () => {
      setTheme(id, false);
      toast(t("commands.themeSwitched", { theme: themeName(id, locale) }));
    },
  }));
  const historyEmpty = state.history.length === 0;
  return [
    ...themes,
    {
      id: "theme-system",
      group: t("commands.group.theme"),
      label: t("commands.followSystem"),
      icon: "monitor",
      hint: `${t("commands.systemHint", { theme: themeName(deps.systemDark ? "dark" : "light", locale) })}${state.settings.follow_system_theme ? t("commands.currentSuffix") : ""}`,
      run: () => {
        setTheme(state.settings.theme, true);
        toast(t("commands.followSystemToast"));
      },
    },
    {
      id: "dictate",
      group: t("commands.group.action"),
      label: listening ? t("commands.stopDictation") : t("commands.startDictation"),
      icon: listening ? "stop" : "mic",
      keys: state.settings.hotkey.replaceAll("+", " "),
      hint: processing ? t("commands.processing") : undefined,
      disabled: processing,
      disabledHint: processing ? t("commands.processingHint") : undefined,
      run: dictate,
    },
    {
      id: "new-rule",
      group: t("commands.group.action"),
      label: t("commands.newRule"),
      icon: "sparkles",
      keys: "Ctrl N",
      run: () => {
        navigate({ name: "rules", compose: true });
      },
    },
    {
      id: "copy-last",
      group: t("commands.group.action"),
      label: t("commands.copyLast"),
      icon: "copy",
      hint: state.history[0]
        ? t("commands.charsHint", { n: Array.from(state.history[0].text).length })
        : t("commands.historyEmptyShort"),
      disabled: historyEmpty,
      disabledHint: historyEmpty ? t("commands.historyEmpty") : undefined,
      run: copyLast,
    },
    {
      id: "clear-history",
      group: t("commands.group.action"),
      label: t("commands.clearHistory"),
      icon: "trash",
      hint: t("commands.entriesHint", { n: state.history.length }),
      disabled: historyEmpty,
      disabledHint: historyEmpty ? t("commands.historyEmpty") : undefined,
      run: clearHistory,
    },
    {
      id: "nav-history",
      group: t("commands.group.nav"),
      label: t("commands.openHistory"),
      icon: "history",
      keys: "Ctrl H",
      run: () => {
        navigate({ name: "history" });
      },
    },
    {
      id: "nav-appearance",
      group: t("commands.group.nav"),
      label: t("commands.openAppearance"),
      icon: "settings",
      keys: "Ctrl ,",
      run: () => {
        navigate({ name: "settings", section: "appearance" });
      },
    },
    {
      id: "nav-engines",
      group: t("commands.group.nav"),
      label: t("commands.openEngines"),
      icon: "wave",
      run: () => {
        navigate(SPEECH_ROUTE);
      },
    },
    {
      id: "nav-ai",
      group: t("commands.group.nav"),
      label: t("commands.openAi"),
      icon: "wand",
      run: () => {
        navigate(AI_ROUTE);
      },
    },
    {
      id: "nav-devices",
      group: t("commands.group.nav"),
      label: t("commands.openDevices"),
      icon: "phone",
      run: () => {
        navigate({ name: "devices" });
      },
    },
  ];
}
