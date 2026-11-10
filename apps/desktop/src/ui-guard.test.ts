// The check every window runs as its document is created (src-tauri/src/ui_guard.js,
// docs/dictation.md §33.4), run here the way `ui_guard.rs` hands it over: the carried scripts and
// builds in place of the placeholders, against a document, a location and a history of the test's.
import source from "../src-tauri/src/ui_guard.js?raw";

const CARRIED = ["/assets/index-NEW00001.js", "/assets/a b.js"];
const BUILD = "voltip-ui-build-0123456789abcdef0123456789abcdef";
const OTHER_BUILD = "voltip-ui-build-ffffffffffffffffffffffffffffffff";
const APP = "http://tauri.localhost/index.html";
const PILL = "http://tauri.localhost/index.html#/overlay?state=listening";

/** How Tauri's IPC answers one of the check's reports: the program has it, refused it, the call
 *  threw at once, it is never answered, or the program has it only `SLOW_MS` later. */
type Answer = "resolves" | "rejects" | "throws" | "hangs" | "slow";

/** How long a `"slow"` answer takes: past the time the check waits for one. */
const SLOW_MS = 6_000;

interface Page {
  /** The `src` of each module script in the document. */
  scripts: string[];
  href?: string;
  /** The window's label in Tauri's metadata. */
  label?: string;
  /** What the bundle declared on the window; `null` declares nothing. */
  build?: string | null;
  /** The builds the program carries. */
  builds?: string[];
  /** The document is still being parsed when the check runs. */
  loading?: boolean;
  /** How Tauri's IPC answers each of the check's reports, in order, the last one for every later
   *  report; `"absent"` leaves Tauri's internals out. */
  invoke?: Answer | Answer[] | "absent";
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
  const answers: Answer[] =
    page.invoke === undefined || page.invoke === "absent" ? ["resolves"] : [page.invoke].flat();
  /** The reports the program has, in the order it got them. */
  const heard: unknown[] = [];
  let calls = 0;
  const invoke = vi.fn<(cmd: string, args: unknown) => Promise<unknown>>((_cmd, args) => {
    const answer = answers[Math.min(calls, answers.length - 1)];
    calls += 1;
    if (answer === "throws") throw new Error("no IPC");
    if (answer === "rejects") return Promise.reject(new Error("refused"));
    if (answer === "hangs") return new Promise(() => {});
    const payload = (args as { payload: unknown }).payload;
    if (answer === "slow")
      return new Promise((resolve) => {
        setTimeout(() => {
          heard.push(payload);
          resolve(undefined);
        }, SLOW_MS);
      });
    heard.push(payload);
    return Promise.resolve();
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
        : { metadata: { currentWebview: { label: page.label ?? "main" } }, invoke },
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
    /** What the check sent the program, every attempt, in order. */
    reports: () =>
      invoke.mock.calls.map(([cmd, args]) => {
        expect(cmd).toBe("plugin:event|emit");
        expect(args).toMatchObject({ event: "voltip://ui-guard" });
        return (args as { payload: unknown }).payload;
      }),
    /** The reports the program has. */
    heard: () => heard,
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
  // A report that is not answered is sent again later; a test runs those timers itself.
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

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
          document: expect.any(String),
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
        document: expect.any(String),
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
        document: expect.any(String),
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

  it("regression: a report the program refused is sent again until the program has it", async () => {
    // CI 42546063667 and 42772486837 (hosted macOS, Apple silicon): the pill window's report never
    // reached app.log, while the same document's other calls did (the pill showed the refused edit
    // take). The check sent its report once and dropped a refusal.
    const page = open({
      scripts: ["/assets/index-NEW00001.js"],
      href: PILL,
      label: "overlay",
      invoke: ["rejects", "throws", "resolves"],
    });
    await vi.runAllTimersAsync();
    expect(page.heard()).toEqual([
      {
        window: "overlay",
        document: expect.any(String),
        verdict: "ok",
        replaced: false,
        scripts: ["/assets/index-NEW00001.js"],
        build: BUILD,
      },
    ]);
    // Three sends of the one report, then none: the program has it.
    const sent = page.reports() as { document: string }[];
    expect(sent).toHaveLength(3);
    expect(new Set(sent.map((report) => report.document)).size).toBe(1);
    expect(page.warn).not.toHaveBeenCalled();
  });

  it("regression: a report the program does not answer is sent again", async () => {
    // The same runs: a send that is never answered leaves no trace either.
    const page = open({ scripts: ["/assets/index-NEW00001.js"], invoke: ["hangs", "resolves"] });
    await vi.advanceTimersByTimeAsync(4_999);
    expect(page.reports()).toHaveLength(1);
    await vi.runAllTimersAsync();
    expect(page.reports()).toHaveLength(2);
    expect(page.heard()).toMatchObject([{ window: "main", verdict: "ok" }]);
  });

  it("stops sending once the program has the report, even from an answer that came late", async () => {
    // The first send is answered after the check stopped waiting for it: the second, already on its
    // way, is the last. Both carry the document's id, so the program logs the document once
    // (`ui_guard.rs`).
    const page = open({ scripts: ["/assets/index-NEW00001.js"], invoke: ["slow", "hangs"] });
    await vi.runAllTimersAsync();
    expect(page.reports()).toHaveLength(2);
    expect(page.heard()).toHaveLength(1);
    // The late answer lands while the next send waits after a refusal: that send is not made.
    // Sends at 0 s (answered at 6 s), 5 s and 5.5 s (refused); the next would go at 6.5 s.
    const waiting = open({
      scripts: ["/assets/index-NEW00001.js"],
      invoke: ["slow", "rejects", "rejects", "resolves"],
    });
    await vi.runAllTimersAsync();
    expect(waiting.reports()).toHaveLength(3);
    expect(waiting.heard()).toHaveLength(1);
  });

  it("gives up on a report the program keeps refusing, and says so", async () => {
    const page = open({ scripts: ["/assets/index-NEW00001.js"], invoke: "rejects" });
    await vi.runAllTimersAsync();
    expect(page.reports()).toHaveLength(8);
    expect(page.heard()).toEqual([]);
    expect(page.warn).toHaveBeenCalledWith(
      "voltip: the interface check could not report to the program",
      { verdict: "ok", replaced: false },
    );
    // The check itself stands.
    expect(page.window.voltipUiCheck).toEqual({ verdict: "ok", replaced: false });
    // Spaced 250 ms, doubling, up to 5 s: eight sends within 18 s.
    const again = open({ scripts: ["/assets/index-NEW00001.js"], invoke: "rejects" });
    await vi.advanceTimersByTimeAsync(17_750);
    expect(again.reports()).toHaveLength(8);
  });

  it("gives every document a report id of its own", () => {
    const one = open({ scripts: ["/assets/index-NEW00001.js"] });
    const two = open({ scripts: ["/assets/index-NEW00001.js"] });
    const id = (page: ReturnType<typeof open>) =>
      (page.reports()[0] as { document?: unknown }).document;
    expect(id(one)).toEqual(expect.any(String));
    expect(id(one)).not.toBe(id(two));
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
