// The toolbar over selected words in the history detail (docs/dictation.md §16.9 3.2, board 02 and
// 10): 「加入词典 D」 and 「新建规则 R」 8 px above the selection, centred on it, below it when the
// window has no room above. It never takes the focus, so the selection stays; a selection that
// cannot be a dictionary form disables 加入词典 and says why.
import { type DictionaryBlock, MAX_TERM_CHARS } from "@voltip/shared";
import { Icon, type IconName, Keycap, cx, useI18n } from "@voltip/ui";
import { useCallback, useId, useRef } from "react";
import { type FloatBox, useFloatingStyle } from "./floating";

export interface SelectionToolbarProps {
  /** The selection's box. */
  rect: FloatBox;
  /** Why 加入词典 is unavailable, when it is. */
  block?: DictionaryBlock | undefined;
  onDictionary: () => void;
  onRule: () => void;
}

export function SelectionToolbar({ rect, block, onDictionary, onRule }: SelectionToolbarProps) {
  const { t } = useI18n();
  const panel = useRef<HTMLDivElement>(null);
  const anchor = useCallback(() => rect, [rect]);
  const { style } = useFloatingStyle(panel, anchor, { side: "top", align: "center" });
  const reasonId = useId();
  const reason =
    block === "tooLong"
      ? t("history.fix.toolbar.tooLong", { max: MAX_TERM_CHARS })
      : block === "oneLine"
        ? t("history.fix.toolbar.oneLine")
        : undefined;
  return (
    <div
      ref={panel}
      role="toolbar"
      aria-label={t("history.fix.toolbar.label")}
      data-testid="fix-toolbar"
      data-tauri-drag-region="false"
      style={style}
      className="z-40 flex flex-col items-center gap-1.5"
      // The selection is what the buttons act on: a press must not move the focus or clear it.
      onMouseDown={(e) => {
        e.preventDefault();
      }}>
      <div className="flex items-center rounded-[12px] border border-raised-edge bg-raised p-1 shadow-pop glass-blur">
        <ToolbarButton
          icon="book"
          label={t("history.fix.toolbar.dictionary")}
          shortcut="D"
          disabled={reason !== undefined}
          describedBy={reason === undefined ? undefined : reasonId}
          onClick={onDictionary}
        />
        <span aria-hidden className="mx-0.5 h-[18px] w-px bg-border" />
        <ToolbarButton
          icon="sparkles"
          label={t("history.fix.toolbar.rule")}
          shortcut="R"
          onClick={onRule}
        />
      </div>
      {reason !== undefined && (
        <p
          id={reasonId}
          className="rounded-[9px] border border-raised-edge bg-raised px-2.5 py-1 text-[12px] text-fg-muted shadow-pop"
          data-testid="fix-toolbar-reason">
          {reason}
        </p>
      )}
    </div>
  );
}

function ToolbarButton({
  icon,
  label,
  shortcut,
  disabled = false,
  describedBy,
  onClick,
}: {
  icon: IconName;
  label: string;
  shortcut: string;
  disabled?: boolean;
  describedBy?: string | undefined;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      // Out of the Tab order: the keyboard has D and R, and the focus stays with the words.
      tabIndex={-1}
      aria-keyshortcuts={shortcut}
      aria-describedby={describedBy}
      disabled={disabled}
      onClick={onClick}
      className={cx(
        "flex h-[30px] items-center gap-1.5 rounded-[8px] px-2.5 text-[13px] font-medium text-fg transition-colors",
        disabled ? "cursor-not-allowed text-fg-subtle" : "hover:bg-accent-soft",
      )}>
      <Icon name={icon} size={14} className={disabled ? "text-fg-subtle" : "text-fg-muted"} />
      {label}
      <span aria-hidden>
        <Keycap className="h-4 min-w-4 px-1 text-[10px]">{shortcut}</Keycap>
      </span>
    </button>
  );
}
