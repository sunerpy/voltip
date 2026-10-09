// What every screen may ask of the app around it: a short message (Material's snackbar, above the
// navigation bar) and a confirmation before something that cannot be undone (an MD3 dialog). The
// apps/mobile shell's `toast` and `confirm`, on Paper.
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
  onConfirm: () => void;
}

export interface Shell {
  toast: (message: string, tone?: "neutral" | "danger") => void;
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
}

/** How long a message stays: a failure longer, as on the desktop. */
const TOAST_MS = { neutral: 3000, danger: 5000 } as const;

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
  const toast = useCallback((message: string, tone: "neutral" | "danger" = "neutral") => {
    next.current += 1;
    const id = next.current;
    setQueue((q) => [...q.filter((m) => m.message !== message), { id, message, tone }]);
  }, []);
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
              onPress={() => {
                setPending(undefined);
              }}>
              {t("mobile.cancel")}
            </Button>
            <Button
              textColor={theme.colors.error}
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
          duration={current === undefined ? TOAST_MS.neutral : TOAST_MS[current.tone]}
          onDismiss={() => {
            setQueue((q) => q.slice(1));
          }}
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
