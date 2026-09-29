// Overflow check for the copy rewrite (plan 1.2). Run in the desktop web preview
// (`pnpm --filter @voltip/desktop exec vite --host <ip>`, MockBackend) through chrome-devtools
// `evaluate_script` at 1280×800 and 1440×900 (`emulate` viewport), once under 中文 and once under
// English. It opens every page of the main layout, every settings group and the four set-up steps
// in the app (pushState + popstate, so the mock keeps its state) and reports, per route:
//   page       the document scrolls sideways (must be false);
//   outside    text that ends right of the window (must be empty);
//   cut        text cut off without both an ellipsis and a title to read it whole (must be empty);
//   spill      text wider than its own box, running over its neighbours (must be empty);
//   scrollers  boxes that scroll sideways, with the overhang in px (must be empty);
//   ellipsis   text shortened with an ellipsis, for review. User content (a result's text, marked
//              `data-user-text`, read whole in the detail) may be shortened without a title; any
//              other text shortened without one is also listed under `cut`.
async () => {
  const sections = ["general", "hotkey", "dictation", "microphone", "scene", "privacy", "appearance", "about"];
  const routes = [
    "/",
    "/history",
    "/dictionary",
    "/rules",
    "/devices",
    "/speech",
    "/ai",
    "/feedback",
    ...sections.map((id) => `/settings/${id}`),
    ...[1, 2, 3, 4].map((step) => `/onboarding?step=${step}`),
  ];
  const go = async (path) => {
    history.pushState(null, "", path);
    dispatchEvent(new PopStateEvent("popstate"));
    await new Promise((resolve) => setTimeout(resolve, 600));
  };
  const text = (el) => el.textContent.trim().replace(/\s+/g, " ").slice(0, 60);
  const ownText = (el) => [...el.childNodes].some((n) => n.nodeType === Node.TEXT_NODE && n.textContent.trim());
  // Nothing but user content: no text left once the `data-user-text` parts and decorations go.
  const userContent = (el) => {
    if (el.matches("[data-user-text]")) return true;
    const copy = el.cloneNode(true);
    for (const part of copy.querySelectorAll("[data-user-text], [aria-hidden='true']")) part.remove();
    return !copy.textContent.trim();
  };
  const shown = (el, rect) =>
    rect.width > 1 && rect.height > 1 && getComputedStyle(el).visibility !== "hidden" && !el.closest("[aria-hidden='true']");
  const out = { viewport: `${innerWidth}x${innerHeight}`, lang: document.documentElement.lang };
  for (const path of routes) {
    await go(path);
    const report = { page: document.scrollingElement.scrollWidth > innerWidth + 1, outside: [], cut: [], spill: [], scrollers: [], ellipsis: [] };
    for (const el of document.body.querySelectorAll("*")) {
      const rect = el.getBoundingClientRect();
      if (!shown(el, rect)) continue;
      if (ownText(el) && rect.right > innerWidth + 1) report.outside.push(text(el));
      if (el.scrollWidth <= el.clientWidth + 1) continue;
      const style = getComputedStyle(el);
      if (style.overflowX === "auto" || style.overflowX === "scroll") {
        report.scrollers.push({ box: `${el.tagName.toLowerCase()}.${[...el.classList].slice(0, 3).join(".")}`, overhang: el.scrollWidth - el.clientWidth });
        continue;
      }
      if (style.overflowX === "visible") {
        if (ownText(el)) report.spill.push(text(el));
        continue;
      }
      if (el.children.length > 3 || !el.textContent.trim()) continue;
      if (style.textOverflow === "ellipsis") {
        const title = el.title || el.closest("[title]")?.title || "";
        const user = userContent(el);
        report.ellipsis.push({ text: text(el), title: Boolean(title), user });
        if (!title && !user) report.cut.push(text(el));
      } else {
        report.cut.push(text(el));
      }
    }
    out[path] = report;
  }
  await go("/");
  return out;
};
