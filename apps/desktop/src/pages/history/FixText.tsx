// The detail's text as the quick fix shows it (docs/dictation.md §16.9 3.1, 3.5, 3.7): the original
// with its marks (buttons in the text, in the Tab order after the view switch), the kept selection
// drawn as a highlight once the focus moved to the card, and the diff view's replacements as
// buttons. Every piece of text is a leaf with `data-at` (text-offsets.ts), so a selection maps back
// to offsets into the entry's text.
import type { DiffItem, PolishPair, TextMark, TextRange } from "@voltip/shared";
import { cx } from "@voltip/ui";
import type { KeyboardEvent, ReactNode } from "react";
import { LEAF_ATTR } from "./text-offsets";

/** The kept selection (board 10: `#CFE2FF` in 澄空), from the theme's primary colour. */
export const HIGHLIGHT_CLASS =
  "rounded-[3px] bg-[color-mix(in_srgb,var(--primary)_18%,transparent)]";

/** A mark: a button in the text that keeps wrapping like text (a `<button>` would not). */
const MARK_CLASS =
  "cursor-pointer rounded-[5px] underline underline-offset-[5px] box-decoration-clone focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary";
const MARK_KIND_CLASS = {
  // 2 px dots in the primary colour; the hover tint is visible on the inset field (§16.9 3.1).
  polish:
    "decoration-dotted decoration-2 decoration-primary hover:bg-[color-mix(in_srgb,var(--primary)_12%,transparent)]",
  known:
    "decoration-solid decoration-[1.5px] decoration-ok-text hover:bg-[color-mix(in_srgb,var(--ok)_18%,transparent)]",
} as const;

/** Enter and Space press a mark or a replacement, as they press a button. */
function pressKeys(press: () => void) {
  return (e: KeyboardEvent<HTMLElement>) => {
    if (e.key !== "Enter" && e.key !== " ") return;
    e.preventDefault();
    e.stopPropagation();
    press();
  };
}

/** A click that ends a drag over the words is a selection, not a press. */
function draggedOver(): boolean {
  return window.getSelection()?.isCollapsed === false;
}

interface Leaf extends TextRange {
  mark?: TextMark;
  lit: boolean;
}

/** The text cut at every mark's and the highlight's ends. */
function leavesOf(
  text: string,
  marks: readonly TextMark[],
  highlight: TextRange | undefined,
): Leaf[] {
  const cuts = new Set<number>([0, text.length]);
  for (const m of marks) {
    cuts.add(m.start);
    cuts.add(m.end);
  }
  if (highlight !== undefined) {
    cuts.add(highlight.start);
    cuts.add(highlight.end);
  }
  const points = [...cuts].filter((n) => n >= 0 && n <= text.length);
  points.sort((a, b) => a - b);
  const leaves: Leaf[] = [];
  for (let i = 0; i + 1 < points.length; i += 1) {
    const start = points[i] ?? 0;
    const end = points[i + 1] ?? start;
    const mark = marks.find((m) => m.start <= start && end <= m.end);
    const lit = highlight !== undefined && highlight.start <= start && end <= highlight.end;
    leaves.push({ start, end, ...(mark === undefined ? {} : { mark }), lit });
  }
  return leaves;
}

export interface MarkedTextProps {
  text: string;
  marks: readonly TextMark[];
  /** The kept selection, drawn once the native one is gone. */
  highlight?: TextRange | undefined;
  /** The mark's accessible name and its tooltip. */
  label: (mark: TextMark) => string;
  onMark: (mark: TextMark, el: HTMLElement) => void;
  /** Hover and focus on a mark (its tooltip); `undefined` when they leave it. */
  onHint?: (mark: TextMark | undefined, el: HTMLElement) => void;
}

/** The original or the polished text as leaves, the marks as buttons around theirs. */
export function MarkedText({ text, marks, highlight, label, onMark, onHint }: MarkedTextProps) {
  const leaves = leavesOf(text, marks, highlight);
  const out: ReactNode[] = [];
  const leafNode = (leaf: Leaf) => (
    <span
      key={leaf.start}
      {...{ [LEAF_ATTR]: leaf.start }}
      data-fix-highlight={leaf.lit ? "" : undefined}
      className={leaf.lit ? HIGHLIGHT_CLASS : undefined}>
      {text.slice(leaf.start, leaf.end)}
    </span>
  );
  for (let i = 0; i < leaves.length; i += 1) {
    const leaf = leaves[i];
    if (leaf === undefined) continue;
    const mark = leaf.mark;
    if (mark === undefined) {
      out.push(leafNode(leaf));
      continue;
    }
    const inside: Leaf[] = [];
    while (leaves[i]?.mark === mark) {
      const next = leaves[i];
      if (next !== undefined) inside.push(next);
      i += 1;
    }
    i -= 1;
    const press = (el: HTMLElement) => {
      onMark(mark, el);
    };
    out.push(
      <span
        key={`mark-${mark.start}`}
        role="button"
        tabIndex={0}
        aria-label={label(mark)}
        data-testid="fix-mark"
        data-mark={mark.kind}
        className={cx(MARK_CLASS, MARK_KIND_CLASS[mark.kind])}
        onClick={(e) => {
          if (!draggedOver()) press(e.currentTarget);
        }}
        onKeyDown={(e) => {
          pressKeys(() => {
            press(e.currentTarget);
          })(e);
        }}
        onMouseEnter={(e) => onHint?.(mark, e.currentTarget)}
        onMouseLeave={(e) => onHint?.(undefined, e.currentTarget)}
        onFocus={(e) => onHint?.(mark, e.currentTarget)}
        onBlur={(e) => onHint?.(undefined, e.currentTarget)}>
        {inside.map(leafNode)}
      </span>,
    );
  }
  return <>{out}</>;
}

export interface DiffTextProps {
  items: readonly DiffItem[];
  /** The replacement whose card is open (a ring around it). */
  active?: PolishPair | undefined;
  label: (pair: PolishPair) => string;
  onPair: (pair: PolishPair, el: HTMLElement) => void;
}

const DEL_CLASS = "mx-0.5 rounded-4 bg-diff-del px-1 text-danger line-through";
const ADD_CLASS = "mx-0.5 rounded-4 bg-diff-add px-1 text-ok-text";

/** The diff view: deleted and added runs as before, each replacement one button (§16.9 3.5);
 *  pure deletions and additions are not buttons. */
export function DiffText({ items, active, label, onPair }: DiffTextProps) {
  return (
    <>
      {items.map((item, i) => {
        if (item.kind === "same") return <span key={i}>{item.text}</span>;
        if (item.kind === "del")
          return (
            <span key={i} className={DEL_CLASS}>
              {item.text}
            </span>
          );
        if (item.kind === "add")
          return (
            <span key={i} className={ADD_CLASS}>
              {item.text}
            </span>
          );
        const pair = item.pair;
        return (
          <span
            key={i}
            role="button"
            tabIndex={0}
            aria-label={label(pair)}
            data-testid="fix-pair"
            className={cx(
              "cursor-pointer rounded-[6px] box-decoration-clone hover:outline hover:outline-[1.5px] hover:outline-accent focus-visible:outline focus-visible:outline-2 focus-visible:outline-primary",
              active?.start === pair.start && "outline outline-2 outline-primary",
            )}
            onClick={(e) => {
              onPair(pair, e.currentTarget);
            }}
            onKeyDown={(e) => {
              pressKeys(() => {
                onPair(pair, e.currentTarget);
              })(e);
            }}>
            <span className={DEL_CLASS}>{item.del}</span>
            <span className={ADD_CLASS}>{item.add}</span>
          </span>
        );
      })}
    </>
  );
}
