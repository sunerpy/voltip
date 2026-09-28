import {
  type Backend,
  type UiEvent,
  type UiState,
  applyEvent,
  defaultSettings,
  emptyEngineStatus,
  emptyHotkeyStatus,
  idleDictation,
  idleUpdate,
} from "@voltip/shared";
import { type ReactNode, createContext, useContext, useEffect, useMemo, useState } from "react";

export interface BackendContextValue {
  backend: Backend;
  /** `undefined` until `core_state` resolved. */
  state: UiState | undefined;
  /** Last non-state event (trusted / identity_changed / message / error) for toasts and banners. */
  lastEvent: UiEvent | undefined;
  error: string | undefined;
}

const BackendContext = createContext<BackendContextValue | undefined>(undefined);

export interface BackendProviderProps {
  backend: Backend;
  children: ReactNode;
  /** Side channel for every event, in arrival order (toasts, banners). */
  onEvent?: (event: UiEvent) => void;
}

/** Loads `core_state`, folds every `voltip://event` into it and shares both via context. */
export function BackendProvider({ backend, children, onEvent }: BackendProviderProps) {
  const [state, setState] = useState<UiState | undefined>(undefined);
  const [lastEvent, setLastEvent] = useState<UiEvent | undefined>(undefined);
  const [error, setError] = useState<string | undefined>(undefined);

  useEffect(() => {
    let alive = true;
    const off = backend.on((event) => {
      if (!alive) return;
      setState((prev) => {
        if (prev) return applyEvent(prev, event);
        return event.type === "state" ? applyEvent(EMPTY, event) : prev;
      });
      setLastEvent(event);
      onEvent?.(event);
    });
    backend
      .getState()
      .then((s) => {
        if (alive) setState((prev) => prev ?? s);
      })
      .catch((e: unknown) => {
        if (alive) setError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      alive = false;
      off();
    };
  }, [backend, onEvent]);

  const value = useMemo(
    () => ({ backend, state, lastEvent, error }),
    [backend, state, lastEvent, error],
  );
  return <BackendContext.Provider value={value}>{children}</BackendContext.Provider>;
}

const EMPTY: UiState = {
  identity: null,
  settings: defaultSettings(),
  secret_backend: "",
  app_version: "",
  relay: { state: "disconnected", attempts: 0, source: "none" },
  pairing: { state: { state: "idle" }, local_confirmed: false, peer_confirmed: false },
  devices: [],
  hotkey: emptyHotkeyStatus(),
  dictation: idleDictation(),
  sent_texts: [],
  nearby: [],
  history: [],
  engines: emptyEngineStatus(),
  update: idleUpdate(),
  models: [],
  dictionary: [],
  rules: [],
  scenes: [],
  hardware: { cpu_threads: 0, gpus: [] },
  connectivity: { running: false },
};

export function useBackend(): BackendContextValue {
  const ctx = useContext(BackendContext);
  if (!ctx) throw new Error("useBackend must be used inside <BackendProvider>");
  return ctx;
}

/** Convenience: the current state or the empty placeholder while loading. */
export function useUiState(): UiState {
  return useBackend().state ?? EMPTY;
}
