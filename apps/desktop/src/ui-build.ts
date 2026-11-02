// The build this bundle is (`vite.config.ts` names every build), declared on the window for the
// program's interface check (src-tauri/src/ui_guard.js, docs/dictation.md §33.4): a window whose
// bundle declares another build, or none, runs another build's interface.

// oxlint-disable-next-line no-underscore-dangle -- the name vite.config.ts defines, in Vite's form for build constants
declare const __VOLTIP_UI_BUILD__: string | undefined;

declare global {
  interface Window {
    voltipUiBuild?: string;
  }
}

/** The build `vite build` gave this bundle; undefined where nothing named it (tests). */
export const UI_BUILD: string | undefined =
  typeof __VOLTIP_UI_BUILD__ === "string" ? __VOLTIP_UI_BUILD__ : undefined;

if (UI_BUILD !== undefined) window.voltipUiBuild = UI_BUILD;
