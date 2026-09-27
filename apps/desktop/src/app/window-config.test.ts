import { existsSync, readFileSync } from "node:fs";
import path from "node:path";

/** Reads the real Tauri config off disk: the cheapest guard against the RFC 7396 array trap
 *  (the macOS override replaces `app.windows` wholesale) and against permission drift.
 *  vitest runs with the package as cwd; the repo root is accepted too. */
function locateSrcTauri(): string {
  const found = ["src-tauri", "apps/desktop/src-tauri"]
    .map((dir) => path.resolve(process.cwd(), dir))
    .find((dir) => existsSync(path.join(dir, "tauri.conf.json")));
  if (found === undefined) throw new Error(`src-tauri not found from ${process.cwd()}`);
  return found;
}
const SRC_TAURI = locateSrcTauri();

type WindowConfig = Record<string, unknown>;

function readJson(file: string): Record<string, unknown> {
  const parsed: unknown = JSON.parse(readFileSync(path.join(SRC_TAURI, file), "utf8"));
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed))
    throw new Error(`${file} is not a JSON object`);
  return parsed as Record<string, unknown>;
}

function mainWindow(file: string): WindowConfig {
  const app = readJson(file).app as { windows?: WindowConfig[] } | undefined;
  const windows = app?.windows;
  if (windows === undefined || windows.length !== 1)
    throw new Error(`${file} must declare exactly one app.windows entry`);
  const [win] = windows;
  if (win === undefined || win.label !== "main")
    throw new Error(`${file} app.windows[0] must be the main window`);
  return win;
}

const SHARED_GEOMETRY = ["label", "title", "width", "height", "minWidth", "minHeight"] as const;

describe("tauri window configuration", () => {
  const base = mainWindow("tauri.conf.json");
  const macos = mainWindow("tauri.macos.conf.json");

  it("turns decorations off in the base config so Windows and Linux draw our title bar", () => {
    expect(base.decorations).toBe(false);
  });

  it("keeps the undecorated shadow on, which is what preserves Windows 11 rounded corners", () => {
    expect(base.shadow).toBe(true);
  });

  it("keeps the native macOS traffic lights instead of drawing a second set", () => {
    expect(macos.decorations).toBe(true);
    expect(macos.titleBarStyle).toBe("Overlay");
    expect(macos.hiddenTitle).toBe(true);
  });

  it("positions the traffic lights where the sidebar's 72 px inset expects them", () => {
    // Sidebar `pl-15` (60 px) + nav `px-3` (12 px) = 72 px = x 12 + 2 × 20 pitch + 12 button + 8.
    // y 14 centres the 12 px buttons in the 40 px strip.
    expect(macos.trafficLightPosition).toEqual({ x: 12, y: 14 });
  });

  it("regression: restates the shared geometry identically, because RFC 7396 replaces the array", () => {
    for (const key of SHARED_GEOMETRY)
      expect(macos[key], `app.windows[0].${key} drifted between the two configs`).toEqual(
        base[key],
      );
  });

  it("lets each platform override touch only its own bundle block, never build or the app identity", () => {
    // docs/dictation.md §15.5: macOS adds `bundle.macOS` (minimum version, the two sherpa dylibs,
    // entitlements, localised prompt strings), Windows `bundle.resources` + `bundle.windows`. The
    // shared bundle (targets, icons, publisher) and the build / identity keys stay in tauri.conf.json.
    const mac = readJson("tauri.macos.conf.json");
    expect(Object.keys(mac.bundle ?? {})).toEqual(["macOS"]);
    const win = readJson("tauri.windows.conf.json");
    expect(new Set(Object.keys(win.bundle ?? {}))).toEqual(new Set(["resources", "windows"]));
    for (const raw of [mac, win]) {
      expect(raw.build).toBeUndefined();
      expect(raw.productName).toBeUndefined();
      expect(raw.version).toBeUndefined();
      expect(raw.identifier).toBeUndefined();
    }
  });
});

describe("window capability permissions", () => {
  const capability = readJson("capabilities/default.json") as {
    windows: string[];
    permissions: string[];
  };

  it("grants every window command the self-drawn title bar invokes", () => {
    // `core:window:default` already covers `is_maximized` and the `internal_toggle_maximize`
    // that Tauri's own drag script fires on double-click; these four are not in that set.
    expect(capability.permissions).toContain("core:window:allow-minimize");
    expect(capability.permissions).toContain("core:window:allow-toggle-maximize");
    expect(capability.permissions).toContain("core:window:allow-close");
    expect(capability.permissions).toContain("core:window:allow-start-dragging");
  });

  it("still includes the core default set that is_maximized relies on, and covers the main window", () => {
    expect(capability.permissions).toContain("core:default");
    expect(capability.windows).toContain("main");
  });
});
