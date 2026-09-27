import { isTauri } from "@tauri-apps/api/core";

export interface Scanner {
  /** Resolves with the QR payload, or `undefined` when the user cancelled / denied the camera. */
  scan(): Promise<string | undefined>;
}

/** Wraps `@tauri-apps/plugin-barcode-scanner`; returns `undefined` outside Tauri so the UI falls back to pasting the link. */
export async function loadScanner(): Promise<Scanner | undefined> {
  if (!isTauri()) return undefined;
  const plugin = await import("@tauri-apps/plugin-barcode-scanner");
  return {
    async scan() {
      let permission = await plugin.checkPermissions();
      if (permission !== "granted") permission = await plugin.requestPermissions();
      if (permission !== "granted") return undefined;
      const result = await plugin.scan({ windowed: false, formats: [plugin.Format.QRCode] });
      return result.content;
    },
  };
}
