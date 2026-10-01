import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../test/render";

describe("the switchers design sheet (`/design/switchers`, pnpm dev only)", () => {
  it("draws every frame chrome-less, with the menus of the proposal opened and working", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/design/switchers" });
    const page = await screen.findByTestId("page-design-switchers");
    for (const id of [
      "design-bar-a",
      "design-bar-b",
      "design-bar-narrow",
      "design-menu-speech",
      "design-menu-mic",
      "design-menu-polish",
      "design-menu-polish-b",
      "design-home-ready",
      "design-home-mic",
      "design-home-engine",
      "design-tray",
    ])
      expect(within(page).getByTestId(id)).toBeInTheDocument();
    // A spec sheet: no sidebar, the strip names the sheet.
    expect(screen.queryByRole("navigation")).toBeNull();
    // The 语音模型 menu of the ready bar: the built-in service in use, the rest still to set up.
    const speech = screen.getByTestId("design-home-speech-menu");
    expect(within(speech).getByRole("menuitemradio", { name: "Qwen3-ASR-1.7B" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    expect(within(speech).getByRole("menuitemradio", { name: "OpenAI · 缺少密钥" })).toBeDisabled();
    // Option B's one menu lists the presets and the models together.
    const b = screen.getByTestId("design-bar-polish-b-menu");
    expect(
      within(b)
        .getAllByRole("group")
        .map((g) => g.getAttribute("aria-label")),
    ).toContain("内置预设");
    expect(
      within(b)
        .getAllByRole("group")
        .map((g) => g.getAttribute("aria-label")),
    ).toContain("内置服务");
    // The opened menus are the real ones: picking another microphone saves it (a press closes the
    // other opened menus, so this comes after reading them).
    const mic = screen.getByTestId("design-card-mic-menu");
    const other = await within(mic).findByRole("menuitemradio", { name: "Realtek Audio" });
    await user.click(other);
    await waitFor(async () => {
      expect((await backend.getState()).settings.microphone).not.toBeNull();
    });
    // The theme switch of the sheet drives the preview's theme.
    await user.click(screen.getByRole("radio", { name: "深色" }));
    await waitFor(async () => {
      expect((await backend.getState()).settings.theme).toBe("dark");
    });
  });

  it("is not found under an unknown sheet name", async () => {
    renderApp({ path: "/design/elsewhere" });
    expect(await screen.findByText("/design/elsewhere")).toBeInTheDocument();
  });
});
