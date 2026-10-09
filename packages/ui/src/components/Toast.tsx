import { useCallback, useEffect, useRef, useState } from "react";
import { cx } from "../cx";
import { Keycap } from "./Keycap";

export interface ToastAction {
  label: string;
  keys?: string;
  onClick: () => void;
}

export interface ToastItem {
  id: string;
  message: string;
  action?: ToastAction;
  /** Milliseconds before auto-dismiss (2 000 / 3 000 / 5 000 by tone). */
  duration: number;
  tone?: "neutral" | "danger";
}

export interface ToastProps {
  toast: ToastItem;
  onDismiss: (id: string) => void;
}

/** 320×44 card with a 2 px drain bar shrinking over the undo window. */
export function Toast({ toast, onDismiss }: ToastProps) {
  return (
    <div
      role={toast.tone === "danger" ? "alert" : "status"}
      data-tone={toast.tone ?? "neutral"}
      className="relative flex h-11 w-80 items-center justify-between gap-3 overflow-hidden rounded-10 bg-surface px-3.5 text-[13px] hairline shadow-pop">
      <span className={cx("truncate", toast.tone === "danger" ? "text-danger" : "text-fg")}>
        {toast.message}
      </span>
      {toast.action && (
        <button
          type="button"
          className="flex shrink-0 items-center gap-1.5 font-semibold text-fg hover:underline"
          onClick={() => {
            toast.action?.onClick();
            onDismiss(toast.id);
          }}>
          {toast.action.label}
          {toast.action.keys && <Keycap>{toast.action.keys}</Keycap>}
        </button>
      )}
      <span
        className="absolute bottom-0 left-0 h-0.5 bg-primary"
        style={{ animation: `vt-drain ${toast.duration}ms linear forwards` }}
      />
    </div>
  );
}

export interface ToastViewportProps {
  toasts: readonly ToastItem[];
  onDismiss: (id: string) => void;
}

/** Bottom-right stack, newest at the bottom, max 3 visible. */
export function ToastViewport({ toasts, onDismiss }: ToastViewportProps) {
  const visible = toasts.slice(-3);
  if (visible.length === 0) return null;
  return (
    <div className="pointer-events-none fixed right-6 bottom-6 z-40 flex flex-col items-end gap-2">
      {visible.map((t) => (
        <div key={t.id} className="pointer-events-auto">
          <Toast toast={t} onDismiss={onDismiss} />
        </div>
      ))}
    </div>
  );
}

export type ToastInput = Omit<ToastItem, "id" | "duration"> & { duration?: number };

export interface ToastStore {
  toasts: ToastItem[];
  push: (input: ToastInput) => string;
  dismiss: (id: string) => void;
}

/** Queue with auto-dismiss timers; timers are cleared on unmount. */
export function useToasts(): ToastStore {
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const timers = useRef(new Map<string, ReturnType<typeof setTimeout>>());
  const counter = useRef(0);

  const dismiss = useCallback((id: string) => {
    const t = timers.current.get(id);
    if (t !== undefined) clearTimeout(t);
    timers.current.delete(id);
    setToasts((prev) => prev.filter((x) => x.id !== id));
  }, []);

  const push = useCallback(
    (input: ToastInput) => {
      counter.current += 1;
      const id = `toast-${counter.current}`;
      const duration = input.duration ?? 3000;
      setToasts((prev) => [...prev, { ...input, id, duration }]);
      timers.current.set(
        id,
        setTimeout(() => {
          dismiss(id);
        }, duration),
      );
      return id;
    },
    [dismiss],
  );

  useEffect(() => {
    const map = timers.current;
    return () => {
      for (const t of map.values()) clearTimeout(t);
      map.clear();
    };
  }, []);

  return { toasts, push, dismiss };
}
