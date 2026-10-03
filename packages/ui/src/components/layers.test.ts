import { dismissTopLayer, openLayer } from "./layers";

describe("layers", () => {
  it("closes only the layer on top, and says when none is open", () => {
    const closeDialog = vi.fn();
    const closeList = vi.fn();
    expect(dismissTopLayer()).toBe(false);
    const dialog = openLayer(closeDialog);
    const list = openLayer(closeList);
    expect([dialog.isTop(), list.isTop()]).toEqual([false, true]);
    expect(dismissTopLayer()).toBe(true);
    expect([closeDialog.mock.calls.length, closeList.mock.calls.length]).toEqual([0, 1]);
    // Closing is the owner's to report; until then the layer stays on top.
    expect(list.isTop()).toBe(true);
    list.remove();
    expect(dialog.isTop()).toBe(true);
    // Removing twice is harmless and leaves the others alone.
    list.remove();
    expect(dialog.isTop()).toBe(true);
    expect(dismissTopLayer()).toBe(true);
    expect(closeDialog).toHaveBeenCalledTimes(1);
    dialog.remove();
    expect(dismissTopLayer()).toBe(false);
  });

  it("lets a layer under another one leave without disturbing the one on top", () => {
    const under = openLayer(vi.fn());
    const top = openLayer(vi.fn());
    under.remove();
    expect(top.isTop()).toBe(true);
    top.remove();
    expect(dismissTopLayer()).toBe(false);
  });
});
