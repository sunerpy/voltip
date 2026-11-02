// The check every window runs as its document is created (src-tauri/src/ui_guard.js,
// docs/dictation.md §33.4), run here the way `ui_guard.rs` hands it over: the carried scripts and
// builds in place of the placeholders, against a document, a location and a history of the test's.
import source from "../src-tauri/src/ui_guard.js?raw";

const CARRIED = ["/assets/index-NEW00001.js", "/assets/a b.js"];
const BUILD = "voltip-ui-build-0123456789abcdef0123456789abcdef";
const OTHER_BUILD = "voltip-ui-build-ffffffffffffffffffffffffffffffff";
const APP = "http://tauri.localhost/index.html";
const PILL = "http://tauri.localhost/index.html#/overlay?state=listening";

interface Page {
  /** The `src` of each module script in the document. */
  scripts: string[];
  href?: string;
  /** What the bundle declared on the window; `null` declares nothing. */
  build?: string | null;
  /** The builds the program carries. */
  builds?: string[];
  /** The document is still being parsed when the check runs. */
  loading?: boolean;
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
  const url = new URL(page.href ?? APP);
  const location = {
    href: url.href,
    protocol: url.protocol,
    hostname: url.hostname,
    replace: vi.fn<(to: string) => void>(),
  };
  const history = {
    state: { from: "the router" },
    replaceState: vi.fn((_state: unknown, _title: string, to: string) => {
      location.href = new URL(to, location.href).href;
    }),
  };
  const fakeWindow = { voltipUiBuild: page.build === null ? undefined : (page.build ?? BUILD) };
  const warn = vi.fn();
  const error = vi.fn();
  const check = source
    .replace("__VOLTIP_UI_SCRIPTS__", JSON.stringify(CARRIED))
    .replace("__VOLTIP_UI_BUILDS__", JSON.stringify(page.builds ?? [BUILD]));
  // oxlint-disable-next-line typescript/no-implied-eval -- the check is a script the webview evaluates
  new Function("window", "document", "location", "history", "console", check)(
    fakeWindow,
    fakeDocument,
    location,
    history,
    { warn, error },
  );
  return {
    location,
    history,
    warn,
    error,
    parsed: () => {
      for (const listener of ready) listener();
    },
  };
}

/** Where a reload goes: the same address with the check's marker. */
function reloadedFrom(page: ReturnType<typeof open>) {
  expect(page.location.replace).toHaveBeenCalledTimes(1);
  return new URL(page.location.replace.mock.calls[0]?.[0] ?? "");
}

describe("ui_guard.js", () => {
  it("leaves a window that runs the interface the program carries", () => {
    for (const href of [APP, PILL]) {
      const page = open({ scripts: ["/assets/index-NEW00001.js"], href });
      expect(page.location.replace).not.toHaveBeenCalled();
      expect(page.history.replaceState).not.toHaveBeenCalled();
      expect(page.warn).not.toHaveBeenCalled();
    }
  });

  it("regression: a window running interface code the program does not carry is replaced once by the program's", () => {
    // User report 2026-11-02: after an update the dictionary page lacked its 学习建议 tab until
    // Voltip was restarted. Until then nothing compared a window's interface with the program's.
    vi.spyOn(Date, "now").mockReturnValue(1_762_000_000_000);
    const stale = open({ scripts: ["/assets/index-OLD00001.js"], build: null, href: PILL });
    const to = reloadedFrom(stale);
    // An address this webview never asked for, so no cache answers it; the page's own route stays.
    expect(to.origin + to.pathname).toBe("http://tauri.localhost/index.html");
    expect(to.searchParams.get("voltip-ui-reload")).toBe("1762000000000");
    expect(to.hash).toBe("#/overlay?state=listening");
    expect(stale.warn).toHaveBeenCalledWith(
      "voltip: this window runs interface code the program does not carry; reloading",
      ["/assets/index-OLD00001.js"],
      undefined,
    );

    // The program's document at that address: the marker leaves the URL before the interface
    // reads it, the router's state and the route kept, and the window keeps the program's code.
    const fresh = open({ scripts: ["/assets/index-NEW00001.js"], href: to.href });
    expect(fresh.history.replaceState).toHaveBeenCalledWith(
      { from: "the router" },
      "",
      "/index.html#/overlay?state=listening",
    );
    expect(fresh.location.href).toBe(PILL);
    expect(fresh.location.replace).not.toHaveBeenCalled();
  });

  it("regression: a window whose bundle is another build at the same path is replaced too", () => {
    // The same script path, other code: the build the bundle declares tells them apart.
    const other = open({ scripts: ["/assets/index-NEW00001.js"], build: OTHER_BUILD });
    expect(reloadedFrom(other).searchParams.has("voltip-ui-reload")).toBe(true);
    // A bundle that declares no build where the program's does is another build as well.
    expect(
      open({ scripts: ["/assets/index-NEW00001.js"], build: null }).location.replace,
    ).toHaveBeenCalledTimes(1);
    // A program whose bundle declares no build checks the scripts alone.
    const unnamed = open({ scripts: ["/assets/index-NEW00001.js"], build: null, builds: [] });
    expect(unnamed.location.replace).not.toHaveBeenCalled();
  });

  it("regression: a reloaded window that still runs other code is cleared, not left with it", () => {
    const again = open({
      scripts: ["/assets/index-OLD00001.js"],
      href: `${APP}?voltip-ui-reload=1762000000000`,
    });
    expect(again.location.replace).toHaveBeenCalledWith("about:blank");
    expect(again.location.replace).toHaveBeenCalledTimes(1);
    expect(again.error).toHaveBeenCalledWith(
      "voltip: the reloaded window still runs interface code the program does not carry; clearing it",
      ["/assets/index-OLD00001.js"],
      BUILD,
    );
    // The same for another build at the program's path.
    const build = open({
      scripts: ["/assets/index-NEW00001.js"],
      build: OTHER_BUILD,
      href: `${APP}?voltip-ui-reload=1`,
    });
    expect(build.location.replace).toHaveBeenCalledWith("about:blank");
  });

  it("keeps the page's own query when it adds and removes the marker", () => {
    const stale = open({
      scripts: ["/assets/index-OLD00001.js"],
      href: `${APP}?usage=near#/history`,
    });
    const to = reloadedFrom(stale);
    expect(to.searchParams.get("usage")).toBe("near");
    expect(to.hash).toBe("#/history");
    const fresh = open({ scripts: ["/assets/index-NEW00001.js"], href: to.href });
    expect(fresh.location.href).toBe(`${APP}?usage=near#/history`);
  });

  it("reloads a document that carries one script of the program's and one it does not", () => {
    const page = open({ scripts: ["/assets/index-NEW00001.js", "/assets/chunk-OLD00001.js"] });
    expect(page.location.replace).toHaveBeenCalledTimes(1);
  });

  it("checks a document still being parsed once it has been", () => {
    const page = open({ scripts: ["/assets/index-OLD00001.js"], loading: true });
    expect(page.location.replace).not.toHaveBeenCalled();
    page.parsed();
    expect(page.location.replace).toHaveBeenCalledTimes(1);
  });

  it("reads script paths the way the document's URL resolves them", () => {
    // Relative to the page, on either scheme the program serves its pages from.
    expect(
      open({ scripts: ["./assets/index-NEW00001.js"] }).location.replace,
    ).not.toHaveBeenCalled();
    expect(
      open({ scripts: ["assets/index-NEW00001.js"], href: "tauri://localhost/index.html" }).location
        .replace,
    ).not.toHaveBeenCalled();
    expect(
      open({ scripts: ["/assets/index-OLD00001.js"], href: "tauri://localhost/" }).location.replace,
    ).toHaveBeenCalledTimes(1);
    // Percent-encoded, and a path that does not decode.
    expect(open({ scripts: ["/assets/a%20b.js"] }).location.replace).not.toHaveBeenCalled();
    expect(open({ scripts: ["/assets/%E0%A4%A.js"] }).location.replace).toHaveBeenCalledTimes(1);
  });

  it("does nothing in a page that is not the program's, or not the interface", () => {
    const elsewhere = open({
      scripts: ["/assets/index-OLD00001.js"],
      href: "https://example.test/index.html?voltip-ui-reload=1",
    });
    expect(elsewhere.location.replace).not.toHaveBeenCalled();
    expect(elsewhere.history.replaceState).not.toHaveBeenCalled();
    expect(elsewhere.warn).not.toHaveBeenCalled();
    const blank = open({ scripts: [], build: null });
    expect(blank.location.replace).not.toHaveBeenCalled();
    expect(blank.warn).not.toHaveBeenCalled();
  });
});
