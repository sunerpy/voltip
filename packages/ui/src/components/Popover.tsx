import { type ReactNode, useCallback, useEffect, useId, useRef, useState } from "react";
import { cx } from "../cx";

export interface PopoverProps {
  /** What the trigger button shows (a short word such as 「依据」). */
  trigger: ReactNode;
  /** The panel's accessible name, and the trigger's unless `triggerLabel` is given. */
  label: string;
  triggerLabel?: string;
  /** The explanation: prose, which may wrap. */
  children: ReactNode;
  /** Which edge of the trigger the panel lines up with. */
  align?: "start" | "end";
  triggerClassName?: string;
  "data-testid"?: string;
}

/** A button that shows a short explanation under it (a disclosure, drawn as a floating panel):
 *  a click, Enter or Space open and close it; Esc closes it and gives the focus back to the
 *  button; Tab or a click elsewhere close it. The panel is a non-modal dialog named `label`. */
export function Popover({
  trigger,
  label,
  triggerLabel,
  children,
  align = "start",
  triggerClassName,
  "data-testid": testId,
}: PopoverProps) {
  const [open, setOpen] = useState(false);
  const panelId = useId();
  const wrapper = useRef<HTMLDivElement>(null);
  const button = useRef<HTMLButtonElement>(null);

  const close = useCallback((refocus: boolean) => {
    setOpen(false);
    if (refocus) button.current?.focus();
  }, []);

  useEffect(() => {
    if (!open) return;
    const onPointer = (e: PointerEvent) => {
      if (!(e.target instanceof Node) || !wrapper.current?.contains(e.target)) close(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        close(true);
      }
    };
    document.addEventListener("pointerdown", onPointer, true);
    // On `window`: its capture phase runs before a `Dialog`'s listener on `document`, so the Esc
    // that closes this panel does not also close the dialog it sits in.
    window.addEventListener("keydown", onKey, true);
    return () => {
      document.removeEventListener("pointerdown", onPointer, true);
      window.removeEventListener("keydown", onKey, true);
    };
  }, [open, close]);

  return (
    <div
      ref={wrapper}
      className="relative inline-flex"
      onKeyDown={(e) => {
        if (e.key === "Tab" && open) close(false);
      }}>
      <button
        ref={button}
        type="button"
        aria-expanded={open}
        aria-controls={open ? panelId : undefined}
        aria-label={triggerLabel ?? label}
        data-testid={testId}
        onClick={() => {
          setOpen((was) => !was);
        }}
        className={triggerClassName}>
        {trigger}
      </button>
      {open && (
        <div
          id={panelId}
          role="dialog"
          aria-label={label}
          data-tauri-drag-region="false"
          data-testid={testId === undefined ? undefined : `${testId}-panel`}
          className={cx(
            "absolute top-full z-50 mt-1 w-80 max-w-[calc(100vw-2rem)] rounded-10 bg-surface p-3 text-[12px] leading-5 text-fg shadow-win hairline",
            align === "end" ? "right-0" : "left-0",
          )}>
          {children}
        </div>
      )}
    </div>
  );
}
