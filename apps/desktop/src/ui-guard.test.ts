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
  /** How Tauri's IPC answers the check's report; `"absent"` leaves Tauri's internals out. */
  invoke?: "resolves" | "rejects" | "throws" | "absent";
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
  const invoke = vi.fn<(cmd: string, args: unknown) => Promise<unknown>>(() => {
    if (page.invoke === "throws") throw new Error("no IPC");
    return page.invoke === "rejects" ? Promise.reject(new Error("refused")) : Promise.resolve();
  });
  const fakeWindow: {
    voltipUiBuild?: string;
    voltipUiCheck?: unknown;
    __TAURI_INTERNALS__?: unknown;
  } = {
    voltipUiBuild: page.build === null ? undefined : (page.build ?? BUILD),
    __TAURI_INTERNALS__:
      page.invoke === "absent"
        ? undefined
        : { metadata: { currentWebview: { label: "main" } }, invoke },
  };
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
    window: fakeWindow,
    /** What the check reported to the program, in order. */
    reports: () =>
      invoke.mock.calls.map(([cmd, args]) => {
        expect(cmd).toBe("plugin:event|emit");
        expect(args).toMatchObject({ event: "voltip://ui-guard" });
        return (args as { payload: unknown }).payload;
      }),
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
      // It says so, on the window and to the program's log.
      expect(page.window.voltipUiCheck).toEqual({ verdict: "ok", replaced: false });
      expect(page.reports()).toEqual([
        {
          window: "main",
          verdict: "ok",
          replaced: false,
          scripts: ["/assets/index-NEW00001.js"],
          build: BUILD,
        },
      ]);
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
      "voltip: this window runs interface code the program does not carry; replacing its document once",
      ["/assets/index-OLD00001.js"],
      undefined,
    );
    expect(stale.reports()).toEqual([
      {
        window: "main",
        verdict: "replacing",
        replaced: false,
        scripts: ["/assets/index-OLD00001.js"],
        build: null,
      },
    ]);

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
    expect(fresh.window.voltipUiCheck).toEqual({ verdict: "ok", replaced: true });
    expect(fresh.reports()).toMatchObject([{ verdict: "ok", replaced: true }]);
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

  it("a replaced window that still runs other code is kept as it is, with the reason", () => {
    // Coordinator's goal text, 2026-11-02: after one replacement no cache can answer, a mismatch more
    // likely means the check misjudges this webview; a blank or reloading window would make every
    // start unusable. The window stays, the console and the program's log say why.
    const again = open({
      scripts: ["/assets/index-OLD00001.js"],
      href: `${APP}?voltip-ui-reload=1762000000000`,
    });
    expect(again.location.replace).not.toHaveBeenCalled();
    // Only the check's own marker leaves the address, as in every replaced document.
    expect(again.history.replaceState).toHaveBeenCalledTimes(1);
    expect(again.location.href).toBe(APP);
    expect(again.error).toHaveBeenCalledWith(
      "voltip: the window still runs interface code the program does not carry after its document was replaced once; it is kept as it is",
      ["/assets/index-OLD00001.js"],
      BUILD,
    );
    expect(again.window.voltipUiCheck).toEqual({ verdict: "kept", replaced: true });
    expect(again.reports()).toEqual([
      {
        window: "main",
        verdict: "kept",
        replaced: true,
        scripts: ["/assets/index-OLD00001.js"],
        build: BUILD,
      },
    ]);
    // The same for another build at the program's path.
    const build = open({
      scripts: ["/assets/index-NEW00001.js"],
      build: OTHER_BUILD,
      href: `${APP}?voltip-ui-reload=1`,
    });
    expect(build.location.replace).not.toHaveBeenCalled();
    expect(build.window.voltipUiCheck).toEqual({ verdict: "kept", replaced: true });
  });

  it("checks and replaces whether or not the program hears of it", () => {
    for (const invoke of ["absent", "throws", "rejects"] as const) {
      const ok = open({ scripts: ["/assets/index-NEW00001.js"], invoke });
      expect(ok.window.voltipUiCheck).toEqual({ verdict: "ok", replaced: false });
      const stale = open({ scripts: ["/assets/index-OLD00001.js"], invoke });
      expect(stale.location.replace).toHaveBeenCalledTimes(1);
    }
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
