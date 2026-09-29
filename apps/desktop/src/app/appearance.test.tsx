import { MockBackend } from "@voltip/shared/mock";
import { BackendProvider } from "@voltip/ui";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  APPEARANCE_STORAGE_KEY,
  AppearanceProvider,
  readLocalAppearance,
  useAppearance,
} from "./appearance";

function Probe() {
  const a = useAppearance();
  return (
    <div>
      <span data-testid="theme">{a.resolvedTheme}</span>
      <span data-testid="density">{a.local.density}</span>
      <button
        onClick={() => {
          a.setLocal({ density: "compact", fontSizePx: 16 });
        }}>
        compact
      </button>
      <button
        onClick={() => {
          a.preview("graphite");
        }}>
        preview
      </button>
      <button
        onClick={() => {
          a.preview(undefined);
        }}>
        restore
      </button>
    </div>
  );
}

describe("readLocalAppearance", () => {
  it("falls back to defaults on missing, malformed or out-of-range values", () => {
    expect(readLocalAppearance({ getItem: () => null })).toEqual({
      accent: "default",
      density: "default",
      fontSizePx: 14,
      reduceMotion: false,
    });
    expect(readLocalAppearance({ getItem: () => "{oops" }).density).toBe("default");
    expect(readLocalAppearance({ getItem: () => "42" }).density).toBe("default");
    expect(
      readLocalAppearance({
        getItem: () =>
          JSON.stringify({
            accent: "purple",
            density: "compact",
            fontSizePx: 99,
            reduceMotion: true,
          }),
      }),
    ).toEqual({
      accent: "purple",
      density: "compact",
      fontSizePx: 18,
      reduceMotion: true,
    });
  });
});

describe("AppearanceProvider", () => {
  it("regression: theme switching writes data-theme on <html> and follow-system resolves from the OS", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ settings: { theme: "warm" } });
    render(
      <BackendProvider backend={backend}>
        <AppearanceProvider>
          <Probe />
        </AppearanceProvider>
      </BackendProvider>,
    );
    await waitFor(() => {
      expect(document.documentElement.dataset.theme).toBe("warm");
    });
    await act(async () => {
      await backend.invoke("settings_set_theme", { theme: "dark", followSystem: false });
    });
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(screen.getByTestId("theme")).toHaveTextContent("dark");
    await act(async () => {
      await backend.invoke("settings_set_theme", { theme: "dark", followSystem: true });
    });
    expect(document.documentElement.dataset.theme).toBe("light");
    await user.click(screen.getByText("preview"));
    expect(document.documentElement.dataset.theme).toBe("graphite");
    await user.click(screen.getByText("restore"));
    expect(document.documentElement.dataset.theme).toBe("light");
    await user.click(screen.getByText("compact"));
    expect(document.documentElement.dataset.density).toBe("compact");
    expect(document.documentElement.style.getPropertyValue("--ui-font-size")).toBe("16px");
    expect(JSON.parse(window.localStorage.getItem(APPEARANCE_STORAGE_KEY) ?? "{}")).toMatchObject({
      density: "compact",
      fontSizePx: 16,
    });
  });

  it("an unknown accent reads as the default", () => {
    expect(readLocalAppearance({ getItem: () => JSON.stringify({ accent: "red" }) }).accent).toBe(
      "default",
    );
  });

  it("the accent is painted on <html>, and a change made in another window is followed", async () => {
    const backend = new MockBackend();
    window.localStorage.setItem(APPEARANCE_STORAGE_KEY, JSON.stringify({ accent: "green" }));
    render(
      <BackendProvider backend={backend}>
        <AppearanceProvider>
          <Probe />
        </AppearanceProvider>
      </BackendProvider>,
    );
    await waitFor(() => {
      expect(document.documentElement.dataset.accent).toBe("green");
    });
    // The settings window wrote a new choice: this window (the pill's, say) follows.
    act(() => {
      window.localStorage.setItem(
        APPEARANCE_STORAGE_KEY,
        JSON.stringify({ accent: "orange", density: "compact" }),
      );
      window.dispatchEvent(new StorageEvent("storage", { key: APPEARANCE_STORAGE_KEY }));
    });
    expect(document.documentElement.dataset.accent).toBe("orange");
    expect(screen.getByTestId("density")).toHaveTextContent("compact");
    // Other keys are not its business.
    act(() => {
      window.localStorage.setItem(APPEARANCE_STORAGE_KEY, JSON.stringify({ accent: "pink" }));
      window.dispatchEvent(new StorageEvent("storage", { key: "voltip.locale" }));
    });
    expect(document.documentElement.dataset.accent).toBe("orange");
    window.localStorage.removeItem(APPEARANCE_STORAGE_KEY);
  });

  it("guards hook usage", () => {
    expect(() => render(<Probe />)).toThrow(
      "useAppearance must be used inside <AppearanceProvider>",
    );
  });
});
