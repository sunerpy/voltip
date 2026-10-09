import type { ReactNode } from "react";
import { cx } from "../cx";
import { useT } from "../i18n/I18nProvider";
import { Icon } from "./Icon";
import { Keycap } from "./Keycap";
import { Lamp, type LampTone } from "./Lamp";

export interface ToolbarReadout {
  /** Localized label: 引擎 / 麦克风 / Engine / Microphone. */
  label: string;
  /** Human-readable value: `精确 · SenseVoice`, `Fifine K669`, `运行中 · 2 客户端`. */
  value: string;
  lamp?: LampTone;
  mono?: boolean;
  title?: string;
  /** Short mono tag after the value (`本地` / `Local` for an on-device engine). */
  badge?: string;
}

export interface ToolbarReadoutsProps {
  readouts: readonly ToolbarReadout[];
  /** Every value in mono, not only those flagged `mono`. */
  mono?: boolean;
  className?: string;
}

/** The readout strip: `中文标签 + 可读值 + 6 px status dot`, separated by 1 px hairlines. */
export function ToolbarReadouts({ readouts, mono = false, className }: ToolbarReadoutsProps) {
  return (
    <div className={cx("flex min-w-0 flex-1 items-center gap-4 overflow-hidden", className)}>
      {readouts.map((r, i) => (
        <span
          key={`${r.label}-${i}`}
          title={r.title}
          className={cx(
            "flex items-center gap-1.5 whitespace-nowrap text-[12px]",
            i > 0 && "border-l border-border pl-4",
          )}>
          <span className="text-fg-subtle">{r.label}</span>
          {r.lamp && <Lamp tone={r.lamp} size={6} />}
          <span className={cx("text-fg", (mono || r.mono) && "mono")}>{r.value}</span>
          {r.badge !== undefined && (
            <span className="mono rounded-6 bg-inset px-1 text-[10px] leading-4 text-fg-muted">
              {r.badge}
            </span>
          )}
        </span>
      ))}
    </div>
  );
}

export interface ToolbarSearchProps {
  onSearch: () => void;
  placeholder?: string;
  className?: string;
}

/** 220 px search field with the `Ctrl K` keycap; opens the command palette. */
export function ToolbarSearch({ onSearch, placeholder, className }: ToolbarSearchProps) {
  const t = useT();
  return (
    <button
      type="button"
      onClick={onSearch}
      className={cx(
        "flex h-7 w-[220px] shrink-0 items-center gap-2 rounded-6 bg-canvas px-2.5 text-[12px] text-fg-subtle hairline hover:border-fg-subtle",
        className,
      )}>
      <Icon name="search" size={13} />
      <span className="flex-1 truncate text-left">
        {placeholder ?? t("ui.toolbar.searchPlaceholder")}
      </span>
      <Keycap>Ctrl K</Keycap>
    </button>
  );
}

export interface ToolbarProps {
  title: ReactNode;
  readouts?: readonly ToolbarReadout[];
  onSearch?: () => void;
  searchPlaceholder?: string;
  /** Right-most slot (润色 chip on every board). */
  right?: ReactNode;
}

/** 40 px toolbar under a native title bar: page title, readouts, Ctrl K search, right slot.
 *  The desktop shell now draws `TitleBar` (the same strip as the window's own title bar); this
 *  stays for surfaces that keep native decorations. */
export function Toolbar({
  title,
  readouts = [],
  onSearch,
  searchPlaceholder,
  right,
}: ToolbarProps) {
  return (
    <header className="flex h-10 shrink-0 items-center gap-4 border-b border-border bg-surface px-6">
      <h1 className="text-[14px] font-semibold text-fg">{title}</h1>
      <ToolbarReadouts readouts={readouts} />
      {onSearch && <ToolbarSearch onSearch={onSearch} placeholder={searchPlaceholder} />}
      {right !== undefined && <div className="flex shrink-0 items-center">{right}</div>}
    </header>
  );
}
