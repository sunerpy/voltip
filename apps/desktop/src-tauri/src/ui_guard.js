// Runs in every window's document as it is created (`ui_guard.rs`, docs/dictation.md §33.4). The
// interface a window runs must be this program's: every module script among the ones the program
// carries, and the build the bundle declares (`window.voltipUiBuild`, apps/desktop/src/
// ui-build.ts) the one the program's bundle carries. A document that fails either runs another
// build's interface. It is replaced once by a document at a URL this webview has never asked for,
// which no cache can answer, so the program serves it; should that one still run other code, the
// window is cleared rather than left with it. `ui_guard.rs` puts the program's script paths and
// builds in place of the two placeholders below.
(() => {
  const carried = __VOLTIP_UI_SCRIPTS__;
  const builds = __VOLTIP_UI_BUILDS__;
  const marker = "voltip-ui-reload";
  // The program's own pages only: tauri://localhost (macOS, Linux), http://tauri.localhost
  // (Windows).
  if (location.protocol !== "tauri:" && location.hostname !== "tauri.localhost") return;
  // A document the check reloaded carries the marker; it leaves the URL before the interface reads it.
  const here = new URL(location.href);
  const reloaded = here.searchParams.has(marker);
  if (reloaded) {
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
  const check = () => {
    const running = Array.from(document.querySelectorAll('script[type="module"][src]'), (s) =>
      path(s.getAttribute("src")),
    );
    // Not the interface (a blank page, an error page).
    if (running.length === 0) return;
    const build = window.voltipUiBuild;
    const scripts = running.every((p) => carried.includes(p));
    if (scripts && (builds.length === 0 || builds.includes(build))) return;
    if (!reloaded) {
      console.warn("voltip: this window runs interface code the program does not carry; reloading", running, build);
      const next = new URL(location.href);
      next.searchParams.set(marker, String(Date.now()));
      location.replace(next.href);
      return;
    }
    console.error("voltip: the reloaded window still runs interface code the program does not carry; clearing it", running, build);
    location.replace("about:blank");
  };
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", check, { once: true });
  } else {
    check();
  }
})();
