import {
  type KeyboardEvent,
  type ReactNode,
  useCallback,
  useEffect,
  useId,
  useRef,
  useState,
} from "react";
import { cx } from "../cx";
import { Icon, type IconName } from "./Icon";

/** One row of a `Menu`: a choice of a set (`radio`) or a command (`action`). */
export type MenuItem =
  | {
      kind: "radio";
      id: string;
      label: string;
      checked: boolean;
      /** The label is the user's own text (a name they typed), not interface copy. */
      userText?: boolean;
    }
  | { kind: "action"; id: string; label: string; icon?: IconName };

/** Items under an optional heading; sections are separated by a rule. */
export interface MenuSection {
  label?: string;
  items: readonly MenuItem[];
}

export interface MenuProps {
  /** What the trigger button shows. */
  trigger: ReactNode;
  /** The menu's accessible name, and the trigger's unless `triggerLabel` is given. */
  label: string;
  triggerLabel?: string;
  sections: readonly MenuSection[];
  onSelect: (id: string) => void;
  /** Which edge of the trigger the menu lines up with. */
  align?: "start" | "end";
  triggerClassName?: string;
  title?: string;
  disabled?: boolean;
  "data-testid"?: string;
}

/** A button that opens a list of choices under it (the WAI-ARIA menu button): Enter, Space or ↓
 *  open it with the checked choice (else the first) focused; ↑ ↓ Home End move; Enter or a click
 *  picks; Esc, Tab or a click elsewhere close it and give the focus back to the button. Rows never
 *  wrap: the list is as wide as its longest row (docs/frontend.md, one line when there is room). */
export function Menu({
  trigger,
  label,
  triggerLabel,
  sections,
  onSelect,
  align = "start",
  triggerClassName,
  title,
  disabled = false,
  "data-testid": testId,
}: MenuProps) {
  const [open, setOpen] = useState(false);
  const menuId = useId();
  const wrapper = useRef<HTMLDivElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const items = useRef<(HTMLButtonElement | null)[]>([]);
  /** The row the menu focuses once it is open: the checked choice, else the first. */
  const focusOnOpen = useRef(0);
  const flat = sections.flatMap((s) => s.items);

  const close = useCallback((refocus: boolean) => {
    setOpen(false);
    if (refocus) button.current?.focus();
  }, []);

  const openMenu = () => {
    const checked = flat.findIndex((i) => i.kind === "radio" && i.checked);
    focusOnOpen.current = checked >= 0 ? checked : 0;
    setOpen(true);
  };

  useEffect(() => {
    if (open) items.current[focusOnOpen.current]?.focus();
  }, [open]);

  // A press anywhere else closes it (without taking the focus back).
  useEffect(() => {
    if (!open) return;
    const onPointer = (e: PointerEvent) => {
      if (!(e.target instanceof Node) || !wrapper.current?.contains(e.target)) close(false);
    };
    document.addEventListener("pointerdown", onPointer, true);
    return () => {
      document.removeEventListener("pointerdown", onPointer, true);
    };
  }, [open, close]);

  const move = (from: number, delta: number) => {
    const n = flat.length;
    if (n === 0) return;
    items.current[(from + delta + n) % n]?.focus();
  };

  const onMenuKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const index = items.current.findIndex((el) => el === document.activeElement);
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        move(index, 1);
        break;
      case "ArrowUp":
        e.preventDefault();
        move(index < 0 ? 0 : index, -1);
        break;
      case "Home":
        e.preventDefault();
        items.current[0]?.focus();
        break;
      case "End":
        e.preventDefault();
        items.current[flat.length - 1]?.focus();
        break;
      case "Escape":
        // Handled here: a dialog under the menu must not close with it.
        e.preventDefault();
        e.stopPropagation();
        close(true);
        break;
      case "Tab":
        close(false);
        break;
    }
  };

  // Each section's first row in `flat` (the arrow keys move across sections).
  const starts = sections.map((_, s) =>
    sections.slice(0, s).reduce((n, section) => n + section.items.length, 0),
  );
  return (
    <div ref={wrapper} className="relative inline-flex">
      <button
        ref={button}
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        aria-label={triggerLabel ?? label}
        title={title}
        disabled={disabled}
        data-testid={testId}
        onClick={() => {
          if (open) close(false);
          else openMenu();
        }}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown" && !open) {
            e.preventDefault();
            openMenu();
          }
        }}
        className={triggerClassName}>
        {trigger}
      </button>
      {open && (
        <div
          id={menuId}
          role="menu"
          aria-label={label}
          // Inside the title bar's drag region: a press on a heading or the padding must not move
          // the window (Tauri's drag script stops at "false").
          data-tauri-drag-region="false"
          onKeyDown={onMenuKey}
          data-testid={testId === undefined ? undefined : `${testId}-menu`}
          className={cx(
            "absolute top-full z-50 mt-1 flex w-max min-w-full flex-col rounded-10 bg-surface py-1 whitespace-nowrap shadow-win hairline",
            align === "end" ? "right-0" : "left-0",
          )}>
          {sections.map((section, s) => (
            <div
              key={section.label ?? `section-${s}`}
              role="group"
              aria-label={section.label}
              className={cx(s > 0 && "mt-1 border-t border-border pt-1")}>
              {section.label !== undefined && (
                <div aria-hidden className="px-3 pt-1 pb-0.5 text-[11px] text-fg-subtle">
                  {section.label}
                </div>
              )}
              {section.items.map((item, i) => {
                const at = (starts[s] ?? 0) + i;
                return (
                  <button
                    key={item.id}
                    ref={(el) => {
                      items.current[at] = el;
                    }}
                    type="button"
                    role={item.kind === "radio" ? "menuitemradio" : "menuitem"}
                    aria-checked={item.kind === "radio" ? item.checked : undefined}
                    tabIndex={-1}
                    onClick={() => {
                      close(true);
                      onSelect(item.id);
                    }}
                    className="flex h-8 w-full items-center gap-2 px-3 text-left text-[13px] text-fg outline-none hover:bg-inset focus-visible:bg-inset focus:bg-inset">
                    <span className="flex w-4 shrink-0 justify-center text-accent-text">
                      {item.kind === "radio" && item.checked && <Icon name="check" size={14} />}
                      {item.kind === "action" && item.icon !== undefined && (
                        <Icon name={item.icon} size={14} className="text-fg-muted" />
                      )}
                    </span>
                    <span
                      {...(item.kind === "radio" && item.userText ? { "data-user-text": "" } : {})}>
                      {item.label}
                    </span>
                  </button>
                );
              })}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
