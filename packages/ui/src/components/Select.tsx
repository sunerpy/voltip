import {
  type CSSProperties,
  type KeyboardEvent as ReactKeyboardEvent,
  type SelectHTMLAttributes,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { cx } from "../cx";
import { usePresentation } from "../presentation/PresentationProvider";
import { NO_TAP_HIGHLIGHT, TOUCH_CONTROL, TOUCH_TARGET } from "../presentation/touch";
import { Icon } from "./Icon";
import { openLayer } from "./layers";
import { type ListPlacement, placeList, revealScrollTop } from "./placement";

export interface SelectOption<V extends string> {
  value: V;
  label: string;
  disabled?: boolean;
}

export interface SelectProps<V extends string> extends Omit<
  SelectHTMLAttributes<HTMLSelectElement>,
  "onChange" | "value" | "size"
> {
  options: readonly SelectOption<V>[];
  value: V;
  onChange: (value: V) => void;
  mono?: boolean;
  label?: string;
  size?: "sm" | "md";
}

/** A select with the design's chrome: hairline, radius 6, chevron, optional mono value. Under
 *  `PresentationProvider value="touch"` (the phone) it opens a list of its own instead of the
 *  platform's picker; otherwise it is a native `<select>`.
 *  It is never narrower than its longest option (user feedback 2026-09-29: callers' fixed widths
 *  clipped the chosen label): a hidden copy of every label shares the select's grid cell and sets
 *  the column's width. CSS `field-sizing` would do the same, but WebKitGTK and WKWebView do not
 *  all support it. Callers pass no width; a container narrower than the longest label still wins. */
export function Select<V extends string>(props: SelectProps<V>) {
  return usePresentation() === "touch" ? <TouchSelect {...props} /> : <NativeSelect {...props} />;
}

/** The props of `props` that are (`markers: true`) or are not the markers that say what the
 *  options are, a user's device names or endonyms. The markers go wherever the labels are shown,
 *  so the language scans treat every copy alike. */
function pickProps(props: object, markers: boolean) {
  return Object.fromEntries(
    Object.entries(props).filter(
      ([key]) => (key === "data-user-text" || key === "data-endonyms") === markers,
    ),
  );
}

/** The hidden copy of every label, one per line, that sets the grid column's width. */
function Sizer<V extends string>({
  options,
  size,
  mono,
  markers,
}: {
  options: readonly SelectOption<V>[];
  size: "sm" | "md";
  mono: boolean;
  markers: ReturnType<typeof pickProps>;
}) {
  return (
    <span
      aria-hidden
      data-select-sizer=""
      {...markers}
      className={cx(
        "invisible col-start-1 row-start-1 flex h-0 min-w-max flex-col overflow-hidden border border-transparent pr-7 pl-2.5 whitespace-nowrap",
        size === "sm" ? "text-[12px]" : "text-[13px]",
        mono && "mono",
      )}>
      {options.map((o) => (
        <span key={o.value}>{o.label}</span>
      ))}
    </span>
  );
}

function NativeSelect<V extends string>({
  options,
  value,
  onChange,
  mono = false,
  label,
  size = "md",
  className,
  id,
  ...rest
}: SelectProps<V>) {
  const autoId = useId();
  const selectId = id ?? autoId;
  const markers = pickProps(rest, true);
  return (
    <div className={cx("flex flex-col gap-1", className)}>
      {label && (
        <label htmlFor={selectId} className="text-[12px] text-fg-muted">
          {label}
        </label>
      )}
      <div className="relative grid">
        <Sizer options={options} size={size} mono={mono} markers={markers} />
        <select
          id={selectId}
          value={value}
          onChange={(e) => {
            const next = options.find((o) => o.value === e.target.value);
            if (next) onChange(next.value);
          }}
          className={cx(
            "col-start-1 row-start-1 w-full min-w-0 appearance-none rounded-6 bg-surface pr-7 pl-2.5 hairline outline-none transition-colors hover:border-fg-subtle focus:border-fg disabled:opacity-50 disabled:hover:border-border",
            size === "sm" ? "h-7 text-[12px]" : "h-8 text-[13px]",
            mono && "mono",
          )}
          {...rest}>
          {options.map((o) => (
            <option key={o.value} value={o.value} disabled={o.disabled}>
              {o.label}
            </option>
          ))}
        </select>
        <Icon
          name="chevronDown"
          size={14}
          className="pointer-events-none absolute top-1/2 right-2 -translate-y-1/2 text-fg-subtle"
        />
      </div>
    </div>
  );
}

function placementStyle(placement: ListPlacement | undefined): CSSProperties {
  // Measured first, unseen and unconstrained: the natural size `placeList` starts from.
  if (placement === undefined) return { visibility: "hidden", top: 0, left: 0 };
  const { side, left, minWidth, maxWidth, maxHeight } = placement;
  return {
    ...(side === "below" ? { top: placement.top } : { bottom: placement.bottom }),
    left,
    minWidth,
    maxWidth,
    maxHeight,
  };
}

/** The touch presentation: the same trigger (a button that says `aria-haspopup="listbox"`), and a
 *  list of options drawn like `Menu` on a hairline and `shadow-pop`, instead of Android's picker (a
 *  dialog of radio buttons). The list opens under the trigger, or above it when there is no room
 *  below, never past the screen's edges, and scrolls past `LIST_MAX_HEIGHT` with the chosen row in
 *  view. ↑ ↓ Home End move over the rows that can be chosen, Enter, Space or a tap choose one; Esc,
 *  Tab, a tap outside it and the phone's system back (`dismissTopDialog`, the stack the dialogs use)
 *  close it, and the focus goes back to the trigger. `onChange` hears only of a new choice. */
function TouchSelect<V extends string>({
  options,
  value,
  onChange,
  mono = false,
  label,
  size = "md",
  className,
  id,
  ...rest
}: SelectProps<V>) {
  const autoId = useId();
  const triggerId = id ?? autoId;
  const labelId = `${triggerId}-label`;
  const valueId = `${triggerId}-value`;
  const listId = `${triggerId}-list`;
  const markers = pickProps(rest, true);
  // Everything else a caller passes (`aria-label`, `data-testid`, `disabled`, …) is the trigger's.
  const forwarded = pickProps(rest, false);
  const disabled = rest.disabled === true;
  const named =
    Boolean(label) || rest["aria-label"] !== undefined || rest["aria-labelledby"] !== undefined;

  const [open, setOpen] = useState(false);
  // Disabled while open (a recording starts): the list closes, and stays closed once enabled.
  if (open && disabled) setOpen(false);
  const [placement, setPlacement] = useState<ListPlacement | undefined>(undefined);
  const wrapper = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const rows = useRef<(HTMLButtonElement | null)[]>([]);
  /** The row to focus once the list is placed; -1 for the list itself (no row can be chosen). */
  const focusOnOpen = useRef<number | undefined>(undefined);

  const choosable = (at: number) => {
    const option = options[at];
    return option !== undefined && option.disabled !== true;
  };
  const selected = options.findIndex((o) => o.value === value);
  const firstChoosable = options.findIndex((o) => o.disabled !== true);
  // What a native select shows: the option with the value, else the first one that can be chosen.
  const current = selected >= 0 ? selected : firstChoosable;

  const close = useCallback((refocus: boolean) => {
    setOpen(false);
    if (refocus) trigger.current?.focus();
  }, []);

  const openList = () => {
    focusOnOpen.current = choosable(current) ? current : firstChoosable;
    setPlacement(undefined);
    setOpen(true);
  };

  const choose = (at: number, next: V) => {
    close(true);
    if (at !== current) onChange(next);
  };

  /** Focuses a row and scrolls the list just enough to show it (`center`: into the middle). */
  const focusRow = useCallback((at: number, mode: "nearest" | "center") => {
    const row = rows.current[at];
    const box = list.current;
    if (!row || !box) return;
    row.focus({ preventScroll: true });
    box.scrollTop = revealScrollTop(
      { top: row.offsetTop, height: row.offsetHeight },
      { scrollTop: box.scrollTop, height: box.clientHeight, scrollHeight: box.scrollHeight },
      mode,
    );
  }, []);

  // Measures the list once it is open (unseen), places it, and follows the trigger when the page
  // scrolls or the window resizes (the keyboard closing, a rotation).
  useLayoutEffect(() => {
    const box = list.current;
    const anchor = trigger.current;
    if (!open || !box || !anchor) return;
    const content = { width: box.offsetWidth, height: box.scrollHeight };
    const place = () => {
      setPlacement(
        placeList({
          trigger: anchor.getBoundingClientRect(),
          viewport: { width: window.innerWidth, height: window.innerHeight },
          content,
        }),
      );
    };
    place();
    const onScroll = (e: Event) => {
      // The list scrolling through its own rows moves nothing.
      if (!e.composedPath().includes(box)) place();
    };
    window.addEventListener("resize", place);
    window.addEventListener("scroll", onScroll, true);
    return () => {
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", onScroll, true);
    };
  }, [open]);

  // Once placed, the chosen row takes the focus, in the middle of the list.
  useLayoutEffect(() => {
    const at = focusOnOpen.current;
    if (placement === undefined || at === undefined) return;
    focusOnOpen.current = undefined;
    if (at >= 0) focusRow(at, "center");
    else list.current?.focus({ preventScroll: true });
  }, [placement, focusRow]);

  // On the stack the dialogs use: Esc and the system back close the list before the dialog it
  // opened in. A press outside it closes it too.
  useEffect(() => {
    const root = wrapper.current;
    if (!open || !root) return;
    const layer = openLayer(() => {
      close(true);
    });
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || !layer.isTop()) return;
      // On `window`, in the capture phase: before a dialog's own listener on `document`.
      e.preventDefault();
      e.stopPropagation();
      close(true);
    };
    const onPointer = (e: PointerEvent) => {
      if (!e.composedPath().includes(root)) close(false);
    };
    window.addEventListener("keydown", onKey, true);
    document.addEventListener("pointerdown", onPointer, true);
    return () => {
      layer.remove();
      window.removeEventListener("keydown", onKey, true);
      document.removeEventListener("pointerdown", onPointer, true);
    };
  }, [open, close]);

  /** The next row from `from` in the direction of `step` that can be chosen; none past the ends. */
  const nextChoosable = (from: number, step: 1 | -1) => {
    for (let at = from + step; at >= 0 && at < options.length; at += step)
      if (choosable(at)) return at;
    return undefined;
  };

  const onListKey = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    const at = rows.current.findIndex((row) => row !== null && row === document.activeElement);
    let next: number | undefined;
    switch (e.key) {
      case "ArrowDown":
        next = nextChoosable(at, 1);
        break;
      case "ArrowUp":
        next = nextChoosable(at < 0 ? options.length : at, -1);
        break;
      case "Home":
        next = nextChoosable(-1, 1);
        break;
      case "End":
        next = nextChoosable(options.length, -1);
        break;
      default:
        return;
    }
    e.preventDefault();
    if (next !== undefined) focusRow(next, "nearest");
  };

  return (
    <div className={cx("flex flex-col gap-1", className, NO_TAP_HIGHLIGHT)}>
      {label && (
        <label id={labelId} htmlFor={triggerId} className="text-[12px] text-fg-muted">
          {label}
        </label>
      )}
      <div
        ref={wrapper}
        className="relative grid"
        onKeyDown={(e) => {
          // Tab leaves from the trigger: the focus moves on to whatever follows the select.
          if (e.key === "Tab" && open) close(true);
        }}>
        <Sizer options={options} size={size} mono={mono} markers={markers} />
        <button
          {...forwarded}
          ref={trigger}
          id={triggerId}
          type="button"
          disabled={disabled}
          aria-haspopup="listbox"
          aria-expanded={open}
          aria-controls={open ? listId : undefined}
          // The chosen option: a label names the button, so its text would go unread.
          aria-describedby={cx(named && valueId, rest["aria-describedby"]) || undefined}
          onClick={() => {
            if (open) close(true);
            else openList();
          }}
          onKeyDown={(e) => {
            if ((e.key === "ArrowDown" || e.key === "ArrowUp") && !open) {
              e.preventDefault();
              openList();
            }
          }}
          className={cx(
            "col-start-1 row-start-1 flex w-full min-w-0 items-center rounded-6 bg-surface pr-7 pl-2.5 text-left hairline outline-none transition-colors select-none hover:border-fg-subtle focus:border-fg disabled:opacity-50 disabled:hover:border-border aria-expanded:border-fg",
            size === "sm" ? "h-7 text-[12px]" : "h-8 text-[13px]",
            mono && "mono",
            TOUCH_CONTROL,
            TOUCH_TARGET,
            !disabled && "active:bg-inset",
          )}>
          <span id={valueId} className="min-w-0 truncate" {...markers}>
            {options[current]?.label}
          </span>
        </button>
        <Icon
          name="chevronDown"
          size={14}
          className="pointer-events-none absolute top-1/2 right-2 -translate-y-1/2 text-fg-subtle"
        />
        {open && (
          <>
            {/* Takes the tap that closes the list, so it does not also press what lies under it
                (Android's picker is modal too). Keeps the focus in the list until then. */}
            <div
              aria-hidden
              data-select-backdrop=""
              className="fixed inset-0 z-50"
              onMouseDown={(e) => {
                e.preventDefault();
              }}
              onClick={() => {
                close(true);
              }}
            />
            <div
              ref={list}
              id={listId}
              role="listbox"
              tabIndex={-1}
              aria-labelledby={label ? labelId : rest["aria-labelledby"]}
              aria-label={label ? undefined : rest["aria-label"]}
              onKeyDown={onListKey}
              style={placementStyle(placement)}
              className="fixed z-50 w-max overflow-y-auto overscroll-contain rounded-10 bg-surface py-1 font-ui whitespace-normal text-fg outline-none hairline shadow-pop">
              {options.map((o, i) => {
                const off = o.disabled === true;
                return (
                  <button
                    key={o.value}
                    ref={(el) => {
                      rows.current[i] = el;
                    }}
                    type="button"
                    role="option"
                    // What `user.selectOptions(listbox, value)` matches in a test.
                    value={o.value}
                    aria-selected={i === current}
                    aria-disabled={off ? true : undefined}
                    disabled={off}
                    tabIndex={-1}
                    onClick={
                      off
                        ? undefined
                        : () => {
                            choose(i, o.value);
                          }
                    }
                    // 44 px in pixels: rem follows the type-size setting (13 px by default).
                    className={cx(
                      "flex min-h-[44px] w-full items-center gap-2 px-3 py-2 text-left text-[13px] outline-none touch-manipulation select-none",
                      off
                        ? "cursor-default text-fg-subtle"
                        : "text-fg focus:bg-inset active:bg-inset2",
                    )}>
                    <span className="flex w-4 shrink-0 justify-center text-accent-text">
                      {i === current && <Icon name="check" size={14} />}
                    </span>
                    <span className={cx("min-w-0 break-words", mono && "mono")} {...markers}>
                      {o.label}
                    </span>
                  </button>
                );
              })}
            </div>
          </>
        )}
      </div>
    </div>
  );
}
