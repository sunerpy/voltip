// What every screen may ask of the app around it: a short message (Material's snackbar, above the
// navigation bar), with an action at its end when there is one (撤销), and a confirmation before
// something that cannot be undone (an MD3 dialog). The apps/mobile shell's `toast` and `confirm`,
// on Paper.
import {
  type ReactNode,
  createContext,
  useCallback,
  useContext,
  useMemo,
  useRef,
  useState,
} from "react";
import { Dialog, Portal, Snackbar, Text } from "react-native-paper";

import { useT } from "./backend/i18n";
import { Button } from "./ui/Button";
import { useAppTheme } from "./ui/kit";

export interface ConfirmSpec {
  title: string;
  body: ReactNode;
  confirmLabel: string;
  /** The button that declines; 取消 unless a confirmation names its own (手势粘贴's 暂不开启). */
  cancelLabel?: string;
  /** The confirming button's colour: the error colour for what cannot be undone (the default),
   *  the primary colour for an agreement. */
  tone?: "danger" | "primary";
  onConfirm: () => void;
}

/** The button at the end of a message: what undoes what it reports (the quick fix's 撤销). */
export interface ToastAction {
  label: string;
  onPress: () => void;
}

export interface Shell {
  toast: (message: string, tone?: "neutral" | "danger", action?: ToastAction) => void;
  confirm: (spec: ConfirmSpec) => void;
}

const ShellContext = createContext<Shell | undefined>(undefined);

export function useShell(): Shell {
  const shell = useContext(ShellContext);
  if (shell === undefined) throw new Error("useShell must be used inside <ShellProvider>");
  return shell;
}

interface Toast {
  id: number;
  message: string;
  tone: "neutral" | "danger";
  action?: ToastAction;
}

/** How long a message stays: a failure longer, as on the desktop. */
const TOAST_MS = { neutral: 3000, danger: 5000 } as const;
/** A message with an action stays as long as the desktop's undo toast. */
export const ACTION_TOAST_MS = 5000;

function toastMs(toast: Toast | undefined): number {
  if (toast === undefined) return TOAST_MS.neutral;
  return toast.action === undefined ? TOAST_MS[toast.tone] : ACTION_TOAST_MS;
}

export function ShellProvider({
  children,
  bottomInset,
}: {
  children: ReactNode;
  bottomInset: number;
}) {
  const theme = useAppTheme();
  const t = useT();
  const [queue, setQueue] = useState<Toast[]>([]);
  const [pending, setPending] = useState<ConfirmSpec | undefined>(undefined);
  const next = useRef(0);
  const toast = useCallback(
    (message: string, tone: "neutral" | "danger" = "neutral", action?: ToastAction) => {
      next.current += 1;
      const id = next.current;
      setQueue((q) => [
        ...q.filter((m) => m.message !== message),
        { id, message, tone, ...(action === undefined ? {} : { action }) },
      ]);
    },
    [],
  );
  const shell = useMemo<Shell>(() => ({ toast, confirm: setPending }), [toast]);
  const current = queue[0];
  return (
    <ShellContext.Provider value={shell}>
      {children}
      <Portal>
        <Dialog
          visible={pending !== undefined}
          onDismiss={() => {
            setPending(undefined);
          }}>
          <Dialog.Title>{pending?.title}</Dialog.Title>
          <Dialog.Content>
            {typeof pending?.body === "string" ? (
              <Text variant="bodyMedium">{pending.body}</Text>
            ) : (
              pending?.body
            )}
          </Dialog.Content>
          <Dialog.Actions>
            <Button
              testID="shell-confirm-cancel"
              onPress={() => {
                setPending(undefined);
              }}>
              {pending?.cancelLabel ?? t("mobile.cancel")}
            </Button>
            <Button
              textColor={pending?.tone === "primary" ? theme.colors.primary : theme.colors.error}
              testID="shell-confirm"
              onPress={() => {
                pending?.onConfirm();
                setPending(undefined);
              }}>
              {pending?.confirmLabel}
            </Button>
          </Dialog.Actions>
        </Dialog>
        <Snackbar
          key={current?.id}
          visible={current !== undefined}
          duration={toastMs(current)}
          onDismiss={() => {
            setQueue((q) => q.slice(1));
          }}
          action={
            current?.action === undefined
              ? undefined
              : { label: current.action.label, onPress: current.action.onPress }
          }
          wrapperStyle={{ bottom: bottomInset }}
          style={current?.tone === "danger" ? { backgroundColor: theme.voltip.danger } : undefined}
          theme={
            current?.tone === "danger" ? { colors: { inverseOnSurface: "#ffffff" } } : undefined
          }>
          {current?.message ?? ""}
        </Snackbar>
      </Portal>
    </ShellContext.Provider>
  );
}
