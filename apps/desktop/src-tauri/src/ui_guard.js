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
  // This document's id: every send of its report carries it, so the program logs it once.
  const id = Math.random().toString(36).slice(2) + Math.random().toString(36).slice(2);
  // What the check found: on the window for whoever inspects it, and to the program's log
  // (`ui_guard.rs`). The check does not depend on the report, but the report is sent until the
  // program has it: a send the program refuses goes again 250 ms later, doubling up to 5 s, as a
  // refused listener does (packages/shared/src/tauri-backend.ts); one it has not answered after
  // `ANSWER_MS` goes again at once; `SENDS` sends at most. Hosted macOS lost the pill window's only
  // send while the same document's other calls went through (CI 42546063667, 42772486837).
  const SENDS = 8;
  const ANSWER_MS = 5000;
  const report = (verdict, running, build) => {
    window.voltipUiCheck = Object.freeze({ verdict, replaced });
    const internals = window.__TAURI_INTERNALS__;
    // Nothing to report to.
    if (!internals) return;
    const label = internals.metadata?.currentWebview?.label ?? null;
    const payload = { window: label, document: id, verdict, replaced, scripts: running, build: build ?? null };
    let heard = false;
    const send = (attempt) => {
      // An earlier send's answer came while this one waited.
      if (heard) return;
      let over = false;
      // This send got no answer, or a refusal: the next one, after `wait`.
      const again = (wait) => {
        if (over || heard) return;
        over = true;
        if (attempt + 1 < SENDS) setTimeout(() => send(attempt + 1), wait);
        else console.warn("voltip: the interface check could not report to the program", { verdict, replaced });
      };
      const unanswered = setTimeout(() => again(0), ANSWER_MS);
      let sent;
      try {
        sent = Promise.resolve(internals.invoke("plugin:event|emit", { event: "voltip://ui-guard", payload }));
      } catch (e) {
        sent = Promise.reject(e);
      }
      sent.then(
        () => {
          heard = true;
          clearTimeout(unanswered);
        },
        () => {
          clearTimeout(unanswered);
          again(Math.min(5000, 250 * 2 ** attempt));
        },
      );
    };
    send(0);
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
