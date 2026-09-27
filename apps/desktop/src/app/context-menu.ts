export interface ContextMenuPolicyOptions {
  /** `true` inside the Tauri webview; the browser dev preview keeps its native menu. */
  enabled: boolean;
}

/** Targets that keep the native context menu: text fields (cut / copy / paste / spell-check) and
 *  anything a page opts in with `data-allow-context-menu`. */
const ALLOWED_TARGETS = 'input, textarea, [contenteditable="true"], [data-allow-context-menu]';

/** Whether a right-click on `target` may open the browser's context menu. */
export function allowsContextMenu(target: EventTarget | null): boolean {
  const element =
    target instanceof Element ? target : target instanceof Node ? target.parentElement : null;
  if (element === null) return false;
  return element.closest(ALLOWED_TARGETS) !== null;
}

function onContextMenu(event: MouseEvent): void {
  if (!allowsContextMenu(event.target)) event.preventDefault();
}

/** Windows test 2026-09-25: the client showed the webview's "Back / Reload / Inspect" menu on
 *  every right-click, which reads as a web page, not a desktop app. When enabled, the policy
 *  cancels `contextmenu` everywhere except on editable targets; returns the uninstaller. */
export function installContextMenuPolicy(
  doc: Document,
  { enabled }: ContextMenuPolicyOptions,
): () => void {
  if (!enabled) return () => undefined;
  doc.addEventListener("contextmenu", onContextMenu);
  return () => {
    doc.removeEventListener("contextmenu", onContextMenu);
  };
}
