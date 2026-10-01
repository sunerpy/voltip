import { type ReactNode, createContext, useContext } from "react";

/** A confirmation before something that cannot be undone. */
export interface FeatureConfirm {
  title: string;
  body: ReactNode;
  confirmLabel: string;
  tone?: "danger" | "primary";
  onConfirm: () => void;
}

/** What a shared feature (the provider cards, the presets, …) asks of the app it runs in: the
 *  desktop's toasts and confirmation dialog, or the phone's (user decision 2026-10-01: the phone
 *  has the desktop's settings). */
export interface FeatureShell {
  /** A short message; `danger` for a failure. */
  notify: (message: string, tone?: "neutral" | "danger") => void;
  confirm: (spec: FeatureConfirm) => void;
}

const FeatureShellContext = createContext<FeatureShell | undefined>(undefined);

export function FeatureShellProvider({
  shell,
  children,
}: {
  shell: FeatureShell;
  children: ReactNode;
}) {
  return <FeatureShellContext.Provider value={shell}>{children}</FeatureShellContext.Provider>;
}

export function useFeatureShell(): FeatureShell {
  const shell = useContext(FeatureShellContext);
  if (shell === undefined)
    throw new Error("useFeatureShell must be used inside <FeatureShellProvider>");
  return shell;
}
