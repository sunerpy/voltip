import {
  MOCK_AVAILABLE_VERSION,
  MOCK_CURRENT_VERSION,
  MOCK_UPDATE_NOTES,
  MockBackend,
} from "@voltip/shared/mock";
import { act, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../test/render";

// Updates on the phone (user request 2026-10-02, docs/dictation.md §20.9): Google Play updates
// what it installed; any other install asks GitHub and opens the newer release's APK.
describe("updates on the phone", () => {
  async function openAbout(user: ReturnType<typeof userEvent.setup>) {
    await user.click(await screen.findByTestId("tab-settings"));
    await user.click(await screen.findByTestId("settings-about"));
    return screen.findByTestId("phone-update");
  }

  it("an install from outside Google Play checks GitHub, opens the new version's installer and follows 自动检查更新", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ role: "phone" });
    renderApp({ backend });
    const card = await openAbout(user);
    expect(within(card).getByRole("heading", { name: "软件更新" })).toBeInTheDocument();
    expect(card).toHaveTextContent("检查更新时会查询 GitHub 上的最新发布");
    expect(within(card).getByTestId("phone-update-status")).toHaveTextContent("尚未检查更新");
    expect(within(card).queryByRole("button", { name: "下载新版本" })).toBeNull();
    await user.click(within(card).getByRole("button", { name: "检查更新" }));
    expect(
      await within(card).findByText(
        `有新版本 ${MOCK_AVAILABLE_VERSION} · 当前 ${MOCK_CURRENT_VERSION}`,
      ),
    ).toHaveAttribute("data-testid", "phone-update-status");
    const notes = within(card).getByText("更新说明");
    expect(notes.closest("details")).toHaveTextContent(MOCK_UPDATE_NOTES.split("\n")[0] ?? "");
    await user.click(within(card).getByRole("button", { name: "下载新版本" }));
    expect(backend.updatePagesOpened).toEqual([MOCK_AVAILABLE_VERSION]);
    expect(card).toHaveTextContent("Android 只接受与当前版本签名相同的安装包");
    // 自动检查更新 is the shared `auto_update` setting, off by default as on the desktop.
    const auto = within(card).getByRole("switch", { name: "自动检查更新" });
    expect(auto).toHaveAttribute("aria-checked", "false");
    await user.click(auto);
    expect(await within(card).findByRole("switch", { name: "自动检查更新" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    expect(backend.peek().settings.auto_update).toBe(true);
    backend.destroy();
  });

  it("the update line says where a check ended, with the desktop's lamp for it", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ role: "phone" });
    renderApp({ backend });
    const card = await openAbout(user);
    const status = () => within(card).getByTestId("phone-update-status");
    const lamp = () => card.querySelector("[data-tone]")?.getAttribute("data-tone");
    expect(lamp()).toBe("idle");
    act(() => {
      backend.simulateUpdate({ state: "checking" });
    });
    expect(status()).toHaveTextContent("正在检查更新…");
    expect(lamp()).toBe("accent");
    expect(within(card).getByRole("button", { name: "检查更新" })).toBeDisabled();
    act(() => {
      backend.simulateUpdate({ state: "up_to_date", version: "0.0.1", checked_at: 1_790_000_000 });
    });
    expect(status()).toHaveTextContent(/^已是最新 · 0\.0\.1 · 检查于 /);
    expect(lamp()).toBe("ok");
    act(() => {
      backend.simulateUpdate({ state: "failed", message: "update: 无法连接 GitHub" });
    });
    // The machine prefix goes; the line turns the danger colour.
    expect(status()).toHaveTextContent("更新失败 · 无法连接 GitHub");
    expect(status()).toHaveClass("text-danger");
    expect(lamp()).toBe("danger");
    backend.destroy();
  });

  it("an install from Google Play is updated by Play: the card opens the listing and checks nothing", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({
      role: "phone",
      update: { state: "store", version: MOCK_CURRENT_VERSION },
    });
    renderApp({ backend });
    const card = await openAbout(user);
    expect(within(card).getByRole("heading", { name: "由 Google Play 更新" })).toBeInTheDocument();
    expect(card).toHaveTextContent("Google Play 会自动更新它");
    expect(within(card).queryByRole("button", { name: "检查更新" })).toBeNull();
    expect(within(card).queryByRole("switch")).toBeNull();
    await user.click(within(card).getByRole("button", { name: "在 Google Play 中打开" }));
    expect(backend.updatePagesOpened).toEqual(["store"]);
    expect(backend.peek().update).toEqual({ state: "store", version: MOCK_CURRENT_VERSION });
    backend.destroy();
  });
});
