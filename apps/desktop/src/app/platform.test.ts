import { platformFromIdentity, platformFromUserAgent, resolvePlatform } from "./platform";

const UA = {
  windows:
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36 Edg/130.0.0.0",
  macos:
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15",
  // WebKitGTK: the Linux Tauri webview also says AppleWebKit … Safari.
  webkitgtk:
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15",
  android:
    "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Mobile Safari/537.36",
  ios: "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1",
  ipad: "Mozilla/5.0 (iPad; CPU OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1",
};

describe("platformFromUserAgent", () => {
  it("resolves the three desktop agents", () => {
    expect(platformFromUserAgent(UA.windows)).toBe("windows");
    expect(platformFromUserAgent(UA.macos)).toBe("macos");
    expect(platformFromUserAgent(UA.webkitgtk)).toBe("linux");
  });

  it("regression: WebKitGTK is linux, not macos, so Linux never gets the traffic-light inset", () => {
    expect(platformFromUserAgent(UA.webkitgtk)).toBe("linux");
    expect(platformFromUserAgent("Mozilla/5.0 (X11; CrOS x86_64) Safari/537.36")).toBe("linux");
  });

  it("screens mobile agents before the desktop patterns they overlap with", () => {
    expect(platformFromUserAgent(UA.android)).toBe("unknown");
    expect(platformFromUserAgent(UA.ios)).toBe("unknown");
    expect(platformFromUserAgent(UA.ipad)).toBe("unknown");
  });

  it("degrades malformed input to unknown instead of throwing", () => {
    for (const ua of ["", "SomeFutureOS/1.0", "\u0000� not a user agent", "null"])
      expect(platformFromUserAgent(ua)).toBe("unknown");
  });

  it("always returns a member of the title-bar union", () => {
    for (const ua of [...Object.values(UA), "", "garbage"])
      expect(["macos", "windows", "linux", "unknown"]).toContain(platformFromUserAgent(ua));
  });
});

describe("resolvePlatform", () => {
  it("prefers the user agent and only falls back to the core identity when the UA is unknown", () => {
    expect(resolvePlatform("macos", UA.windows)).toBe("windows");
    expect(resolvePlatform("windows", UA.macos)).toBe("macos");
    expect(resolvePlatform("macos", "SomeFutureOS/1.0")).toBe("macos");
    expect(resolvePlatform("linux", "")).toBe("linux");
    expect(resolvePlatform("android", "")).toBe("unknown");
    expect(resolvePlatform(undefined, "")).toBe("unknown");
  });

  it("reads the live navigator by default (jsdom reports linux)", () => {
    expect(navigator.userAgent).toMatch(/linux/i);
    expect(resolvePlatform(undefined)).toBe("linux");
    expect(resolvePlatform("macos")).toBe("linux");
  });

  it("falls back to the identity hint when there is no navigator at all", () => {
    vi.stubGlobal("navigator", undefined);
    try {
      expect(resolvePlatform("windows")).toBe("windows");
      expect(resolvePlatform(undefined)).toBe("unknown");
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it("maps the core's platform enum onto the title-bar union", () => {
    expect(platformFromIdentity("macos")).toBe("macos");
    expect(platformFromIdentity("windows")).toBe("windows");
    expect(platformFromIdentity("linux")).toBe("linux");
    expect(platformFromIdentity("ios")).toBe("unknown");
    expect(platformFromIdentity("other")).toBe("unknown");
    expect(platformFromIdentity(undefined)).toBe("unknown");
  });
});
