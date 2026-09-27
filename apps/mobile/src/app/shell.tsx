import { type ReactNode, createContext, useContext } from "react";
import type { Scanner } from "./scanner";

export type Screen = "welcome" | "device" | "pair" | "verify" | "devices";

export interface ConfirmSpec {
  title: string;
  body: ReactNode;
  confirmLabel: string;
  onConfirm: () => void;
}

export interface MobileShell {
  screen: Screen;
  go: (screen: Screen) => void;
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
