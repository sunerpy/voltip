import { type Backend, type UiEvent, coreMessageText, resolveLocale } from "@voltip/shared";
import { BackendProvider, I18nProvider, useT, useUiState } from "@voltip/ui";
import { type ReactNode, Suspense, lazy, useCallback, useEffect, useRef } from "react";
import { AppearanceProvider } from "./app/appearance";
import { FeedbackDraftProvider } from "./app/feedback-draft";
import { type Route, RouterProvider, isDialogRoute, routePath, useRouter } from "./app/router";
import { ShellProvider, useShell } from "./app/shell-context";
import type { TrayRequestSource } from "./app/tray-requests";
import { Devices } from "./pages/Devices";
import { Dictionary } from "./pages/Dictionary";
import { FeedbackDialog } from "./pages/Feedback";
import { History } from "./pages/History";
import { Home } from "./pages/Home";
import { NotFound } from "./pages/NotFound";
import { Onboarding } from "./pages/Onboarding";
import { AiModels, SpeechModels } from "./pages/Models";
import { LIVE_STATE, BLANK_STATE, Overlay, isOverlayWindowState } from "./pages/Overlay";
import { Rules } from "./pages/Rules";
import { SettingsDialog } from "./pages/settings/SettingsDialog";
import { Shell } from "./shell/Shell";

/** The pill spec sheet and the single sample pills run on made-up values: `pnpm dev` only.
 *  `import.meta.env.DEV` is a build-time constant, so a release build drops the import. */
const OverlaySheet = import.meta.env.DEV ? lazy(() => import("./pages/OverlaySheet")) : undefined;

function OverlayPreview({ state }: { state?: string }) {
  if (OverlaySheet === undefined) {
    return (
      <NotFound
        path={routePath(state === undefined ? { name: "overlay" } : { name: "overlay", state })}
      />
    );
  }
  return (
    <Suspense fallback={null}>
      <OverlaySheet state={state} />
    </Suspense>
  );
}

export interface AppProps {
  backend: Backend;
  /** Memory routing for tests; the real app reads `window.location`. */
  initialPath?: string;
  /** The OS / webview language `settings.locale = "system"` follows; defaults to `navigator.language`. */
  systemLanguage?: string;
  /** The tray menu's requests; defaults to the desktop shell's `voltip://tray` event. */
  traySource?: TrayRequestSource;
}

/** `/settings/:section` and `/feedback` are modals over the last page that was not a dialog
 *  (`background`), so the page beneath stays mounted; it is inert and hidden from assistive tech
 *  while the dialog is open. */
function Routes() {
  const { route, background } = useRouter();
  if (!isDialogRoute(route)) return <Page route={route} />;
  return (
    <>
      <div inert aria-hidden="true" data-testid="page-background">
        <Page route={background} />
      </div>
      {route.name === "settings" ? <SettingsDialog section={route.section} /> : <FeedbackDialog />}
    </>
  );
}

function Page({ route }: { route: Exclude<Route, { name: "settings" | "feedback" }> }) {
  switch (route.name) {
    case "home":
      return <Home />;
    case "history":
      return <History initialFilter={route.filter} />;
    case "dictionary":
      return <Dictionary />;
    case "rules":
      return <Rules compose={route.compose === true} />;
    case "devices":
      return <Devices />;
    case "speech":
      return <SpeechModels />;
    case "ai":
      return <AiModels />;
    case "onboarding":
      return <Onboarding step={route.step} />;
    case "overlay":
      return <OverlayPreview state={route.state} />;
    case "notfound":
      return <NotFound path={route.path} />;
  }
}

/** Turns core events into toasts; identity-changed banners live on the devices page. */
function EventToasts({ onEvent }: { onEvent: (fn: (e: UiEvent) => void) => void }) {
  const shell = useShell();
  const t = useT();
  const state = useUiState();
  const devices = useRef(state.devices);
  useEffect(() => {
    devices.current = state.devices;
  }, [state.devices]);
  useEffect(() => {
    onEvent((event) => {
      switch (event.type) {
        case "error":
          shell.toast({
            message: t("app.error", { message: coreMessageText(event.message) }),
            duration: 5000,
            tone: "danger",
          });
          break;
        case "trusted":
          shell.toast({ message: t("app.trusted", { name: event.name }), duration: 3000 });
          break;
        case "unpaired":
          shell.toast({ message: t("app.unpaired", { name: event.name }), duration: 5000 });
          break;
        case "message": {
          const from =
            devices.current.find((d) => d.device.public_key === event.from)?.device.name ??
            t("app.unknownDevice");
          shell.toast({ message: t("app.message", { from, body: event.body }), duration: 5000 });
          break;
        }
        case "identity_changed":
          shell.toast({
            message: t("app.identityChanged", { name: event.previous.name }),
            duration: 5000,
            tone: "danger",
          });
          break;
        case "state":
        case "identity":
        case "settings":
        case "relay":
        case "pairing":
        case "devices":
        case "hotkey":
        case "dictation":
        case "history":
        case "engines":
        case "update":
        case "models":
        case "dictionary":
        case "rules":
          break;
      }
    });
  }, [onEvent, shell, t]);
  return null;
}

/** The UI language is the core's `settings.locale` (shared by the main window, the pill window
 *  and the phone); `system` follows the webview language. `<html lang>` follows the resolution. */
function LocaleProvider({
  systemLanguage,
  children,
}: {
  systemLanguage?: string;
  children: ReactNode;
}) {
  const { settings } = useUiState();
  const locale = resolveLocale(settings.locale, systemLanguage ?? navigator.language);
  return (
    <I18nProvider locale={locale} documentLang>
      {children}
    </I18nProvider>
  );
}

export function App({ backend, initialPath, systemLanguage, traySource }: AppProps) {
  const handler = useRef<((e: UiEvent) => void) | undefined>(undefined);
  const onEvent = useCallback((event: UiEvent) => {
    handler.current?.(event);
  }, []);
  const register = useCallback((fn: (e: UiEvent) => void) => {
    handler.current = fn;
  }, []);
  return (
    <BackendProvider backend={backend} onEvent={onEvent}>
      <LocaleProvider systemLanguage={systemLanguage}>
        <RouterProvider initialPath={initialPath}>
          <ShellProvider>
            <AppearanceProvider>
              <EventToasts onEvent={register} />
              <Frame traySource={traySource} />
            </AppearanceProvider>
          </ShellProvider>
        </RouterProvider>
      </LocaleProvider>
    </BackendProvider>
  );
}

/** The overlay window has no chrome; every other route lives inside the shell. */
function Frame({ traySource }: { traySource?: TrayRequestSource }) {
  const { route } = useRouter();
  const overlayWindow =
    route.name === "overlay" && route.state !== undefined && isOverlayWindowState(route.state);
  // `body[data-window="overlay"]` makes the document background transparent so only the pill paints.
  useEffect(() => {
    if (!overlayWindow) return;
    document.body.dataset.window = "overlay";
    return () => {
      delete document.body.dataset.window;
    };
  }, [overlayWindow]);
  if (overlayWindow) {
    return route.state === LIVE_STATE || route.state === BLANK_STATE ? (
      <Overlay state={route.state} />
    ) : (
      <OverlayPreview state={route.state} />
    );
  }
  // The 反馈 draft belongs to the main window: the overlay window never stages files.
  return (
    <FeedbackDraftProvider>
      <Shell traySource={traySource}>
        <Routes />
      </Shell>
    </FeedbackDraftProvider>
  );
}
