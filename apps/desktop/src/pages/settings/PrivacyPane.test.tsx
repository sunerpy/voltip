import { type HistoryEntry, type KeptRecording, defaultEngineSettings } from "@voltip/shared";
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

  it("清空历史 asks first, then clears every entry and the kept recordings", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({
      path: "/settings/privacy",
      backend: new MockBackend({
        now: () => NOW,
        history: sampleHistory(NOW),
        audioCache: [recording("r1", { kind: "cancelled" })],
      }),
    });
    await screen.findByTestId("privacy-history");
    const n = backend.peek().history_recent.length;
    expect(n).toBeGreaterThan(0);
    await user.click(screen.getByRole("button", { name: "清空历史" }));
    const confirm = screen.getByRole("dialog", { name: "清空全部听写历史？" });
    // docs/dictation.md §26: the recordings go with the history.
    expect(confirm).toHaveTextContent(`这台电脑上的 ${n} 条记录和保留的录音会被删除`);
    await user.click(within(confirm).getByRole("button", { name: "清空" }));
    await waitFor(() => {
      expect(backend.peek().history_recent).toEqual([]);
    });
    expect(backend.peek().audio_cache.recordings).toEqual([]);
    expect(await screen.findByText("已清空听写历史")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "清空历史" })).toBeDisabled();
  });
});

/** A kept recording (docs/dictation.md §26) of a 3-second take. */
function recording(id: string, status: KeptRecording["status"], bytes = 96_044): KeptRecording {
  return { id, at_ms: NOW - 60_000, duration_ms: 3000, bytes, status };
}

describe("Settings · 隐私与历史 · 录音缓存", () => {
  it("keeps the recordings by default and says how many there are, how much space they take and where they stay", async () => {
    renderApp({
      path: "/settings/privacy",
      backend: new MockBackend({
        now: () => NOW,
        audioCache: [
          recording("r1", { kind: "cancelled" }),
          recording("r2", { kind: "interrupted" }),
        ],
      }),
    });
    const section = await screen.findByTestId("privacy-recordings");
    expect(within(section).getByRole("switch", { name: "在本机保留录音" })).toBeChecked();
    expect(within(section).getByTestId("recordings-count")).toHaveTextContent("2 条 · 192 KB");
    expect(within(section).getByRole("combobox", { name: "最多保留" })).toHaveValue("20");
    expect(within(section).getByRole("combobox", { name: "最多占用" })).toHaveValue("200");
    expect(within(section).getByTestId("recordings-note")).toHaveTextContent(
      "录音只保存在这台电脑上，不同步到手机，也不包含在备份中。",
    );
  });

  it("changes how many recordings and how much space it keeps, and dropping the limit removes the oldest", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({
      path: "/settings/privacy",
      backend: new MockBackend({
        now: () => NOW,
        audioCache: Array.from({ length: 8 }, (_, i) => recording(`r${i}`, { kind: "cancelled" })),
      }),
    });
    const section = await screen.findByTestId("privacy-recordings");
    await user.selectOptions(within(section).getByRole("combobox", { name: "最多保留" }), "5");
    await waitFor(() => {
      expect(backend.peek().settings.audio_cache.keep).toBe(5);
    });
    // The newest five stay.
    expect(backend.peek().audio_cache.recordings.map((r) => r.id)).toEqual([
      "r0",
      "r1",
      "r2",
      "r3",
      "r4",
    ]);
    await user.selectOptions(within(section).getByRole("combobox", { name: "最多占用" }), "1000");
    await waitFor(() => {
      expect(backend.peek().settings.audio_cache.max_mb).toBe(1000);
    });
    expect(within(section).getByRole("combobox", { name: "最多占用" })).toHaveValue("1000");
  });

  it("turning it off deletes the kept recordings and disables the limits", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({
      path: "/settings/privacy",
      backend: new MockBackend({
        now: () => NOW,
        audioCache: [recording("r1", { kind: "cancelled" })],
      }),
    });
    const section = await screen.findByTestId("privacy-recordings");
    await user.click(within(section).getByRole("switch", { name: "在本机保留录音" }));
    await waitFor(() => {
      expect(backend.peek().settings.audio_cache.enabled).toBe(false);
    });
    expect(backend.peek().audio_cache.recordings).toEqual([]);
    expect(within(section).getByRole("combobox", { name: "最多保留" })).toBeDisabled();
    expect(within(section).getByRole("combobox", { name: "最多占用" })).toBeDisabled();
    expect(within(section).getByRole("button", { name: "清理录音" })).toBeDisabled();
  });

  it("清理录音 asks first, then deletes every kept recording and leaves the history's text", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({
      path: "/settings/privacy",
      backend: new MockBackend({
        now: () => NOW,
        history: sampleHistory(NOW),
        audioCache: [
          recording("r1", { kind: "cancelled" }),
          recording("r2", { kind: "interrupted" }),
        ],
      }),
    });
    const section = await screen.findByTestId("privacy-recordings");
    const entries = backend.peek().history_recent.length;
    await user.click(within(section).getByRole("button", { name: "清理录音" }));
    const confirm = screen.getByRole("dialog", { name: "删除全部保留的录音？" });
    expect(confirm).toHaveTextContent("2 条录音（192 KB）会被删除");
    await user.click(within(confirm).getByRole("button", { name: "删除" }));
    await waitFor(() => {
      expect(backend.peek().audio_cache.recordings).toEqual([]);
    });
    expect(await screen.findByText("已删除保留的录音")).toBeInTheDocument();
    expect(backend.peek().history_recent).toHaveLength(entries);
    expect(within(section).getByRole("button", { name: "清理录音" })).toBeDisabled();
  });

  it("needs the history: with the history off nothing is kept and the switch says why", async () => {
    renderApp({
      path: "/settings/privacy",
      backend: new MockBackend({
        now: () => NOW,
        settings: { history: { enabled: false, keep: 200 } },
      }),
    });
    const section = await screen.findByTestId("privacy-recordings");
    expect(within(section).getByRole("switch", { name: "在本机保留录音" })).toBeDisabled();
    expect(section).toHaveTextContent("需要先开启「保存听写历史」。");
    expect(within(section).getByRole("combobox", { name: "最多保留" })).toBeDisabled();
  });

  it("is not there on a device that keeps no recordings", async () => {
    renderApp({
      path: "/settings/privacy",
      backend: new MockBackend({ now: () => NOW, role: "phone" }),
    });
    await screen.findByTestId("privacy-pane");
    expect(screen.queryByTestId("privacy-recordings")).toBeNull();
  });
});
