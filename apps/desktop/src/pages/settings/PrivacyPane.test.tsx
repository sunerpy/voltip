import { type HistoryEntry, defaultEngineSettings } from "@voltip/shared";
import { MOCK_PUBLIC_KEYS, MockBackend, sampleHistory } from "@voltip/shared/mock";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../../test/render";
import { KEEP_OPTIONS } from "./PrivacyPane";

const NOW = 1_758_700_000_000;

describe("Settings · 隐私与历史", () => {
  it("regression: says the history and the settings go to the phones that sync, and stay here once none does", async () => {
    // 2026-10-02 (docs/dictation.md §20.8): 发送出去的内容 listed the audio, the text and the app
    // context only, while a phone with Sync on received the history and the settings.
    const { backend } = renderApp({ path: "/settings/privacy" });
    const value = await screen.findByTestId("privacy-history-sync");
    await waitFor(() => {
      expect(value).toHaveTextContent("同步到已配对的手机");
    });
    expect(
      screen.getByText(/开启「同步」的已配对手机会收到历史记录和设置，在手机上只能查看/),
    ).toBeInTheDocument();
    await backend.invoke("device_sync_set", { publicKey: MOCK_PUBLIC_KEYS.phone, on: false });
    await waitFor(() => {
      expect(value).toHaveTextContent("不离开本机");
    });
  });

  it("says where the audio, the text and the app context go, from the core's engines and switches", async () => {
    const { backend } = renderApp({ path: "/settings/privacy" });
    await screen.findByTestId("privacy-pane");
    // The built-in service by name, never its host.
    expect(screen.getByTestId("privacy-audio")).toHaveTextContent("发送到内置服务");
    expect(screen.getByTestId("privacy-text")).toHaveTextContent("发送到内置服务");
    expect(screen.getByTestId("privacy-context")).toHaveTextContent("应用名称");
    expect(screen.getByTestId("privacy-context")).not.toHaveTextContent("窗口标题");
    expect(screen.getByTestId("privacy-backend")).toHaveTextContent("Windows 凭据管理器");
    // On-device recognition with polish off: nothing leaves the computer.
    await backend.invoke("settings_set_engines", {
      engines: { ...defaultEngineSettings(), asr_provider: "local", refine_enabled: false },
    });
    await waitFor(() => {
      expect(screen.getByTestId("privacy-audio")).toHaveTextContent("不离开本机");
    });
    expect(screen.getByTestId("privacy-text")).toHaveTextContent("不发送");
    expect(screen.getByTestId("privacy-context")).toHaveTextContent("不发送");
    expect(document.body.textContent).not.toMatch(/示例|安全输入框/);
  });

  it("the history switch and retention write settings_set_history; a smaller retention trims at once", async () => {
    const user = userEvent.setup();
    // More entries than the smallest choice keeps (500), so choosing it trims.
    const rows = Array.from({ length: 600 }, (_, i) => ({
      ...(sampleHistory(NOW)[0] as HistoryEntry),
      id: `id-${i}`,
      at_ms: NOW - i * 60_000,
    }));
    const { backend } = renderApp({
      path: "/settings/privacy",
      backend: new MockBackend({ now: () => NOW, history: rows }),
    });
    const section = await screen.findByTestId("privacy-history");
    expect(within(section).getByTestId("history-count")).toHaveTextContent("600 / 20,000 条");
    const keep = within(section).getByLabelText("保留最近");
    expect(
      within(keep)
        .getAllByRole("option")
        .map((o) => o.textContent),
    ).toEqual(["500 条", "2,000 条", "5,000 条", "10,000 条", "20,000 条"]);
    expect(KEEP_OPTIONS).toEqual([500, 2000, 5000, 10_000, 20_000]);
    await user.click(within(section).getByRole("switch", { name: "保存听写历史" }));
    await waitFor(() => {
      expect(backend.peek().settings.history).toEqual({ enabled: false, keep: 20_000 });
    });
    await user.selectOptions(keep, "500");
    await waitFor(() => {
      expect(backend.peek().settings.history).toEqual({ enabled: false, keep: 500 });
    });
    expect(backend.peek().history_total).toBe(500);
    expect(within(section).getByTestId("history-count")).toHaveTextContent("500 / 500 条");
  });

  it("an install that saved another retention (plan 3.1: no settings migration) keeps it on offer", async () => {
    renderApp({
      path: "/settings/privacy",
      backend: new MockBackend({
        now: () => NOW,
        settings: { history: { enabled: true, keep: 200 } },
      }),
    });
    const keep = within(await screen.findByTestId("privacy-history")).getByLabelText("保留最近");
    expect(keep).toHaveValue("200");
    expect(
      within(keep)
        .getAllByRole("option")
        .map((o) => o.textContent),
    ).toEqual(["200 条", "500 条", "2,000 条", "5,000 条", "10,000 条", "20,000 条"]);
  });

  it("清空历史 asks first, then clears every entry", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({
      path: "/settings/privacy",
      backend: new MockBackend({ now: () => NOW, history: sampleHistory(NOW) }),
    });
    await screen.findByTestId("privacy-history");
    const n = backend.peek().history_recent.length;
    expect(n).toBeGreaterThan(0);
    await user.click(screen.getByRole("button", { name: "清空历史" }));
    const confirm = screen.getByRole("dialog", { name: "清空全部听写历史？" });
    expect(confirm).toHaveTextContent(`${n} 条记录会被删除`);
    await user.click(within(confirm).getByRole("button", { name: "清空" }));
    await waitFor(() => {
      expect(backend.peek().history_recent).toEqual([]);
    });
    expect(await screen.findByText("已清空听写历史")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "清空历史" })).toBeDisabled();
  });
});
