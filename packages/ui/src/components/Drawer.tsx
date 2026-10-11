import { type ReactNode, useEffect, useId, useRef } from "react";
import { cx } from "../cx";
import { useT } from "../i18n/I18nProvider";
import { Icon, type IconName } from "./Icon";
import { IconButton } from "./IconButton";
import { openLayer } from "./layers";

export interface DrawerProps {
  open: boolean;
  onClose: () => void;
  /** Names the panel. */
  title: ReactNode;
  icon?: IconName;
  /** `accent`: the item in use (a soft accent block, never a solid one). */
  iconTone?: "neutral" | "accent";
  /** Status badges after the title. */
  badges?: ReactNode;
  /** A sentence under the title. */
  description?: ReactNode;
  /** `DrawerBody` (scrolls) and `DrawerFooter` (stays at the bottom). */
  children?: ReactNode;
  /** The widest the panel gets; never wider than the window less 24 px. */
  width?: number;
  "data-testid"?: string;
}

/** What Tab moves between inside the panel. */
const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/** Keeps Tab inside `root`: past the last control it comes back to the first, and the other way. */
function keepTab(e: KeyboardEvent, root: HTMLElement) {
  const items = [...root.querySelectorAll<HTMLElement>(FOCUSABLE)].filter(
    (el) => el.closest("[hidden]") === null,
  );
  const first = items[0];
  const last = items.at(-1);
  if (first === undefined || last === undefined) {
    e.preventDefault();
    return;
  }
  const active = document.activeElement;
  if (e.shiftKey && (active === first || active === root)) {
    e.preventDefault();
    last.focus();
  } else if (!e.shiftKey && (active === last || !root.contains(active))) {
    e.preventDefault();
    first.focus();
  }
}

/** A panel from the right edge over a 28 % scrim, for the settings of one item of a list (a
 *  provider's model, address and key; user decision 2026-10-10). Esc, the close button and a click
 *  on the scrim close it; Tab stays inside it, and the focus goes back to what opened it. It shares
 *  the dialogs' stack (`layers.ts`): Esc closes a dialog or a list opened over it first. */
export function Drawer({
  open,
  onClose,
  title,
  icon,
  iconTone = "neutral",
  badges,
  description,
  children,
  width = 480,
  ...rest
}: DrawerProps) {
  const t = useT();
  const panel = useRef<HTMLDivElement>(null);
  const titleId = useId();
  const descriptionId = useId();
  // The latest close handler, so a re-render neither re-focuses the panel nor reorders the stack.
  const close = useRef(onClose);
  useEffect(() => {
    close.current = onClose;
  }, [onClose]);

  useEffect(() => {
    if (!open) return;
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const layer = openLayer(() => {
      close.current();
    });
    const node = panel.current;
    (node?.querySelector<HTMLElement>("[data-autofocus]") ?? node)?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || !layer.isTop() || node === null) return;
      if (e.key === "Escape") {
        // Capture phase, marked consumed: outer listeners (the settings dialog) leave it alone.
        e.preventDefault();
        e.stopPropagation();
        close.current();
      } else if (e.key === "Tab") keepTab(e, node);
    };
    document.addEventListener("keydown", onKey, true);
    return () => {
      document.removeEventListener("keydown", onKey, true);
      layer.remove();
      // Back to the card that opened it, unless the focus has gone somewhere else meanwhile.
      const now = document.activeElement;
      if (opener?.isConnected && (now === null || now === document.body || node?.contains(now)))
        opener.focus();
    };
  }, [open]);

  if (!open) return null;
  return (
    <div
      className="fixed inset-0 z-50 flex justify-end scrim"
      onClick={onClose}
      data-testid="drawer-scrim">
      <div
        ref={panel}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={description === undefined ? undefined : descriptionId}
        tabIndex={-1}
        data-testid={rest["data-testid"]}
        onClick={(e) => {
          e.stopPropagation();
        }}
        className="flex h-full flex-col border-l border-raised-edge bg-raised shadow-win outline-none glass-blur motion-safe:[animation:vt-drawer-in_180ms_cubic-bezier(0.2,0,0,1)]"
        style={{ width: `min(${width}px, calc(100vw - 24px))` }}>
        <header className="flex shrink-0 items-start gap-3 border-b border-border px-6 pt-5 pb-4">
          {icon !== undefined && (
            <span
              className={cx(
                "flex size-9 shrink-0 items-center justify-center rounded-10",
                iconTone === "accent"
                  ? "bg-accent-soft text-accent-text"
                  : "bg-inset text-fg-muted",
              )}>
              <Icon name={icon} size={18} />
            </span>
          )}
          <div className="flex min-w-0 flex-1 flex-col gap-1">
            <div className="flex min-h-7 flex-wrap items-center gap-x-2 gap-y-1">
              <h2 id={titleId} className="text-[16px] leading-6 font-semibold text-fg">
                {title}
              </h2>
              {badges}
            </div>
            {description !== undefined && (
              <p id={descriptionId} className="text-[12px] leading-5 text-fg-muted">
                {description}
              </p>
            )}
          </div>
          <IconButton icon="x" label={t("ui.drawer.close")} size={28} onClick={onClose} />
        </header>
        {children}
      </div>
    </div>
  );
}

/** The drawer's content; it scrolls when it does not fit. */
export function DrawerBody({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div className={cx("flex min-h-0 flex-1 flex-col gap-6 overflow-y-auto px-6 py-5", className)}>
      {children}
    </div>
  );
}

/** The drawer's actions, under its content, always in view. */
export function DrawerFooter({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div
      className={cx(
        "flex shrink-0 flex-wrap items-center gap-2 border-t border-border px-6 py-4",
        className,
      )}>
      {children}
    </div>
  );
}
