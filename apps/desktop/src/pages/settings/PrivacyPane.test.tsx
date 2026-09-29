import { defaultEngineSettings } from "@voltip/shared";
import { MockBackend, sampleHistory } from "@voltip/shared/mock";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../../test/render";
import { KEEP_OPTIONS } from "./PrivacyPane";

const NOW = 1_758_700_000_000;

describe("Settings · 隐私与历史", () => {
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
      expect(screen.getByTestId("privacy-audio")).toHaveTextContent("不离开这台电脑");
    });
    expect(screen.getByTestId("privacy-text")).toHaveTextContent("不发送");
    expect(screen.getByTestId("privacy-context")).toHaveTextContent("不发送");
    expect(document.body.textContent).not.toMatch(/示例|安全输入框/);
  });

  it("the history switch and retention write settings_set_history; a smaller retention trims at once", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({
      path: "/settings/privacy",
      backend: new MockBackend({ now: () => NOW, history: sampleHistory(NOW) }),
    });
    const section = await screen.findByTestId("privacy-history");
    const total = backend.peek().history.length;
    expect(within(section).getByTestId("history-count")).toHaveTextContent(`${total} / 500 条`);
    const keep = within(section).getByLabelText("保留最近");
    expect(
      within(keep)
        .getAllByRole("option")
        .map((o) => o.textContent),
    ).toEqual(KEEP_OPTIONS.map((n) => `${n} 条`));
    await user.click(within(section).getByRole("switch", { name: "保存听写历史" }));
    await waitFor(() => {
      expect(backend.peek().settings.history).toEqual({ enabled: false, keep: 500 });
    });
    await user.selectOptions(keep, "50");
    await waitFor(() => {
      expect(backend.peek().settings.history).toEqual({ enabled: false, keep: 50 });
    });
    expect(backend.peek().history).toHaveLength(Math.min(total, 50));
  });

  it("清空历史 asks first, then clears every entry", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({
      path: "/settings/privacy",
      backend: new MockBackend({ now: () => NOW, history: sampleHistory(NOW) }),
    });
    await screen.findByTestId("privacy-history");
    const n = backend.peek().history.length;
    expect(n).toBeGreaterThan(0);
    await user.click(screen.getByRole("button", { name: "清空历史" }));
    const confirm = screen.getByRole("dialog", { name: "清空全部听写历史？" });
    expect(confirm).toHaveTextContent(`${n} 条记录会被删除`);
    await user.click(within(confirm).getByRole("button", { name: "清空" }));
    await waitFor(() => {
      expect(backend.peek().history).toEqual([]);
    });
    expect(await screen.findByText("已清空听写历史")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "清空历史" })).toBeDisabled();
  });
});
