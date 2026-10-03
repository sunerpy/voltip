import { type Backend, type UiEvent, coreMessageText, resolveLocale } from "@voltip/shared";
import {
  BackendProvider,
  Button,
  Dialog,
  type FeatureShell,
  FeatureShellProvider,
  I18nProvider,
  IconButton,
  Logo,
  applyTheme,
  cx,
  dismissTopDialog,
  resolveTheme,
  systemPrefersDark,
  useBackend,
  useT,
  useToasts,
  useUiState,
} from "@voltip/ui";
import {
  type ReactNode,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import type { SystemBack } from "./app/back";
import { TOUCH, TOUCH_ICON } from "./app/phone-ui";
import type { Scanner } from "./app/scanner";
import { PhoneToasts } from "./app/toasts";
import {
  type ConfirmSpec,
  type MobileShell,
  type Screen,
  ShellContext,
  TAB_ROOTS,
} from "./app/shell";
import { About } from "./screens/About";
import { AiModels } from "./screens/AiModels";
import { Appearance } from "./screens/Appearance";
import { Devices } from "./screens/Devices";
import { Dictionary } from "./screens/Dictionary";
import { Feedback } from "./screens/Feedback";
import { History } from "./screens/History";
import { ComputerSettings } from "./screens/ComputerSettings";
import { HistoryEntry } from "./screens/HistoryEntry";
import { MirrorEntry } from "./screens/MirrorEntry";
import { HistorySettings } from "./screens/HistorySettings";
import { PairDevice } from "./screens/PairDevice";
import { Recording } from "./screens/Recording";
import { Rules } from "./screens/Rules";
import { Scenes } from "./screens/Scenes";
import { Settings } from "./screens/Settings";
import { SpeechModels } from "./screens/SpeechModels";
import { TabBar } from "./screens/TabBar";
import { ThisDevice } from "./screens/ThisDevice";
import { VerifyDevice } from "./screens/VerifyDevice";
import { Welcome } from "./screens/Welcome";

export type { Screen };

export interface AppProps {
  backend: Backend;
  loadScanner: () => Promise<Scanner | undefined>;
  /** Android's back (edge swipe or button); absent in a browser. */
  systemBack?: SystemBack;
  initialScreen?: Screen;
  /** What `settings.locale = "system"` follows; defaults to `navigator.language`. */
  systemLanguage?: string;
}

/** Where 返回 leads from a screen opened on its own (the first screen, or one of a tab root's
 *  pages); a screen opened from another one goes back to that one. */
const PARENT: Partial<Record<Screen, Screen>> = {
  device: "welcome",
  pair: "device",
  verify: "pair",
  speech: "settings",
  ai: "settings",
  appearance: "settings",
  recording: "settings",
  about: "settings",
  dictionary: "settings",
  rules: "settings",
  scenes: "settings",
  entry: "history",
  mirrorEntry: "history",
  computerSettings: "settings",
  historySettings: "settings",
  feedback: "settings",
};

/** How long the second back at 说话 has to leave the app: the usual two seconds on Android. */
export const EXIT_WINDOW_MS = 2000;

/** One screen on the stack, and what it shows (`MobileShell.param`). */
interface Opened {
  screen: Screen;
  param?: string;
}

/** The screens under `screen` when it is the first one shown. */
function stackFor(screen: Screen): Opened[] {
  const stack: Opened[] = [{ screen }];
  for (let parent = PARENT[screen]; parent !== undefined; parent = PARENT[parent])
    stack.unshift({ screen: parent });
  return stack;
}

export function App({ backend, loadScanner, initialScreen, systemLanguage, systemBack }: AppProps) {
  const handler = useRef<((e: UiEvent) => void) | undefined>(undefined);
  const onEvent = useCallback((e: UiEvent) => {
    handler.current?.(e);
  }, []);
  const register = useCallback((fn: (e: UiEvent) => void) => {
    handler.current = fn;
  }, []);
  return (
    <BackendProvider backend={backend} onEvent={onEvent}>
      <LocaleProvider systemLanguage={systemLanguage}>
        <Gate
          loadScanner={loadScanner}
          initialScreen={initialScreen}
          register={register}
          {...(systemBack === undefined ? {} : { systemBack })}
        />
      </LocaleProvider>
    </BackendProvider>
  );
}

/** The phone shares the core's `settings.locale`; before `core_state` resolves the default settings
 *  (`system`) apply, so the splash already speaks the OS language. */
function LocaleProvider({
  systemLanguage,
  children,
}: {
  systemLanguage?: string;
  children: ReactNode;
}) {
  const { settings } = useUiState();
  return (
    <I18nProvider
      locale={resolveLocale(settings.locale, systemLanguage ?? navigator.language)}
      documentLang>
      {children}
    </I18nProvider>
  );
}

interface FrameProps {
  loadScanner: () => Promise<Scanner | undefined>;
  initialScreen?: Screen;
  register: (fn: (e: UiEvent) => void) => void;
  systemBack?: SystemBack;
  initialDevices?: number;
  /** A handshake already completed before the frame mounted (events raced the splash). */
  verifying?: boolean;
}

/** Splash until `core_state` resolves, so the frame can pick its first screen from real data. */
function Gate(props: FrameProps) {
  const { state, error } = useBackend();
  const t = useT();
  if (!state) {
    return (
      <div
        className="flex h-full flex-col items-center justify-center gap-4 bg-canvas p-6 text-center text-fg"
        role="status">
        <Logo size={40} />
        <span className="text-[13px] text-fg-muted">
          {error ? t("mobile.connectFailed", { error }) : t("mobile.connecting")}
        </span>
      </div>
    );
  }
  return (
    <Frame
      {...props}
      initialDevices={state.devices.length}
      verifying={state.pairing.state.state === "awaiting_verification"}
    />
  );
}

function Frame({
  loadScanner,
  initialScreen,
  register,
  systemBack,
  initialDevices = 0,
  verifying = false,
}: FrameProps) {
  const { backend } = useBackend();
  const t = useT();
  const state = useUiState();
  const toasts = useToasts();
  // Land on the device list when this phone already trusts someone; otherwise start at welcome.
  const [stack, setStack] = useState<Opened[]>(() =>
    stackFor(
      verifying ? "verify" : (initialScreen ?? (initialDevices > 0 ? "devices" : "welcome")),
    ),
  );
  const top = stack.at(-1);
  const screen = top?.screen ?? "welcome";
  const param = top?.param;
  const go = useCallback((next: Screen, about?: string) => {
    setStack((current) => {
      // A tab root replaces the stack; it keeps what it shows (记录 a computer's history, §20.8).
      if (TAB_ROOTS.includes(next))
        return [{ screen: next, ...(about === undefined ? {} : { param: about }) }];
      const last = current.at(-1);
      return last?.screen === next && last.param === about
        ? current
        : [...current, { screen: next, ...(about === undefined ? {} : { param: about }) }];
    });
  }, []);
  const back = useCallback(() => {
    setStack((current) => (current.length > 1 ? current.slice(0, -1) : current));
  }, []);
  const [scanner, setScanner] = useState<{ ready: boolean; value: Scanner | undefined }>({
    ready: false,
    value: undefined,
  });
  const [pending, setPending] = useState<ConfirmSpec | undefined>(undefined);
  // Every screen shares the one scroller: a screen opens at its top, not where the last one was.
  const scroller = useRef<HTMLElement>(null);
  useLayoutEffect(() => {
    if (scroller.current) scroller.current.scrollTop = 0;
    // Opening another screen (or the same one about something else) is the reason to scroll up.
    // oxlint-disable-next-line react/exhaustive-effect-dependencies
  }, [screen, param]);

  useEffect(() => {
    applyTheme(resolveTheme(state.settings, systemPrefersDark()));
  }, [state.settings]);

  useEffect(() => {
    let alive = true;
    loadScanner()
      .then((value) => {
        if (alive) setScanner({ ready: true, value });
      })
      .catch(() => {
        if (alive) setScanner({ ready: true, value: undefined });
      });
    return () => {
      alive = false;
    };
  }, [loadScanner]);

  const toast = useCallback(
    (message: string, tone: "neutral" | "danger" = "neutral") => {
      toasts.push({ message, tone, duration: tone === "danger" ? 5000 : 3000 });
    },
    [toasts],
  );

  // Pairing events drive the screen: handshake done → verify; trusted → devices (then reset the session).
  useEffect(() => {
    register((event) => {
      if (event.type === "error")
        toast(t("mobile.toast.error", { message: coreMessageText(event.message) }), "danger");
      else if (event.type === "trusted") toast(t("mobile.toast.trusted", { name: event.name }));
      else if (event.type === "unpaired") toast(t("mobile.toast.unpaired", { name: event.name }));
      else if (event.type === "message") toast(t("mobile.toast.message", { body: event.body }));
      else if (event.type === "identity_changed")
        toast(t("mobile.toast.identityChanged", { name: event.previous.name }), "danger");
      else if (event.type === "pairing") {
        if (event.state.state === "awaiting_verification") go("verify");
        else if (event.state.state === "trusted") {
          go("devices");
          void backend.invoke("pairing_reset");
        }
      }
    });
  }, [register, toast, backend, t, go]);

  const shell = useMemo<MobileShell>(
    () => ({
      screen,
      param,
      go,
      back,
      toast,
      confirm: setPending,
      scanner: scanner.value,
      scannerReady: scanner.ready,
    }),
    [screen, param, go, back, toast, scanner],
  );
  // The shared settings (`@voltip/ui`: provider cards, presets) report through the same toasts and
  // confirmation dialog.
  const features = useMemo<FeatureShell>(
    () => ({
      notify: toast,
      confirm: ({ title, body, confirmLabel, onConfirm }) => {
        setPending({ title, body, confirmLabel, onConfirm });
      },
    }),
    [toast],
  );

  const canGoBack = stack.length > 1;
  const tabRoot = TAB_ROOTS.includes(screen);
  const goUp = () => {
    if (screen === "verify") void backend.invoke("pairing_cancel");
    back();
  };
  // Android's back (user report 2026-10-02: it left the app from every screen). A dialog closes
  // first, then a screen goes up a level, and 记录 or 设置 go to 说话; at 说话 a first back says a
  // second one leaves, and lets that one through to the system, which does what it always does.
  const onSystemBack = useRef<() => void>(() => undefined);
  useEffect(() => {
    onSystemBack.current = () => {
      if (dismissTopDialog()) return;
      if (canGoBack) goUp();
      else if (screen === "history" || screen === "settings")
        go(state.devices.length > 0 ? "devices" : "welcome");
      else {
        toast(t("mobile.backToLeave"));
        systemBack?.release(EXIT_WINDOW_MS);
      }
    };
  });
  useEffect(
    () =>
      systemBack?.listen(() => {
        onSystemBack.current();
      }),
    [systemBack],
  );
  return (
    <ShellContext.Provider value={shell}>
      <FeatureShellProvider shell={features}>
        <div className="mx-auto flex h-full w-full max-w-[430px] flex-col bg-canvas text-fg">
          {/* The title bar of every screen, the desktop's: the surface, a hairline below, the
              title at 16 px, 44 px targets at either end. It runs under the status bar. */}
          <header className="shrink-0 border-b border-border bg-surface pt-[env(safe-area-inset-top)]">
            <div className={cx("flex h-12 items-center gap-1 pr-1", canGoBack ? "pl-1" : "pl-4")}>
              {canGoBack && (
                // The chevron points right in the icon set; turned, it points back.
                <IconButton
                  icon="chevronRight"
                  label={t("mobile.back")}
                  size={28}
                  onClick={goUp}
                  className={cx(TOUCH_ICON, "rotate-180 active:bg-inset")}
                />
              )}
              <h1 className="flex min-w-0 flex-1 items-center gap-2 text-[16px] font-semibold text-fg">
                {screen === "welcome" ? (
                  <>
                    <Logo size={22} />
                    Voltip
                  </>
                ) : (
                  <span className="truncate">{t(`mobile.title.${screen}`)}</span>
                )}
              </h1>
              {screen === "devices" && (
                <IconButton
                  icon="user"
                  label={t("mobile.thisDevice")}
                  size={28}
                  onClick={() => {
                    go("device");
                  }}
                  className={cx(TOUCH_ICON, "active:bg-inset")}
                />
              )}
            </div>
          </header>
          {/* The one scroller; a screen without the tab bar keeps its end above the navigation
              bar. */}
          <main
            ref={scroller}
            className={cx(
              "min-h-0 flex-1 overflow-y-auto overscroll-none",
              !tabRoot && "pb-[env(safe-area-inset-bottom)]",
            )}>
            {screen === "welcome" && <Welcome />}
            {screen === "device" && <ThisDevice />}
            {screen === "pair" && <PairDevice />}
            {screen === "verify" && <VerifyDevice />}
            {screen === "devices" && <Devices />}
            {screen === "settings" && <Settings />}
            {screen === "speech" && <SpeechModels />}
            {screen === "ai" && <AiModels />}
            {screen === "appearance" && <Appearance />}
            {screen === "recording" && <Recording />}
            {screen === "about" && <About />}
            {screen === "dictionary" && <Dictionary />}
            {screen === "rules" && <Rules />}
            {screen === "scenes" && <Scenes />}
            {screen === "history" && <History />}
            {screen === "entry" && <HistoryEntry key={param} />}
            {screen === "mirrorEntry" && <MirrorEntry key={param} />}
            {screen === "computerSettings" && <ComputerSettings key={param} />}
            {screen === "historySettings" && <HistorySettings />}
            {screen === "feedback" && <Feedback />}
          </main>
          {tabRoot && <TabBar />}
          {/* No key hint under the buttons: a phone has no Esc; its back closes the dialog. */}
          <Dialog
            open={pending !== undefined}
            title={pending?.title ?? ""}
            width={340}
            hint=""
            onClose={() => {
              setPending(undefined);
            }}
            actions={
              <>
                <Button
                  variant="ghost"
                  className={TOUCH}
                  data-autofocus
                  onClick={() => {
                    setPending(undefined);
                  }}>
                  {t("mobile.cancel")}
                </Button>
                <Button
                  variant="danger"
                  className={TOUCH}
                  onClick={() => {
                    pending?.onConfirm();
                    setPending(undefined);
                  }}>
                  {pending?.confirmLabel}
                </Button>
              </>
            }>
            {pending?.body}
          </Dialog>
          <PhoneToasts toasts={toasts.toasts} aboveTabBar={tabRoot} />
        </div>
      </FeatureShellProvider>
    </ShellContext.Provider>
  );
}
