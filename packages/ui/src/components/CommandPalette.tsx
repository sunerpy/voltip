import { type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import { cx } from "../cx";
import { useT } from "../i18n/I18nProvider";
import { Icon, type IconName } from "./Icon";
import { Keycap, Keycaps } from "./Keycap";

export interface CommandItem {
  id: string;
  group: string;
  label: string;
  icon?: IconName;
  /** Keyboard shortcut rendered as keycaps, e.g. `Ctrl Alt Space`. */
  keys?: string;
  /** Mono hint on the right when there is no shortcut (`当前`, `系统：明亮`). */
  hint?: string;
  disabled?: boolean;
  disabledHint?: string;
  run: () => void;
}

export interface CommandPaletteProps {
  open: boolean;
  items: readonly CommandItem[];
  onClose: () => void;
  /** Called as the highlighted item changes (theme preview). */
  onHighlight?: (item: CommandItem | undefined) => void;
  placeholder?: string;
}

export function filterCommands(items: readonly CommandItem[], query: string): CommandItem[] {
  const q = query.trim().toLowerCase();
  if (q.length === 0) return [...items];
  return items.filter((i) => `${i.group} ${i.label} ${i.hint ?? ""}`.toLowerCase().includes(q));
}

function Highlight({ text, query }: { text: string; query: string }): ReactNode {
  const q = query.trim();
  if (q.length === 0) return text;
  const idx = text.toLowerCase().indexOf(q.toLowerCase());
  if (idx < 0) return text;
  return (
    <>
      {text.slice(0, idx)}
      <span className="underline decoration-fg decoration-1 underline-offset-2">
        {text.slice(idx, idx + q.length)}
      </span>
      {text.slice(idx + q.length)}
    </>
  );
}

/** Ctrl K palette: 560 wide, grouped items, ↑↓ Enter Tab Esc, 28 % scrim. Remounts on open so
 *  query and cursor start fresh every time. */
export function CommandPalette(props: CommandPaletteProps) {
  if (!props.open) return null;
  return <PaletteBody {...props} />;
}

function PaletteBody({ items, onClose, onHighlight, placeholder }: CommandPaletteProps) {
  const t = useT();
  const [query, setQuery] = useState("");
  const [cursor, setCursor] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const results = useMemo(() => filterCommands(items, query), [items, query]);
  const groups = useMemo(() => {
    const order: string[] = [];
    for (const r of results) if (!order.includes(r.group)) order.push(r.group);
    return order;
  }, [results]);
  const current = results[Math.min(cursor, Math.max(0, results.length - 1))];

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  useEffect(() => {
    onHighlight?.(current);
  }, [current, onHighlight]);

  const move = (delta: number) => {
    if (results.length === 0) return;
    setCursor((c) => (c + delta + results.length) % results.length);
  };
  const jumpGroup = () => {
    if (!current) return;
    const idx = groups.indexOf(current.group);
    const nextGroup = groups[(idx + 1) % groups.length];
    const nextIndex = results.findIndex((r) => r.group === nextGroup);
    if (nextIndex >= 0) setCursor(nextIndex);
  };
  const run = (item: CommandItem | undefined) => {
    if (!item || item.disabled) return;
    item.run();
    onClose();
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center pt-24 scrim"
      onClick={onClose}
      data-testid="palette-scrim">
      <div
        role="dialog"
        aria-modal="true"
        aria-label={t("ui.palette.label")}
        onClick={(e) => {
          e.stopPropagation();
        }}
        className="flex w-[560px] max-h-[424px] flex-col overflow-hidden rounded-14 bg-surface hairline shadow-pop"
        onKeyDown={(e) => {
          if (e.key === "ArrowDown") {
            e.preventDefault();
            move(1);
          } else if (e.key === "ArrowUp") {
            e.preventDefault();
            move(-1);
          } else if (e.key === "Tab") {
            e.preventDefault();
            jumpGroup();
          } else if (e.key === "Enter") {
            e.preventDefault();
            run(current);
          } else if (e.key === "Escape") {
            e.preventDefault();
            onClose();
          }
        }}>
        <div className="flex h-14 items-center gap-3 border-b border-border px-4">
          <Icon name="search" size={16} className="text-fg-subtle" />
          <input
            ref={inputRef}
            role="combobox"
            aria-expanded="true"
            aria-controls="vt-palette-list"
            aria-activedescendant={current ? `vt-cmd-${current.id}` : undefined}
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setCursor(0);
            }}
            placeholder={placeholder ?? t("ui.palette.placeholder")}
            className="flex-1 bg-transparent text-[15px] outline-none placeholder:text-fg-subtle"
          />
          <Keycap>Esc</Keycap>
        </div>
        <ul id="vt-palette-list" role="listbox" className="flex-1 overflow-y-auto py-1">
          {results.length === 0 && (
            <li className="flex flex-col items-center gap-1 py-10 text-center">
              <span className="text-[13px] text-fg-subtle">{t("ui.palette.noMatch")}</span>
              <span className="mono text-[11px] text-fg-subtle">{t("ui.palette.tryHint")}</span>
            </li>
          )}
          {groups.map((g) => (
            <li key={g}>
              <div className="eyebrow px-4 pt-2 pb-1 text-[10px]">{g}</div>
              <ul>
                {results
                  .filter((r) => r.group === g)
                  .map((item) => {
                    const active = item.id === current?.id;
                    return (
                      <li
                        key={item.id}
                        id={`vt-cmd-${item.id}`}
                        role="option"
                        aria-selected={active}
                        aria-disabled={item.disabled || undefined}
                        onMouseEnter={() => {
                          setCursor(results.indexOf(item));
                        }}
                        onClick={() => {
                          run(item);
                        }}
                        className={cx(
                          "mx-2 flex h-10 cursor-pointer items-center gap-3 rounded-6 px-2 text-[13px]",
                          active && "bg-canvas shadow-[inset_2px_0_0_var(--primary)]",
                          item.disabled ? "cursor-not-allowed text-fg-subtle" : "text-fg",
                        )}>
                        {item.icon && <Icon name={item.icon} size={15} className="text-fg-muted" />}
                        <span className="flex-1 truncate">
                          <Highlight text={item.label} query={query} />
                        </span>
                        {item.disabled && item.disabledHint ? (
                          <span className="mono text-[11px] text-fg-subtle">
                            {item.disabledHint}
                          </span>
                        ) : item.keys ? (
                          <Keycaps keys={item.keys} />
                        ) : item.hint ? (
                          <span className="mono text-[11px] text-fg-muted">{item.hint}</span>
                        ) : null}
                      </li>
                    );
                  })}
              </ul>
            </li>
          ))}
        </ul>
        <div className="mono flex h-8 items-center justify-between border-t border-border px-4 text-[10px] text-fg-subtle">
          <span>{t("ui.palette.footer")}</span>
          <span>{t("ui.palette.results", { n: results.length })}</span>
        </div>
      </div>
    </div>
  );
}
