import { type Backend, type UiEvent, coreMessageText, resolveLocale } from "@voltip/shared";
import {
  BackendProvider,
  Button,
  Dialog,
  I18nProvider,
  Icon,
  ToastViewport,
  applyTheme,
  resolveTheme,
  systemPrefersDark,
  useBackend,
  useT,
  useToasts,
  useUiState,
} from "@voltip/ui";
import { type ReactNode, useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { Scanner } from "./app/scanner";
import { type ConfirmSpec, type MobileShell, type Screen, ShellContext } from "./app/shell";
import { Devices } from "./screens/Devices";
import { PairDevice } from "./screens/PairDevice";
import { ThisDevice } from "./screens/ThisDevice";
import { VerifyDevice } from "./screens/VerifyDevice";
import { Welcome } from "./screens/Welcome";

export type { Screen };

export interface AppProps {
  backend: Backend;
  loadScanner: () => Promise<Scanner | undefined>;
  initialScreen?: Screen;
  /** What `settings.locale = "system"` follows; defaults to `navigator.language`. */
  systemLanguage?: string;
}

const BACK: Partial<Record<Screen, Screen>> = {
  device: "welcome",
  pair: "device",
  verify: "pair",
  devices: "device",
};

export function App({ backend, loadScanner, initialScreen, systemLanguage }: AppProps) {
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
        <Gate loadScanner={loadScanner} initialScreen={initialScreen} register={register} />
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
        className="flex h-full min-h-screen flex-col items-center justify-center gap-3 bg-canvas p-6 text-center text-fg"
        role="status">
        <Icon name="wave" size={28} className="text-fg-subtle" />
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
  initialDevices = 0,
  verifying = false,
}: FrameProps) {
  const { backend } = useBackend();
  const t = useT();
  const state = useUiState();
  const toasts = useToasts();
  // Land on the device list when this phone already trusts someone; otherwise start at welcome.
  const [screen, setScreen] = useState<Screen>(
    verifying ? "verify" : (initialScreen ?? (initialDevices > 0 ? "devices" : "welcome")),
  );
  const [scanner, setScanner] = useState<{ ready: boolean; value: Scanner | undefined }>({
    ready: false,
    value: undefined,
  });
  const [pending, setPending] = useState<ConfirmSpec | undefined>(undefined);

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
        if (event.state.state === "awaiting_verification") setScreen("verify");
        else if (event.state.state === "trusted") {
          setScreen("devices");
          void backend.invoke("pairing_reset");
        }
      }
    });
  }, [register, toast, backend, t]);

  const shell = useMemo<MobileShell>(
    () => ({
      screen,
      go: setScreen,
      toast,
      confirm: setPending,
      scanner: scanner.value,
      scannerReady: scanner.ready,
    }),
    [screen, toast, scanner],
  );

  const back = BACK[screen];
  return (
    <ShellContext.Provider value={shell}>
      <div className="mx-auto flex h-full min-h-screen w-full max-w-[430px] flex-col bg-canvas text-fg">
        {screen !== "welcome" && (
          <header className="flex h-12 shrink-0 items-center gap-2 border-b border-border bg-surface px-3">
            {back && (
              <button
                type="button"
                aria-label={t("mobile.back")}
                onClick={() => {
                  if (screen === "verify") void backend.invoke("pairing_cancel");
                  setScreen(back);
                }}
                className="flex h-8 w-8 items-center justify-center rounded-6 text-fg-muted hover:bg-inset">
                <Icon name="chevronRight" size={16} className="rotate-180" />
              </button>
            )}
            <h1 className="flex-1 text-[15px] font-semibold">{t(`mobile.title.${screen}`)}</h1>
            {screen === "devices" && (
              <button
                type="button"
                aria-label={t("mobile.thisDevice")}
                onClick={() => {
                  setScreen("device");
                }}
                className="flex h-8 w-8 items-center justify-center rounded-6 text-fg-muted hover:bg-inset">
                <Icon name="user" size={16} />
              </button>
            )}
          </header>
        )}
        <main className="min-h-0 flex-1 overflow-y-auto">
          {screen === "welcome" && <Welcome />}
          {screen === "device" && <ThisDevice />}
          {screen === "pair" && <PairDevice />}
          {screen === "verify" && <VerifyDevice />}
          {screen === "devices" && <Devices />}
        </main>
        <Dialog
          open={pending !== undefined}
          title={pending?.title ?? ""}
          width={340}
          onClose={() => {
            setPending(undefined);
          }}
          actions={
            <>
              <Button
                size="sm"
                variant="ghost"
                data-autofocus
                onClick={() => {
                  setPending(undefined);
                }}>
                {t("mobile.cancel")}
              </Button>
              <Button
                size="sm"
                variant="danger"
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
        <ToastViewport toasts={toasts.toasts} onDismiss={toasts.dismiss} />
      </div>
    </ShellContext.Provider>
  );
}
