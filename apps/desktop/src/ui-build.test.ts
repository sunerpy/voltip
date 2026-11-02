// The build the bundle declares for the program's interface check (docs/dictation.md §33.4).
const BUILD = "voltip-ui-build-0123456789abcdef0123456789abcdef";

describe("ui-build", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.resetModules();
    delete window.voltipUiBuild;
  });

  it("declares the build vite named on the window", async () => {
    vi.stubGlobal("__VOLTIP_UI_BUILD__", BUILD);
    const { UI_BUILD } = await import("./ui-build");
    expect(UI_BUILD).toBe(BUILD);
    expect(window.voltipUiBuild).toBe(BUILD);
  });

  it("declares nothing where nothing named the build", async () => {
    const { UI_BUILD } = await import("./ui-build");
    expect(UI_BUILD).toBeUndefined();
    expect(window.voltipUiBuild).toBeUndefined();
  });
});
