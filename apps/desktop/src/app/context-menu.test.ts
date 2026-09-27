import { allowsContextMenu, installContextMenuPolicy } from "./context-menu";

function rightClick(target: EventTarget): boolean {
  const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2 });
  target.dispatchEvent(event);
  return event.defaultPrevented;
}

describe("context menu policy", () => {
  let root: HTMLDivElement;
  beforeEach(() => {
    root = document.createElement("div");
    root.innerHTML = `
      <button id="btn" type="button">开始听写</button>
      <p id="text">把 fetchUser 改成 async</p>
      <input id="input" />
      <textarea id="area"></textarea>
      <div id="rich" contenteditable="true"><b id="rich-child">词条</b></div>
      <div id="static" contenteditable="false">只读</div>
      <pre id="allowed" data-allow-context-menu><code id="allowed-child">voltip-cli</code></pre>
    `;
    document.body.append(root);
  });
  afterEach(() => {
    root.remove();
  });

  it("regression: the Tauri client shows no browser context menu except on editable fields", () => {
    const uninstall = installContextMenuPolicy(document, { enabled: true });
    const byId = (id: string) => document.getElementById(id) as HTMLElement;
    // Chrome: cancelled.
    expect(rightClick(byId("btn"))).toBe(true);
    expect(rightClick(byId("text"))).toBe(true);
    expect(rightClick(byId("static"))).toBe(true);
    expect(rightClick(document.body)).toBe(true);
    // Editable targets and opted-in regions keep the native menu, children included.
    expect(rightClick(byId("input"))).toBe(false);
    expect(rightClick(byId("area"))).toBe(false);
    expect(rightClick(byId("rich"))).toBe(false);
    expect(rightClick(byId("rich-child"))).toBe(false);
    expect(rightClick(byId("allowed"))).toBe(false);
    expect(rightClick(byId("allowed-child"))).toBe(false);
    // Uninstalling restores the default everywhere.
    uninstall();
    expect(rightClick(byId("btn"))).toBe(false);
  });

  it("regression: the browser dev preview (enabled: false) keeps its context menu", () => {
    const uninstall = installContextMenuPolicy(document, { enabled: false });
    expect(rightClick(document.getElementById("btn") as HTMLElement)).toBe(false);
    uninstall();
    expect(rightClick(document.getElementById("btn") as HTMLElement)).toBe(false);
  });

  it("allowsContextMenu resolves text nodes through their parent and refuses non-nodes", () => {
    const input = document.getElementById("input") as HTMLElement;
    const text = document.getElementById("text") as HTMLElement;
    expect(allowsContextMenu(input)).toBe(true);
    expect(allowsContextMenu(text)).toBe(false);
    expect(allowsContextMenu(text.firstChild)).toBe(false);
    const richText = (document.getElementById("rich-child") as HTMLElement).firstChild;
    expect(allowsContextMenu(richText)).toBe(true);
    expect(allowsContextMenu(null)).toBe(false);
    expect(allowsContextMenu(window)).toBe(false);
  });
});
