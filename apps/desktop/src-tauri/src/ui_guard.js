// Runs in every window's document as it is created (`ui_guard.rs`, docs/dictation.md §33.4). The
// interface a window runs must be this program's: every module script among the ones the program
// carries, and the build the bundle declares (`window.voltipUiBuild`, apps/desktop/src/
// ui-build.ts) the one the program's bundle carries. A document that fails either runs another
// build's interface. It is replaced once by a document at a URL this webview has never asked for,
// which no cache can answer, so the program serves it. Should that one still run other code, the
// window keeps it and the reason goes to the console and the program's log: a mismatch that
// survives a fresh document more likely means the check misjudges this webview, and a blank or
// reloading window would leave every start unusable. `ui_guard.rs` puts the program's script paths
// and builds in place of the two placeholders below.
(() => {
  const carried = __VOLTIP_UI_SCRIPTS__;
  const builds = __VOLTIP_UI_BUILDS__;
  const marker = "voltip-ui-reload";
  // The program's own pages only: tauri://localhost (macOS, Linux), http://tauri.localhost
  // (Windows).
  if (location.protocol !== "tauri:" && location.hostname !== "tauri.localhost") return;
  // A document the check replaced carries the marker; it leaves the URL before the interface reads it.
  const here = new URL(location.href);
  const replaced = here.searchParams.has(marker);
  if (replaced) {
    here.searchParams.delete(marker);
    history.replaceState(history.state, "", here.pathname + here.search + here.hash);
  }
  const path = (src) => {
    const raw = new URL(src, location.href).pathname;
    try {
      return decodeURI(raw);
    } catch {
      return raw;
    }
  };
  // What the check found: on the window for whoever inspects it, and to the program's log
  // (`ui_guard.rs`). The report is best effort; the check does not depend on it.
  const report = (verdict, running, build) => {
    window.voltipUiCheck = Object.freeze({ verdict, replaced });
    try {
      const internals = window.__TAURI_INTERNALS__;
      const label = internals?.metadata?.currentWebview?.label ?? null;
      const payload = { window: label, verdict, replaced, scripts: running, build: build ?? null };
      internals?.invoke("plugin:event|emit", { event: "voltip://ui-guard", payload })?.catch?.(() => {});
    } catch {
      // Nothing to report to.
    }
  };
  const check = () => {
    const running = Array.from(document.querySelectorAll('script[type="module"][src]'), (s) =>
      path(s.getAttribute("src")),
    );
    // Not the interface (a blank page, an error page).
    if (running.length === 0) return;
    const build = window.voltipUiBuild;
    const scripts = running.every((p) => carried.includes(p));
    if (scripts && (builds.length === 0 || builds.includes(build))) {
      report("ok", running, build);
      return;
    }
    if (!replaced) {
      console.warn("voltip: this window runs interface code the program does not carry; replacing its document once", running, build);
      report("replacing", running, build);
      const next = new URL(location.href);
      next.searchParams.set(marker, String(Date.now()));
      location.replace(next.href);
      return;
    }
    console.error("voltip: the window still runs interface code the program does not carry after its document was replaced once; it is kept as it is", running, build);
    report("kept", running, build);
  };
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", check, { once: true });
  } else {
    check();
  }
})();
