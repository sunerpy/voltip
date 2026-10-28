// Updates on the phone (docs/dictation.md §20.9), apps/mobile's About tests on this app: Google Play
// updates what it installed; any other install asks GitHub and opens the newer release's APK. The
// Tauri phone app had this up to 0.0.49; this app has it again from the version after 0.0.50.
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react-native";
import {
  MOCK_AVAILABLE_VERSION,
  MOCK_CURRENT_VERSION,
  MOCK_UPDATE_NOTES,
} from "@voltip/shared/mock";

import { openTab, renderApp } from "../test/render";
import { appTheme } from "../theme/themes";
import { toneColor } from "../ui/kit";

async function openAbout() {
  await openTab("settings");
  await fireEvent.press(await screen.findByTestId("settings-about"));
  return screen.findByTestId("phone-update");
}

describe("updates on the phone", () => {
  it("an install from outside Google Play checks GitHub, opens the new version's APK and follows 自动检查更新", async () => {
    const { backend } = await renderApp();
    const card = await openAbout();
    expect(within(card).getByText("软件更新")).toBeOnTheScreen();
    expect(within(card).getByText(/检查更新时会查询 GitHub 上的最新发布/)).toBeOnTheScreen();
    expect(within(card).getByTestId("phone-update-status")).toHaveTextContent("尚未检查更新");
    expect(within(card).queryByTestId("phone-update-download")).toBeNull();
    await fireEvent.press(within(card).getByTestId("phone-update-check"));
    await waitFor(() => {
      expect(within(card).getByTestId("phone-update-status")).toHaveTextContent(
        `有新版本 ${MOCK_AVAILABLE_VERSION} · 当前 ${MOCK_CURRENT_VERSION}`,
      );
    });
    await fireEvent.press(within(card).getByTestId("phone-update-notes"));
    expect(await within(card).findByText(MOCK_UPDATE_NOTES)).toBeOnTheScreen();
    await fireEvent.press(within(card).getByTestId("phone-update-download"));
    expect(backend.updatePagesOpened).toEqual([MOCK_AVAILABLE_VERSION]);
    expect(within(card).getByText(/Android 只接受与当前版本签名相同的安装包/)).toBeOnTheScreen();
    // 自动检查更新 is the shared `auto_update` setting, off by default as on the desktop.
    expect(within(card).getByTestId("phone-update-auto")).not.toBeChecked();
    await fireEvent.press(within(card).getByTestId("phone-update-auto"));
    await waitFor(() => {
      expect(within(card).getByTestId("phone-update-auto")).toBeChecked();
    });
    expect(backend.peek().settings.auto_update).toBe(true);
  });

  it("the update line says where a check ended, with the desktop's lamp for it", async () => {
    const { backend } = await renderApp();
    const card = await openAbout();
    const status = () => within(card).getByTestId("phone-update-status");
    const lamp = (tone: Parameters<typeof toneColor>[1]) => {
      expect(within(card).getByTestId("phone-update-status-lamp")).toHaveStyle({
        backgroundColor: toneColor(appTheme("glass"), tone),
      });
    };
    lamp("idle");
    await act(async () => {
      backend.simulateUpdate({ state: "checking" });
    });
    expect(status()).toHaveTextContent("正在检查更新…");
    lamp("accent");
    expect(within(card).getByTestId("phone-update-check")).toBeDisabled();
    await act(async () => {
      backend.simulateUpdate({ state: "up_to_date", version: "0.0.1", checked_at: 1_790_000_000 });
    });
    expect(status()).toHaveTextContent(/^已是最新 · 0\.0\.1 · 检查于 /);
    lamp("ok");
    await act(async () => {
      backend.simulateUpdate({ state: "failed", message: "update: 无法连接 GitHub" });
    });
    // The machine prefix goes; the lamp turns the danger colour.
    expect(status()).toHaveTextContent("更新失败 · 无法连接 GitHub");
    lamp("danger");
  });

  it("an install from Google Play is updated by Play: the card opens the listing and checks nothing", async () => {
    const { backend } = await renderApp({
      mock: { update: { state: "store", version: MOCK_CURRENT_VERSION } },
    });
    const card = await openAbout();
    expect(within(card).getByText("由 Google Play 更新")).toBeOnTheScreen();
    expect(within(card).getByText(/Google Play 会自动更新它/)).toBeOnTheScreen();
    expect(within(card).queryByTestId("phone-update-check")).toBeNull();
    expect(within(card).queryByTestId("phone-update-auto")).toBeNull();
    await fireEvent.press(within(card).getByText("在 Google Play 中打开"));
    expect(backend.updatePagesOpened).toEqual(["store"]);
    expect(backend.peek().update).toEqual({ state: "store", version: MOCK_CURRENT_VERSION });
  });
});

describe("关于", () => {
  it("says a newer version during the download steps, and shows no update card in a build without updates", async () => {
    const { backend } = await renderApp();
    const card = await openAbout();
    for (const update of [
      { state: "downloading" as const, version: "9.9.9", received: 1 },
      { state: "ready" as const, version: "9.9.9" },
      { state: "installing" as const, version: "9.9.9" },
    ]) {
      await act(async () => {
        backend.simulateUpdate(update);
        await Promise.resolve();
      });
      expect(within(card).getByTestId("phone-update-status")).toHaveTextContent(
        `有新版本 9.9.9 · 当前 ${MOCK_CURRENT_VERSION}`,
      );
    }
    await act(async () => {
      backend.simulateUpdate({ state: "disabled" });
      await Promise.resolve();
    });
    await waitFor(() => {
      expect(screen.queryByTestId("phone-update")).toBeNull();
    });
  });

  it("opens the source and the releases", async () => {
    const { backend } = await renderApp();
    await openAbout();
    const page = within(screen.getByTestId("phone-about"));
    await fireEvent.press(page.getByText("源代码"));
    await fireEvent.press(page.getByText("发布版本"));
    await waitFor(() => {
      expect(backend.linksOpened).toEqual(["source", "releases"]);
    });
  });

  // One failure a test: each says so in a toast, and toasts wait their turn.
  it.each([
    ["opening the source", "源代码", "projectLinkOpen", new Error("没有浏览器"), /没有浏览器/],
    ["opening the privacy policy", "隐私政策", "guideOpen", "打不开", /打不开/],
    ["the update check", "检查更新", "invoke", new Error("检查失败"), /检查失败/],
  ] as const)("says why %s did not go through", async (_what, button, method, error, shown) => {
    const { backend } = await renderApp();
    await openAbout();
    jest.spyOn(backend, method).mockRejectedValueOnce(error);
    await fireEvent.press(within(screen.getByTestId("phone-about")).getByText(button));
    expect(await screen.findByText(shown)).toBeOnTheScreen();
  });
});
