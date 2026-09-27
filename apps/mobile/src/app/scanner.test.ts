const tauri = vi.hoisted(() => ({ isTauri: vi.fn(() => false) }));
const plugin = vi.hoisted(() => ({
  checkPermissions: vi.fn(),
  requestPermissions: vi.fn(),
  scan: vi.fn(),
  Format: { QRCode: "QR_CODE" },
}));

vi.mock("@tauri-apps/api/core", () => tauri);
vi.mock("@tauri-apps/plugin-barcode-scanner", () => plugin);

import { loadScanner } from "./scanner";

describe("loadScanner", () => {
  it("returns undefined outside Tauri", async () => {
    tauri.isTauri.mockReturnValue(false);
    expect(await loadScanner()).toBeUndefined();
  });

  it("requests camera permission and scans a QR code inside Tauri", async () => {
    tauri.isTauri.mockReturnValue(true);
    plugin.checkPermissions.mockResolvedValue("prompt");
    plugin.requestPermissions.mockResolvedValue("granted");
    plugin.scan.mockResolvedValue({ content: "voltip://pair?v=1", format: "QR_CODE" });
    const scanner = await loadScanner();
    expect(scanner).toBeDefined();
    expect(await scanner?.scan()).toBe("voltip://pair?v=1");
    expect(plugin.scan).toHaveBeenCalledWith({ windowed: false, formats: ["QR_CODE"] });
    plugin.checkPermissions.mockResolvedValue("granted");
    expect(await scanner?.scan()).toBe("voltip://pair?v=1");
    plugin.checkPermissions.mockResolvedValue("denied");
    plugin.requestPermissions.mockResolvedValue("denied");
    expect(await scanner?.scan()).toBeUndefined();
  });
});
