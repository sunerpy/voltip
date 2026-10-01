import { type ReactNode, createContext, useContext } from "react";
import type { Scanner } from "./scanner";

export type Screen =
  | "welcome"
  | "device"
  | "pair"
  | "verify"
  | "devices"
  | "settings"
  | "speech"
  | "ai"
  | "appearance"
  | "recording"
  | "about";

/** The screens the tab bar switches between (user decision 2026-10-01: the phone has its own
 *  settings). 说话 is the welcome screen until a computer is paired, the device list after. */
export const TAB_ROOTS: readonly Screen[] = ["welcome", "devices", "settings"];

export interface ConfirmSpec {
  title: string;
  body: ReactNode;
  confirmLabel: string;
  onConfirm: () => void;
}

export interface MobileShell {
  screen: Screen;
  /** Open `screen`: a tab root replaces the stack, any other screen goes on top of it. */
  go: (screen: Screen) => void;
  /** Back to the screen underneath (the header's back button). */
  back: () => void;
  toast: (message: string, tone?: "neutral" | "danger") => void;
  confirm: (spec: ConfirmSpec) => void;
  scanner: Scanner | undefined;
  scannerReady: boolean;
}

export const ShellContext = createContext<MobileShell | undefined>(undefined);

export function useMobileShell(): MobileShell {
  const ctx = useContext(ShellContext);
  if (!ctx) throw new Error("useMobileShell must be used inside <App>");
  return ctx;
}
