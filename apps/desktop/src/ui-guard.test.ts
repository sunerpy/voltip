// The check every window runs as its document is created (src-tauri/src/ui_guard.js,
// docs/dictation.md §33.4), run here the way `ui_guard.rs` hands it over: the carried scripts in
// place of the placeholder, against a document, a location and a session storage of the test's.
import source from "../src-tauri/src/ui_guard.js?raw";

const CARRIED = ["/assets/index-NEW00001.js", "/assets/a b.js"];
const KEY = "voltip.ui-guard.reloaded";

class MemoryStorage {
  readonly items = new Map<string, string>();
  getItem(key: string) {
    return this.items.get(key) ?? null;
  }
  setItem(key: string, value: string) {
    this.items.set(key, value);
  }
  removeItem(key: string) {
    this.items.delete(key);
  }
}

interface Page {
  /** The `src` of each module script in the document. */
  scripts: string[];
  href?: string;
  /** The document is still being parsed when the check runs. */
  loading?: boolean;
  /** The window's session storage; `"denied"` when reading it throws. */
  storage?: MemoryStorage | "denied";
}

function open(page: Page) {
  const doc = document.implementation.createHTMLDocument("Voltip");
  for (const src of page.scripts) {
    const script = doc.createElement("script");
    script.type = "module";
    script.setAttribute("src", src);
    doc.head.append(script);
  }
  // A classic script is not an entry of the interface.
  doc.head.append(Object.assign(doc.createElement("script"), { src: "/classic.js" }));
  const ready: (() => void)[] = [];
  const fakeDocument = {
    readyState: page.loading ? "loading" : "complete",
    querySelectorAll: (selector: string) => doc.querySelectorAll(selector),
    addEventListener: (type: string, listener: () => void) => {
      if (type === "DOMContentLoaded") ready.push(listener);
    },
  };
  const url = new URL(page.href ?? "http://tauri.localhost/index.html#/overlay?state=listening");
  const reload = vi.fn();
  const location = { href: url.href, protocol: url.protocol, hostname: url.hostname, reload };
  const storage = page.storage ?? new MemoryStorage();
  const fakeWindow = {
    get sessionStorage() {
      if (storage === "denied") throw new DOMException("denied", "SecurityError");
      return storage;
    },
  };
  const warn = vi.fn();
  const check = source.replace("__VOLTIP_UI_SCRIPTS__", JSON.stringify(CARRIED));
  // oxlint-disable-next-line typescript/no-implied-eval -- the check is a script the webview evaluates
  new Function("window", "document", "location", "console", check)(
    fakeWindow,
    fakeDocument,
    location,
    { warn },
  );
  return {
    reload,
    warn,
    parsed: () => {
      for (const listener of ready) listener();
    },
  };
}

describe("ui_guard.js", () => {
  it("leaves a window that runs the interface the program carries", () => {
    const storage = new MemoryStorage();
    storage.setItem(KEY, CARRIED.join(" "));
    const page = open({ scripts: ["/assets/index-NEW00001.js"], storage });
    expect(page.reload).not.toHaveBeenCalled();
    expect(page.warn).not.toHaveBeenCalled();
    // A later stale document in this window may reload again.
    expect(storage.getItem(KEY)).toBeNull();
  });

  it("regression: a window running interface code the program does not carry reloads, once", () => {
    // User report 2026-11-02: after an update the dictionary page lacked its 学习建议 tab until
    // Voltip was restarted. Until then nothing compared a window's interface with the program's.
    const storage = new MemoryStorage();
    const stale = open({ scripts: ["/assets/index-OLD00001.js"], storage });
    expect(stale.reload).toHaveBeenCalledTimes(1);
    expect(storage.getItem(KEY)).toBe(CARRIED.join(" "));
    expect(stale.warn).toHaveBeenCalledWith(
      "voltip: this window runs interface code the program does not carry; reloading",
      ["/assets/index-OLD00001.js"],
    );

    // The reload brought the same document again: it stays, and says so, instead of looping.
    const again = open({ scripts: ["/assets/index-OLD00001.js"], storage });
    expect(again.reload).not.toHaveBeenCalled();
    expect(again.warn).toHaveBeenCalledWith(
      "voltip: this window runs interface code the program does not carry",
      ["/assets/index-OLD00001.js"],
    );

    // The reload brought the program's document: the window runs it, and the once starts over.
    const fixed = open({ scripts: ["/assets/index-NEW00001.js"], storage });
    expect(fixed.reload).not.toHaveBeenCalled();
    expect(storage.getItem(KEY)).toBeNull();
  });

  it("reloads a document that carries one script of the program's and one it does not", () => {
    const page = open({ scripts: ["/assets/index-NEW00001.js", "/assets/chunk-OLD00001.js"] });
    expect(page.reload).toHaveBeenCalledTimes(1);
  });

  it("checks a document still being parsed once it has been", () => {
    const page = open({ scripts: ["/assets/index-OLD00001.js"], loading: true });
    expect(page.reload).not.toHaveBeenCalled();
    page.parsed();
    expect(page.reload).toHaveBeenCalledTimes(1);
  });

  it("reads script paths the way the document's URL resolves them", () => {
    // Relative to the page, on either scheme the program serves its pages from.
    expect(open({ scripts: ["./assets/index-NEW00001.js"] }).reload).not.toHaveBeenCalled();
    expect(
      open({ scripts: ["assets/index-NEW00001.js"], href: "tauri://localhost/index.html" }).reload,
    ).not.toHaveBeenCalled();
    expect(
      open({ scripts: ["/assets/index-OLD00001.js"], href: "tauri://localhost/" }).reload,
    ).toHaveBeenCalledTimes(1);
    // Percent-encoded, and a path that does not decode.
    expect(open({ scripts: ["/assets/a%20b.js"] }).reload).not.toHaveBeenCalled();
    expect(open({ scripts: ["/assets/%E0%A4%A.js"] }).reload).toHaveBeenCalledTimes(1);
  });

  it("does nothing in a page that is not the program's, or not the interface", () => {
    const elsewhere = open({
      scripts: ["/assets/index-OLD00001.js"],
      href: "https://example.test/index.html",
    });
    expect(elsewhere.reload).not.toHaveBeenCalled();
    expect(elsewhere.warn).not.toHaveBeenCalled();
    const blank = open({ scripts: [] });
    expect(blank.reload).not.toHaveBeenCalled();
    expect(blank.warn).not.toHaveBeenCalled();
  });

  it("does not reload without a session storage to keep the once in", () => {
    const page = open({ scripts: ["/assets/index-OLD00001.js"], storage: "denied" });
    expect(page.reload).not.toHaveBeenCalled();
    expect(page.warn).toHaveBeenCalledWith(
      "voltip: this window runs interface code the program does not carry",
      ["/assets/index-OLD00001.js"],
    );
  });
});
