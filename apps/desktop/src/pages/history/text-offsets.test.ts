// Between DOM selections and offsets into the entry's text (docs/dictation.md §16.9): text-node
// and element boundaries, boundaries before and after the text, and the selection back.
import { boundaryOffset, rangeRect, selectOffsets, selectionIn } from "./text-offsets";

/** `<p>before</p><div><span data-at=0>下午用</span><b><span data-at=3>维他命</span></b><span data-at=6>测了</span></div><p>after</p>` */
function fixture() {
  document.body.innerHTML =
    '<p id="before">before</p><div id="root"><span data-at="0">下午用</span><b><span data-at="3">维他命</span></b><span data-at="6">测了</span></div><p id="after">after</p>';
  const root = document.getElementById("root");
  if (root === null) throw new Error("no root");
  const leaf = (at: number) => {
    const el = root.querySelector(`[data-at="${at}"]`);
    if (el?.firstChild == null) throw new Error(`no leaf ${at}`);
    return el.firstChild;
  };
  return { root, leaf };
}

afterEach(() => {
  window.getSelection()?.removeAllRanges();
  document.body.innerHTML = "";
});

describe("boundaryOffset", () => {
  it("a text node: its leaf's start plus the offset, at most the text's length", () => {
    const { root, leaf } = fixture();
    expect(boundaryOffset(root, leaf(3), 1, 8)).toBe(4);
    expect(boundaryOffset(root, leaf(6), 2, 8)).toBe(8);
    expect(boundaryOffset(root, leaf(6), 2, 7)).toBe(7);
  });

  it("an element: the first leaf from the child at the offset, or the end of the element", () => {
    const { root } = fixture();
    expect(boundaryOffset(root, root, 0, 8)).toBe(0);
    expect(boundaryOffset(root, root, 1, 8)).toBe(3);
    expect(boundaryOffset(root, root, 3, 8)).toBe(8);
    const bold = root.querySelector("b");
    if (bold === null) throw new Error("no b");
    expect(boundaryOffset(root, bold, 0, 8)).toBe(3);
    expect(boundaryOffset(root, bold, 1, 8)).toBe(6);
    const leafEl = root.querySelector('[data-at="6"]');
    if (leafEl === null) throw new Error("no leaf");
    expect(boundaryOffset(root, leafEl, 1, 8)).toBe(8);
    const empty = document.createElement("i");
    root.append(empty);
    expect(boundaryOffset(root, empty, 0, 8)).toBe(0);
    // A text node outside any leaf counts from the start.
    const stray = document.createTextNode("x");
    root.append(stray);
    expect(boundaryOffset(root, stray, 0, 8)).toBe(0);
  });

  it("outside the text: its start before it, its end after it", () => {
    const { root } = fixture();
    const before = document.getElementById("before");
    const after = document.getElementById("after");
    if (before === null || after === null) throw new Error("no siblings");
    expect(boundaryOffset(root, before, 0, 8)).toBe(0);
    expect(boundaryOffset(root, after, 0, 8)).toBe(8);
  });
});

describe("selectionIn / selectOffsets", () => {
  it("what is selected in the text, with its box", () => {
    const { root, leaf } = fixture();
    const range = document.createRange();
    range.setStart(leaf(0), 2);
    range.setEnd(leaf(3), 3);
    window.getSelection()?.addRange(range);
    expect(selectionIn(root, 8)).toMatchObject({ start: 2, end: 6 });
    // The box: the range's where the page is laid out, the text's otherwise.
    const rect = rangeRect(range, root);
    expect(rect).toBeDefined();
  });

  it("nothing: no selection, a bare stand-in, a collapsed one, one wholly outside", () => {
    const { root, leaf } = fixture();
    expect(selectionIn(root, 8, null)).toBeUndefined();
    expect(selectionIn(root, 8, { toString: () => "x" } as unknown as Selection)).toBeUndefined();
    expect(selectionIn(root, 8)).toBeUndefined();
    const collapsed = document.createRange();
    collapsed.setStart(leaf(0), 1);
    collapsed.setEnd(leaf(0), 1);
    window.getSelection()?.addRange(collapsed);
    expect(selectionIn(root, 8)).toBeUndefined();
    window.getSelection()?.removeAllRanges();
    const outside = document.createRange();
    const before = document.getElementById("before")?.firstChild;
    if (before == null) throw new Error("no text");
    outside.setStart(before, 0);
    outside.setEnd(before, 3);
    window.getSelection()?.addRange(outside);
    expect(selectionIn(root, 8)).toBeUndefined();
  });

  it("selects offsets back, across leaves, and refuses ends that are not in the text", () => {
    const { root } = fixture();
    expect(selectOffsets(root, 1, 5)?.toString()).toBe("午用维他");
    expect(window.getSelection()?.toString()).toBe("午用维他");
    // An end at a leaf boundary lands in the next leaf.
    expect(selectOffsets(root, 3, 6)?.toString()).toBe("维他命");
    expect(selectOffsets(root, 0, 99)).toBeUndefined();
  });

  it("the range's own box where the page is laid out", () => {
    const { root } = fixture();
    const range = document.createRange();
    const box = { top: 1, bottom: 2, left: 3, right: 4 } as DOMRect;
    Object.defineProperty(range, "getBoundingClientRect", { value: () => box });
    expect(rangeRect(range, root)).toBe(box);
  });
});
