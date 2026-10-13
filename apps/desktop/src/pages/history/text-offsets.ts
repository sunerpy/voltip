// Between a DOM selection in the history detail's text and offsets into the entry's text
// (docs/dictation.md §16.9). The text renders as leaves — elements with `data-at` (where they start
// in the text, UTF-16 code units) around exactly one text node — so a boundary is its leaf's start
// plus the offset inside it; boundaries before or after the text clamp to its ends.
import type { TextRange } from "@voltip/shared";

/** The attribute of a leaf: where its text starts in the entry's text. */
export const LEAF_ATTR = "data-at";

function leafStart(el: Element): number {
  return Number(el.getAttribute(LEAF_ATTR) ?? "0");
}

/** The leaf an element is or holds first; `undefined` for an element without one. */
function firstLeaf(node: Node): Element | undefined {
  if (!(node instanceof Element)) return node.parentElement?.closest(`[${LEAF_ATTR}]`) ?? undefined;
  return node.hasAttribute(LEAF_ATTR) ? node : (node.querySelector(`[${LEAF_ATTR}]`) ?? undefined);
}

/** The offset in the text of a selection boundary `(node, offset)` in `root`. */
export function boundaryOffset(root: Element, node: Node, offset: number, length: number): number {
  if (!root.contains(node)) {
    // Before the text, or after it (the raw-transcript line below it, the page further on).
    return root.compareDocumentPosition(node) & Node.DOCUMENT_POSITION_PRECEDING ? 0 : length;
  }
  if (node.nodeType === Node.TEXT_NODE) {
    const leaf = node.parentElement?.closest(`[${LEAF_ATTR}]`);
    return leaf != null && root.contains(leaf) ? Math.min(leafStart(leaf) + offset, length) : 0;
  }
  const children = node.childNodes;
  for (let i = offset; i < children.length; i += 1) {
    const child = children[i];
    const leaf = child === undefined ? undefined : firstLeaf(child);
    if (leaf !== undefined) return leafStart(leaf);
  }
  // At the end of `node`: after its last leaf.
  const leaves = node instanceof Element ? node.querySelectorAll(`[${LEAF_ATTR}]`) : [];
  const last = leaves[leaves.length - 1];
  if (last === undefined)
    return node instanceof Element && node.hasAttribute(LEAF_ATTR)
      ? leafStart(node) + (node.textContent ?? "").length
      : 0;
  return Math.min(leafStart(last) + (last.textContent ?? "").length, length);
}

/** What is selected in `root`, as offsets into its text of `length`, with the selection's box;
 *  `undefined` when nothing is, or the selection lies wholly outside it. */
export function selectionIn(
  root: Element,
  length: number,
  selection: Selection | null = window.getSelection(),
): (TextRange & { rect: DOMRect }) | undefined {
  // A test may stand a bare object in for the selection: only a real one has ranges.
  if (
    selection === null ||
    typeof selection.getRangeAt !== "function" ||
    selection.rangeCount === 0
  ) {
    return undefined;
  }
  if (selection.isCollapsed) return undefined;
  const range = selection.getRangeAt(0);
  if (!range.intersectsNode(root)) return undefined;
  const start = boundaryOffset(root, range.startContainer, range.startOffset, length);
  const end = boundaryOffset(root, range.endContainer, range.endOffset, length);
  if (end <= start) return undefined;
  return { start, end, rect: rangeRect(range, root) };
}

/** The box of `range` (where the webview lays it out; jsdom does not, the text's box stands in). */
export function rangeRect(range: Range, fallback: Element): DOMRect {
  return typeof range.getBoundingClientRect === "function"
    ? range.getBoundingClientRect()
    : fallback.getBoundingClientRect();
}

/** The DOM position of text offset `at` in `root`: inside the leaf that holds it. */
function positionOf(root: Element, at: number): { node: Node; offset: number } | undefined {
  let found: { node: Node; offset: number } | undefined;
  for (const leaf of root.querySelectorAll(`[${LEAF_ATTR}]`)) {
    const start = leafStart(leaf);
    const text = leaf.firstChild;
    const size = (leaf.textContent ?? "").length;
    if (text !== null && at >= start && at <= start + size) {
      found = { node: text, offset: at - start };
      if (at < start + size) break;
    }
  }
  return found;
}

/** Select `[start, end)` of the text in `root` (the double click's word); the range, or `undefined`
 *  when an end is not in it. */
export function selectOffsets(root: Element, start: number, end: number): Range | undefined {
  const from = positionOf(root, start);
  const to = positionOf(root, end);
  const selection = window.getSelection();
  if (from === undefined || to === undefined || selection === null) return undefined;
  const range = document.createRange();
  range.setStart(from.node, from.offset);
  range.setEnd(to.node, to.offset);
  selection.removeAllRanges();
  selection.addRange(range);
  return range;
}
