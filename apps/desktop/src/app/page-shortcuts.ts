import { useEffect, useRef } from "react";

/** A modal is over the page (settings, the command palette, a confirm): page keys stand down. The
 *  settings dialog keeps the page beneath mounted as its inert background, so its listener is still
 *  attached and must not act. Every modal of the app carries `aria-modal="true"`. */
export function modalOpen(doc: Document = document): boolean {
  return doc.querySelector('[aria-modal="true"]') !== null;
}

/** Focus is in a field that owns its keys (Enter, Esc, Delete, Ctrl C, Ctrl F …). */
export function inTextField(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || target.closest("input, textarea, select") !== null;
}

/** Focus is on a control that answers Enter / Space itself (a button, a link, a tab, a switch). */
export function onControl(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return (
    target.closest(
      'button, a[href], summary, [role="button"], [role="tab"], [role="radio"], [role="switch"], [role="option"], [role="checkbox"]',
    ) !== null
  );
}

/** Ctrl on Windows / Linux, ⌘ on macOS: both count, like the global shortcuts in `Shell`. */
export function withCommand(e: KeyboardEvent): boolean {
  return e.ctrlKey || e.metaKey;
}

/** The page's own keyboard shortcuts (the ones its footer lists). `handler` returns `true` when it
 *  used the key, which then does nothing else (no browser find, reload or copy). Skipped while a
 *  modal is open and for key repeats. */
export function usePageShortcuts(handler: (e: KeyboardEvent) => boolean): void {
  const latest = useRef(handler);
  useEffect(() => {
    latest.current = handler;
  });
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.repeat || modalOpen()) return;
      if (latest.current(e)) e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  }, []);
}
