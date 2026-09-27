import { type ReactNode, useEffect, useRef } from "react";
import { cx } from "../cx";
import { useT } from "../i18n/I18nProvider";
import { Keycap } from "./Keycap";

export interface DialogProps {
  open: boolean;
  title: ReactNode;
  children?: ReactNode;
  /** Mono facts row under the body (`history.db · 1.8 MB · 上限 500 条 / 30 天`). */
  facts?: ReactNode;
  actions: ReactNode;
  onClose: () => void;
  /** Footer hint, defaults to `Esc 取消`. */
  hint?: ReactNode;
  width?: number;
}

/** Modal over a 28 % scrim; Esc and scrim click close. Focuses `[data-autofocus]` or the dialog. */
export function Dialog({
  open,
  title,
  children,
  facts,
  actions,
  onClose,
  hint,
  width = 420,
}: DialogProps) {
  const t = useT();
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const node = ref.current;
    const target = node?.querySelector<HTMLElement>("[data-autofocus]") ?? node;
    target?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !e.defaultPrevented) {
        // Every dialog listens on `document`, so stopPropagation cannot order them; the nested
        // dialog marks the key consumed and outer ones (the settings dialog) check the flag.
        e.preventDefault();
        e.stopPropagation();
        onClose();
      }
    };
    // Capture phase: it runs before every bubble-phase document listener regardless of the order
    // they were registered in, so the innermost open dialog is the one that consumes Escape.
    document.addEventListener("keydown", onKey, true);
    return () => {
      document.removeEventListener("keydown", onKey, true);
    };
  }, [open, onClose]);

  if (!open) return null;
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center scrim"
      onClick={onClose}
      data-testid="dialog-scrim">
      <div
        ref={ref}
        role="dialog"
        aria-modal="true"
        aria-labelledby="vt-dialog-title"
        tabIndex={-1}
        onClick={(e) => {
          e.stopPropagation();
        }}
        className={cx(
          "flex flex-col gap-4 rounded-14 bg-surface p-6 outline-none hairline shadow-pop",
        )}
        style={{ width }}>
        <h2 id="vt-dialog-title" className="text-[16px] font-semibold text-fg">
          {title}
        </h2>
        {children !== undefined && (
          <div className="text-[13px] leading-5 text-fg-muted">{children}</div>
        )}
        {facts !== undefined && <div className="mono text-[11px] text-fg-subtle">{facts}</div>}
        <div className="flex items-center justify-between gap-3">
          <span className="mono flex items-center gap-1.5 text-[10px] text-fg-subtle">
            {hint ?? (
              <>
                <Keycap>Esc</Keycap> {t("ui.dialog.escCancel")}
              </>
            )}
          </span>
          <div className="flex items-center gap-2">{actions}</div>
        </div>
      </div>
    </div>
  );
}
