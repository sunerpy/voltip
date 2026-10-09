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
  | "about"
  | "dictionary"
  | "rules"
  | "scenes"
  | "history"
  | "entry"
  | "mirrorEntry"
  | "computerSettings"
  | "historySettings"
  | "feedback";

/** The screens the tab bar switches between (user decision 2026-10-01: the phone has its own
 *  settings and history). 说话 is the welcome screen until a computer is paired, the device list
 *  after. */
export const TAB_ROOTS: readonly Screen[] = ["welcome", "devices", "history", "settings"];

export interface ConfirmSpec {
  title: string;
  body: ReactNode;
  confirmLabel: string;
  onConfirm: () => void;
}

export interface MobileShell {
  screen: Screen;
  /** What the current screen shows, when it shows one thing: the id of the history entry on
   *  `entry`, `desktop/id` on `mirrorEntry`, the computer's key on `computerSettings` and on
   *  `history` when it shows a computer's history (docs/dictation.md §20.8). */
  param: string | undefined;
  /** Open `screen` (about `param`): a tab root replaces the stack, any other screen goes on top
   *  of it. */
  go: (screen: Screen, param?: string) => void;
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
