import { type TFunction, zhT } from "@voltip/shared";
import { type ToastInput, type ToastStore, useT, useToasts } from "@voltip/ui";
import { type ReactNode, createContext, useCallback, useContext, useMemo, useState } from "react";

export interface ConfirmSpec {
  title: string;
  body: ReactNode;
  facts?: ReactNode;
  confirmLabel: string;
  tone?: "danger" | "primary";
  onConfirm: () => void;
}

export interface ShellActions {
  toast: (input: ToastInput) => string;
  toasts: ToastStore;
  confirm: (spec: ConfirmSpec) => void;
  /** The pending confirm dialog, rendered by the shell. */
  pending: ConfirmSpec | undefined;
  closeConfirm: () => void;
  paletteOpen: boolean;
  setPaletteOpen: (open: boolean) => void;
  /** The mounted locale's translator, so helpers such as `copyWithToast` speak the UI language. */
  t: TFunction;
}

const ShellContext = createContext<ShellActions | undefined>(undefined);

export function ShellProvider({ children }: { children: ReactNode }) {
  const t = useT();
  const toasts = useToasts();
  const [pending, setPending] = useState<ConfirmSpec | undefined>(undefined);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const closeConfirm = useCallback(() => {
    setPending(undefined);
  }, []);
  const value = useMemo<ShellActions>(
    () => ({
      toast: toasts.push,
      toasts,
      confirm: setPending,
      pending,
      closeConfirm,
      paletteOpen,
      setPaletteOpen,
      t,
    }),
    [toasts, pending, closeConfirm, paletteOpen, t],
  );
  return <ShellContext.Provider value={value}>{children}</ShellContext.Provider>;
}

export function useShell(): ShellActions {
  const ctx = useContext(ShellContext);
  if (!ctx) throw new Error("useShell must be used inside <ShellProvider>");
  return ctx;
}

/** Copies to the clipboard when the API exists (jsdom and old webviews lack it) and reports success. */
export async function copyText(text: string): Promise<boolean> {
  const clipboard: Pick<Clipboard, "writeText"> | undefined = navigator.clipboard;
  if (!clipboard) return false;
  try {
    await clipboard.writeText(text);
    return true;
  } catch (_error) {
    return false;
  }
}

/** The clipboard failure toast in the default locale; `copyWithToast` reads the shell's `t`. */
export const CLIPBOARD_UNAVAILABLE_MESSAGE = zhT.t("common.clipboardUnavailable");
const COPY_TOAST_MS = 2000;
const COPY_FAILED_TOAST_MS = 3000;

/** Copies and reports honestly: the success toast fires only after the clipboard accepted the
 *  text; otherwise a danger toast says nothing was copied. Resolves to the copy result. */
export async function copyWithToast(
  shell: Pick<ShellActions, "toast" | "t">,
  text: string,
  message: string,
): Promise<boolean> {
  const ok = await copyText(text);
  if (ok) shell.toast({ message, duration: COPY_TOAST_MS });
  else
    shell.toast({
      message: shell.t("common.clipboardUnavailable"),
      duration: COPY_FAILED_TOAST_MS,
      tone: "danger",
    });
  return ok;
}
