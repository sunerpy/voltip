// Runs in every window's document as it is created (`ui_guard.rs`, docs/dictation.md §33.4). A
// document whose module scripts are not all among the ones this program carries runs the interface
// of another build, and it reloads once. `ui_guard.rs` puts the paths of the scripts the program
// carries in place of the placeholder below. The once is kept in the window's session storage: a
// window that still runs other code after its reload stays as it is, and says so in its console.
(() => {
  const carried = __VOLTIP_UI_SCRIPTS__;
  const key = "voltip.ui-guard.reloaded";
  // The program's own pages only: tauri://localhost (macOS, Linux), http://tauri.localhost
  // (Windows).
  if (location.protocol !== "tauri:" && location.hostname !== "tauri.localhost") return;
  const store = (() => {
    try {
      return window.sessionStorage;
    } catch {
      return undefined;
    }
  })();
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
    if (running.every((p) => carried.includes(p))) {
      store?.removeItem(key);
      return;
    }
    const once = carried.join(" ");
    if (store === undefined || store.getItem(key) === once) {
      console.warn("voltip: this window runs interface code the program does not carry", running);
      return;
    }
    store.setItem(key, once);
    console.warn("voltip: this window runs interface code the program does not carry; reloading", running);
    location.reload();
  };
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", check, { once: true });
  } else {
    check();
  }
})();
