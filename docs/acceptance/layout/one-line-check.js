// One-line layout check (plan 1.6, user feedback 2026-09-29). Run in the desktop web preview
// (`pnpm --filter @voltip/desktop exec vite --host <ip>`, MockBackend) through chrome-devtools
// `evaluate_script` with the settings dialog open, at 1440×900, 1920×1080 and the 960 px minimum
// window width (`emulate` viewport). It walks every settings group and reports, per group:
//   helps / fit   StatusRow help lines, and how many fit their column on one line;
//   wrapped       help that fits its column but still went to a second line (must be empty);
//   cutOptions    radios, tabs and options cut off without an ellipsis (must be empty);
//   selects       native selects narrower than their chosen label plus padding (must be empty;
//                 confirm a hit on a screenshot: the select draws its text itself);
//   cut           text cut off without both an ellipsis and a title to read it whole (must be empty).
async () => {
  const textLines = (el) => {
    const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
    const tops = [];
    for (let n = walker.nextNode(); n; n = walker.nextNode()) {
      if (!n.textContent.trim()) continue;
      const range = document.createRange();
      range.selectNodeContents(n);
      for (const rect of range.getClientRects()) if (rect.width > 0) tops.push(rect.top);
    }
    tops.sort((a, b) => a - b);
    const lineHeight = parseFloat(getComputedStyle(el).lineHeight) || 16;
    let count = tops.length ? 1 : 0;
    for (let i = 1; i < tops.length; i++) if (tops[i] - tops[i - 1] > lineHeight * 0.6) count++;
    return count;
  };
  const naturalWidth = (el) => {
    const copy = el.cloneNode(true);
    copy.style.cssText = "position:absolute;visibility:hidden;white-space:nowrap;width:auto;max-width:none;left:0;top:0";
    document.body.appendChild(copy);
    const width = copy.getBoundingClientRect().width;
    copy.remove();
    return width;
  };
  const text = (el) => el.textContent.trim().replace(/\s+/g, " ").slice(0, 50);
  const out = { viewport: `${innerWidth}x${innerHeight}` };
  for (const id of ["general", "hotkey", "dictation", "microphone", "scene", "privacy", "appearance", "about"]) {
    document.getElementById(`vt-settings-tab-${id}`)?.click();
    await new Promise((r) => setTimeout(r, 400));
    const dialog = document.querySelector('[role="dialog"]');
    let helps = 0;
    let fit = 0;
    const wrapped = [];
    for (const help of dialog.querySelectorAll(".min-h-\\[52px\\] > .min-w-0 > div:nth-child(2)")) {
      helps++;
      if (naturalWidth(help) <= help.parentElement.getBoundingClientRect().width - 1) {
        fit++;
        if (textLines(help) > 1) wrapped.push(text(help));
      }
    }
    const cutOptions = [];
    for (const el of dialog.querySelectorAll('[role="radio"], [role="tab"], [role="option"]')) {
      if (el.getClientRects().length && el.scrollWidth > el.clientWidth + 1 && getComputedStyle(el).textOverflow !== "ellipsis") cutOptions.push(text(el));
    }
    const selects = [];
    for (const select of dialog.querySelectorAll("select")) {
      const style = getComputedStyle(select);
      const probe = document.createElement("span");
      probe.textContent = select.selectedOptions[0]?.textContent ?? "";
      probe.style.cssText = `position:absolute;visibility:hidden;white-space:nowrap;font:${style.font}`;
      document.body.appendChild(probe);
      const need = probe.getBoundingClientRect().width + parseFloat(style.paddingLeft) + parseFloat(style.paddingRight) + 2;
      probe.remove();
      if (select.getBoundingClientRect().width + 0.5 < need) selects.push({ chosen: text(select.selectedOptions[0] ?? select), width: Math.round(select.getBoundingClientRect().width), need: Math.round(need) });
    }
    const cut = [];
    for (const el of dialog.querySelectorAll("*")) {
      if (el.children.length > 3 || el.getClientRects().length === 0 || !el.textContent.trim()) continue;
      const style = getComputedStyle(el);
      if (el.scrollWidth <= el.clientWidth + 1 || style.overflowX === "visible") continue;
      if (style.textOverflow !== "ellipsis" || !(el.title || el.closest("[title]")?.title)) cut.push(text(el));
    }
    out[id] = { helps, fit, wrapped, cutOptions, selects, cut };
  }
  return out;
};
