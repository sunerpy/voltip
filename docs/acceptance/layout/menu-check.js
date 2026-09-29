// Preset menu check (plan 2.3, the one-line rule of plan 1.6). Run in the desktop web preview
// (MockBackend) through chrome-devtools `evaluate_script` on the home page, at 1440×900, 1920×1080
// and the 960 px minimum window width (`emulate` viewport), in 中文 and in English. It opens the
// home page's preset menu and the title bar's, and reports, per menu:
//   rows      every row's text and width;
//   wrapped   rows whose text takes more than one line (must be empty);
//   cut       rows whose text is cut off (must be empty);
//   outside   the menu's box reaches past the window (must be false).
async () => {
  const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  const lines = (el) => {
    const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
    const tops = [];
    for (let n = walker.nextNode(); n; n = walker.nextNode()) {
      if (!n.textContent.trim()) continue;
      const range = document.createRange();
      range.selectNodeContents(n);
      for (const rect of range.getClientRects()) if (rect.width > 0) tops.push(rect.top);
    }
    tops.sort((a, b) => a - b);
    let count = tops.length ? 1 : 0;
    for (let i = 1; i < tops.length; i++) if (tops[i] - tops[i - 1] > 6) count++;
    return count;
  };
  const out = { viewport: `${innerWidth}x${innerHeight}`, lang: document.documentElement.lang };
  for (const id of ["home-preset", "polish-preset"]) {
    const trigger = document.querySelector(`[data-testid="${id}"]`);
    if (!trigger) {
      out[id] = "no trigger on this page";
      continue;
    }
    trigger.click();
    await pause(300);
    const menu = document.querySelector(`[data-testid="${id}-menu"]`);
    if (!menu) {
      out[id] = "the menu did not open";
      continue;
    }
    const box = menu.getBoundingClientRect();
    const rows = [...menu.querySelectorAll('[role="menuitem"], [role="menuitemradio"]')];
    out[id] = {
      trigger: Math.round(trigger.getBoundingClientRect().width),
      menu: `${Math.round(box.left)}–${Math.round(box.right)} × ${Math.round(box.top)}–${Math.round(box.bottom)}`,
      outside: box.left < 0 || box.right > innerWidth || box.bottom > innerHeight,
      rows: rows.map((row) => `${row.textContent.trim()} (${Math.round(row.getBoundingClientRect().width)})`),
      wrapped: rows.filter((row) => lines(row) > 1).map((row) => row.textContent.trim()),
      cut: rows.filter((row) => row.scrollWidth > row.clientWidth + 1).map((row) => row.textContent.trim()),
    };
    document.activeElement?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await pause(200);
  }
  return out;
};
