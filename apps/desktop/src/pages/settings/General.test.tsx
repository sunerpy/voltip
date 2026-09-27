import { createTranslator, zhT } from "@voltip/shared";
import {
  MOCK_AVAILABLE_VERSION,
  MOCK_UPDATE_CHECK_MS,
  MOCK_UPDATE_TICK_MS,
  MOCK_UPDATE_TICKS,
  MockBackend,
} from "@voltip/shared/mock";
import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../../test/render";
import { downloadProgress, updateStatusLine } from "./General";

describe("Settings · 通用", () => {
  it("regression: the first-run guide can be run again from here", async () => {
    const user = userEvent.setup();
    renderApp({ path: "/settings/general" });
    const dialog = await screen.findByRole("dialog", { name: "设置" });
    expect(within(dialog).getByText("首次设置")).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "重新运行" }));
    expect(await screen.findByRole("heading", { name: "系统权限", level: 2 })).toBeInTheDocument();
    expect(screen.queryByRole("dialog", { name: "设置" })).not.toBeInTheDocument();
  });

  it("regression: the language segmented control writes settings_set_locale for every option", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/settings/general" });
    const dialog = await screen.findByRole("dialog", { name: "设置" });
    expect(within(dialog).getByRole("heading", { name: "通用", level: 2 })).toBeInTheDocument();
    const before = backend.log.length;
    await user.click(screen.getByRole("radio", { name: "English" }));
    await waitFor(() => {
      expect(backend.peek().settings.locale).toBe("en");
    });
    await user.click(screen.getByRole("radio", { name: "简体中文" }));
    await waitFor(() => {
      expect(backend.peek().settings.locale).toBe("zh-cn");
    });
    await user.click(screen.getByRole("radio", { name: "跟随系统" }));
    await waitFor(() => {
      expect(backend.peek().settings.locale).toBe("system");
    });
    // Each click is one settings event from the core; nothing else fires.
    const events = backend.log.slice(before);
    expect(events.map((e) => e.type)).toEqual(["settings", "settings", "settings"]);
    expect(events.map((e) => (e.type === "settings" ? e.locale : ""))).toEqual([
      "en",
      "zh-cn",
      "system",
    ]);
  });

  it("regression: the auto-update toggle writes settings_set_auto_update and reflects the core", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/settings/general" });
    await screen.findByRole("dialog", { name: "设置" });
    const toggle = screen.getByRole("switch", { name: "自动更新" });
    expect(toggle).toHaveAttribute("aria-checked", "false");
    await user.click(toggle);
    await waitFor(() => {
      expect(backend.peek().settings.auto_update).toBe(true);
    });
    expect(screen.getByRole("switch", { name: "自动更新" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    await user.click(screen.getByRole("switch", { name: "自动更新" }));
    await waitFor(() => {
      expect(backend.peek().settings.auto_update).toBe(false);
    });
    // A core that already has it on renders it on.
    const { unmount } = renderApp({
      path: "/settings/general",
      backend: new MockBackend({ settings: { auto_update: true } }),
    });
    expect((await screen.findAllByRole("switch", { name: "自动更新" })).at(-1)).toHaveAttribute(
      "aria-checked",
      "true",
    );
    unmount();
  });

  it("regression: 检查更新 → 立即更新 → downloading → 重启并更新 → installing renders every updater state", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    try {
      const { backend } = renderApp({ path: "/settings/general" });
      await screen.findByRole("dialog", { name: "设置" });
      const status = () => screen.getByTestId("update-status");
      expect(status()).toHaveTextContent("尚未检查更新");
      await user.click(screen.getByRole("button", { name: "检查更新" }));
      await waitFor(() => {
        expect(backend.peek().update.state).toBe("checking");
      });
      expect(status()).toHaveTextContent("正在检查更新…");
      expect(screen.getByRole("button", { name: "检查中…" })).toBeDisabled();
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_UPDATE_CHECK_MS);
      });
      expect(status()).toHaveTextContent(`有新版本 ${MOCK_AVAILABLE_VERSION} · 当前 0.0.1`);
      expect(screen.queryByRole("button", { name: "检查更新" })).toBeNull();
      await user.click(screen.getByRole("button", { name: "立即更新" }));
      await waitFor(() => {
        expect(backend.peek().update.state).toBe("downloading");
      });
      expect(status()).toHaveTextContent(`正在下载 ${MOCK_AVAILABLE_VERSION} · 33%`);
      expect(screen.queryByRole("button", { name: "立即更新" })).toBeNull();
      await act(async () => {
        await vi.advanceTimersByTimeAsync(MOCK_UPDATE_TICK_MS * MOCK_UPDATE_TICKS);
      });
      expect(backend.peek().update.state).toBe("ready");
      expect(status()).toHaveTextContent(`${MOCK_AVAILABLE_VERSION} 已下载 · 重启后生效`);
      await user.click(screen.getByRole("button", { name: "重启并更新" }));
      await waitFor(() => {
        expect(backend.peek().update.state).toBe("installing");
      });
      expect(status()).toHaveTextContent(`正在安装 ${MOCK_AVAILABLE_VERSION}…`);
      expect(screen.getByRole("button", { name: "检查更新" })).toBeDisabled();
    } finally {
      vi.useRealTimers();
    }
  });

  it("renders the up-to-date, failed and disabled states, and 关于 shows the same status line", async () => {
    const { backend } = renderApp({
      path: "/settings/general",
      backend: new MockBackend({ update: { state: "disabled" } }),
    });
    await screen.findByRole("dialog", { name: "设置" });
    expect(screen.getByTestId("update-status")).toHaveTextContent("此构建未配置更新源");
    expect(screen.getByRole("button", { name: "检查更新" })).toBeDisabled();
    act(() => {
      backend.simulateUpdate({ state: "failed", message: "offline" });
    });
    expect(screen.getByTestId("update-status")).toHaveTextContent("更新失败 · offline");
    expect(screen.getByRole("button", { name: "检查更新" })).toBeEnabled();
    act(() => {
      backend.simulateUpdate({
        state: "up_to_date",
        version: "0.0.1",
        checked_at: 1_758_700_000,
      });
    });
    expect(screen.getByTestId("update-status")).toHaveTextContent(/已是最新 · 0\.0\.1 · 检查于 /);
    // The About pane shows the same line in its update row.
    const user = userEvent.setup();
    await user.click(screen.getByRole("tab", { name: /关于/ }));
    expect(screen.getByTestId("update-status")).toHaveTextContent(/已是最新 · 0\.0\.1/);
    expect(screen.getByRole("button", { name: "检查更新" })).toBeInTheDocument();
  });

  it("updateStatusLine and downloadProgress cover every state in both locales", () => {
    const en = createTranslator("en").t;
    const at = () => "2026-09-25";
    expect(downloadProgress(12, 48)).toBe("25%");
    expect(downloadProgress(1_048_576 * 2.5, undefined)).toBe("2.5 MB");
    expect(downloadProgress(5, 0)).toBe("0.0 MB");
    expect(updateStatusLine({ state: "idle" }, zhT.t, at)).toEqual({
      text: "尚未检查更新",
      tone: "idle",
    });
    expect(updateStatusLine({ state: "checking" }, en, at).text).toBe("Checking for updates…");
    expect(
      updateStatusLine({ state: "up_to_date", version: "1", checked_at: 0 }, en, at).text,
    ).toBe("Up to date · 1 · checked 2026-09-25");
    expect(updateStatusLine({ state: "available", version: "2", current: "1" }, en, at).text).toBe(
      "Version 2 available · current 1",
    );
    expect(
      updateStatusLine({ state: "downloading", version: "2", received: 5, total: 10 }, en, at).text,
    ).toBe("Downloading 2 · 50%");
    expect(updateStatusLine({ state: "ready", version: "2" }, en, at)).toEqual({
      text: "2 downloaded · applies after a restart",
      tone: "ok",
    });
    expect(updateStatusLine({ state: "installing", version: "2" }, en, at).text).toBe(
      "Installing 2…",
    );
    expect(updateStatusLine({ state: "failed", message: "x" }, en, at)).toEqual({
      text: "Update failed · x",
      tone: "danger",
    });
    expect(updateStatusLine({ state: "disabled" }, en, at).text).toBe(
      "This build has no update source",
    );
  });
});
