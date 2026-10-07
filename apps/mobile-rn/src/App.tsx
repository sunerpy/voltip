// The app around the screens (docs/mobile-rn.md §5): the backend's state, the language and the
// theme the core's settings name, the snackbar and confirmation dialog, and the navigation. Pairing
// events move the screens as in apps/mobile: a finished handshake opens 核对安全码, a trusted
// computer goes back to 说话 (and the session is reset).
import {
  NavigationContainer,
  DarkTheme as NavigationDark,
  DefaultTheme as NavigationLight,
  type Theme as NavigationTheme,
  createNavigationContainerRef,
} from "@react-navigation/native";
import { type Backend, type UiEvent, coreMessageText, resolveLocale } from "@voltip/shared";
import { getLocales } from "expo-localization";
import { NavigationBar } from "expo-navigation-bar";
import { StatusBar } from "expo-status-bar";
import { type ReactNode, useCallback, useEffect, useMemo, useRef } from "react";
import { ActivityIndicator, View, useColorScheme } from "react-native";
import { MaterialCommunityIcons } from "@expo/vector-icons";
import { PaperProvider, Text } from "react-native-paper";
import {
  type Metrics,
  SafeAreaProvider,
  initialWindowMetrics,
  useSafeAreaInsets,
} from "react-native-safe-area-context";

import { BackendProvider, useBackend, useUiState } from "./backend/BackendProvider";
import { I18nProvider, useT } from "./backend/i18n";
import { RootNavigator } from "./navigation";
import type { RootParams } from "./routes";
import { ShellProvider, useShell } from "./shell";
import { type AppTheme, appTheme, themeId, themeSeed } from "./theme/themes";
import { Logo } from "./ui/Logo";

export const navigation = createNavigationContainerRef<RootParams>();

/** What `settings.locale = "system"` follows: the phone's first language. */
function systemLanguage(): string {
  return getLocales()[0]?.languageTag ?? "zh-CN";
}

function Localized({ children, language }: { children: ReactNode; language: string }) {
  const { settings } = useUiState();
  return <I18nProvider locale={resolveLocale(settings.locale, language)}>{children}</I18nProvider>;
}

function navigationTheme(theme: AppTheme): NavigationTheme {
  const base = theme.dark ? NavigationDark : NavigationLight;
  return {
    ...base,
    colors: {
      ...base.colors,
      primary: theme.colors.primary,
      background: theme.colors.background,
      card: theme.colors.surface,
      text: theme.colors.onSurface,
      border: theme.colors.outlineVariant,
      notification: theme.colors.error,
    },
  };
}

/** The bottom of the snackbar: above the navigation bar of the tabs. */
const TAB_BAR_HEIGHT = 80;

function Themed({
  accent,
  children,
}: {
  accent: string | null;
  children: (theme: AppTheme) => ReactNode;
}) {
  const { settings } = useUiState();
  const dark = useColorScheme() === "dark";
  const theme = useMemo(
    () => appTheme(themeId(settings, dark), themeSeed(settings, accent)),
    [settings, dark, accent],
  );
  return (
    // The icons from the app's own copy of the set: under pnpm, Paper's optional lookup of
    // `@expo/vector-icons` from its own directory finds nothing.
    <PaperProvider
      theme={theme}
      settings={{ icon: (props) => <MaterialCommunityIcons {...props} /> }}>
      <StatusBar style={theme.dark ? "light" : "dark"} />
      <NavigationBar style={theme.dark ? "light" : "dark"} />
      {children(theme)}
    </PaperProvider>
  );
}

/** Toasts for the core's notes, and the screens pairing moves to. */
function useCoreEvents(register: (fn: (e: UiEvent) => void) => void) {
  const { backend } = useBackend();
  const shell = useShell();
  const t = useT();
  useEffect(() => {
    register((event) => {
      if (event.type === "error")
        shell.toast(t("mobile.toast.error", { message: coreMessageText(event.message) }), "danger");
      else if (event.type === "trusted")
        shell.toast(t("mobile.toast.trusted", { name: event.name }));
      else if (event.type === "unpaired")
        shell.toast(t("mobile.toast.unpaired", { name: event.name }));
      else if (event.type === "message")
        shell.toast(t("mobile.toast.message", { body: event.body }));
      else if (event.type === "identity_changed")
        shell.toast(t("mobile.toast.identityChanged", { name: event.previous.name }), "danger");
      else if (event.type === "pairing" && navigation.isReady()) {
        if (event.state.state === "awaiting_verification") navigation.navigate("Verify");
        // On 核对安全码 the page itself goes once it has seen the trusted phase (Verify.tsx).
        else if (
          event.state.state === "trusted" &&
          navigation.getCurrentRoute()?.name !== "Verify"
        ) {
          navigation.navigate("Tabs", { screen: "Talk" });
          void backend.invoke("pairing_reset");
        }
      }
    });
  }, [register, shell, t, backend]);
}

/** A splash until `core_state` resolved, so the first screen comes from real data. */
function Gate({ register }: { register: (fn: (e: UiEvent) => void) => void }) {
  const { state, error } = useBackend();
  const t = useT();
  useCoreEvents(register);
  if (state === undefined) {
    return (
      <View
        style={{ flex: 1, alignItems: "center", justifyContent: "center", gap: 16, padding: 24 }}
        accessibilityRole="progressbar">
        <Logo size={48} />
        {error === undefined ? <ActivityIndicator /> : null}
        <Text variant="bodyMedium">
          {error === undefined ? t("mobile.connecting") : t("mobile.connectFailed", { error })}
        </Text>
      </View>
    );
  }
  return (
    <RootNavigator
      initial={state.pairing.state.state === "awaiting_verification" ? "Verify" : "Tabs"}
    />
  );
}

function Shell({
  register,
  theme,
}: {
  register: (fn: (e: UiEvent) => void) => void;
  theme: AppTheme;
}) {
  const insets = useSafeAreaInsets();
  return (
    <ShellProvider bottomInset={insets.bottom + TAB_BAR_HEIGHT}>
      <NavigationContainer ref={navigation} theme={navigationTheme(theme)}>
        <Gate register={register} />
      </NavigationContainer>
    </ShellProvider>
  );
}

export function App({
  backend,
  language = systemLanguage(),
  metrics = initialWindowMetrics,
  accent = null,
}: {
  backend: Backend;
  /** What `settings.locale = "system"` follows (tests pass one; the app reads the phone's). */
  language?: string;
  /** The wallpaper's colour (Android 12+), which the colours follow while the appearance follows
   *  the system; `null` keeps Voltip's own. */
  accent?: string | null;
  /** The window's size and insets before the first layout (tests pass them: there is no window). */
  metrics?: Metrics | null;
}) {
  const handler = useRef<((e: UiEvent) => void) | undefined>(undefined);
  const onEvent = useCallback((e: UiEvent) => {
    handler.current?.(e);
  }, []);
  const register = useCallback((fn: (e: UiEvent) => void) => {
    handler.current = fn;
  }, []);
  return (
    <SafeAreaProvider initialMetrics={metrics}>
      <BackendProvider backend={backend} onEvent={onEvent}>
        <Localized language={language}>
          <Themed accent={accent}>{(theme) => <Shell register={register} theme={theme} />}</Themed>
        </Localized>
      </BackendProvider>
    </SafeAreaProvider>
  );
}
