import { type ToastItem, cx } from "@voltip/ui";

/** The phone's toasts: the desktop's card (surface, hairline, the drain bar), the width of the
 *  screen less its 16 px margins, above the tab bar when there is one; a message wraps instead of
 *  being cut. Newest at the bottom, three at most. */
export function PhoneToasts({
  toasts,
  aboveTabBar,
}: {
  toasts: readonly ToastItem[];
  aboveTabBar: boolean;
}) {
  const visible = toasts.slice(-3);
  if (visible.length === 0) return null;
  return (
    <div
      className={cx(
        "pointer-events-none fixed inset-x-0 z-40 mx-auto flex max-w-[430px] flex-col gap-2 px-4",
        aboveTabBar
          ? "bottom-[calc(env(safe-area-inset-bottom)+4.5rem)]"
          : "bottom-[calc(env(safe-area-inset-bottom)+1rem)]",
      )}>
      {visible.map((toast) => (
        <div
          key={toast.id}
          role={toast.tone === "danger" ? "alert" : "status"}
          data-tone={toast.tone}
          className="relative overflow-hidden rounded-10 bg-surface px-4 py-3 text-[14px] leading-5 break-words hairline shadow-pop">
          <span className={toast.tone === "danger" ? "text-danger" : "text-fg"}>
            {toast.message}
          </span>
          <span
            aria-hidden
            className="absolute bottom-0 left-0 h-0.5 bg-primary"
            style={{ animation: `vt-drain ${toast.duration}ms linear forwards` }}
          />
        </div>
      ))}
    </div>
  );
}
