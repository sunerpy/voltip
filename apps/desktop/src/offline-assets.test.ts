import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const here = resolve(import.meta.dirname, "..");

/** regression (Windows test 2026-09-24): the webview fetched its fonts from Google Fonts, so an
 *  offline or firewalled machine rendered the system font and the app stopped matching its design.
 *  Every asset must ship inside the bundle; the CSP must not grant any remote host. */
describe("offline assets", () => {
  it("regression: index.html references no remote stylesheet or font host", () => {
    const html = readFileSync(resolve(here, "index.html"), "utf8");
    expect(html).not.toMatch(/https?:\/\//);
    expect(html).not.toContain("googleapis");
  });

  it("regression: the Tauri CSP grants no remote font or style origin", () => {
    const conf: { app: { security: { csp: string } } } = JSON.parse(
      readFileSync(resolve(here, "src-tauri/tauri.conf.json"), "utf8"),
    );
    const csp = conf.app.security.csp;
    expect(csp).toContain("font-src 'self' data:");
    expect(csp).not.toMatch(/https:\/\/fonts\./);
    expect(csp).not.toContain("googleapis");
  });

  it("regression: the shared tokens bundle the three design families", () => {
    const tokens = readFileSync(resolve(here, "../../packages/ui/src/tokens.css"), "utf8");
    for (const family of ["instrument-sans", "jetbrains-mono", "noto-sans-sc"])
      expect(tokens).toContain(`@import "@fontsource-variable/${family}/index.css";`);
  });
});
