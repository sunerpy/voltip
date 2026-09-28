import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MOCK_FEEDBACK_MS, MockBackend, sampleDevices } from "@voltip/shared/mock";
import { zhT } from "@voltip/shared";
import { renderApp } from "../test/render";
import { diagnosticValue, feedbackError } from "./Feedback";

function backend(options: ConstructorParameters<typeof MockBackend>[0] = {}) {
  return new MockBackend({
    devices: sampleDevices(1_758_700_000),
    now: () => 1_758_700_000_000,
    ...options,
  });
}

/** 反馈 from the sidebar: a page of the main layout since 2026-09-28, no longer a dialog. */
async function openFeedback() {
  const user = userEvent.setup({ advanceTimers: (ms) => vi.advanceTimersByTime(ms) });
  await screen.findByRole("heading", { name: "首页", level: 1 });
  await user.click(screen.getByTestId("sidebar-feedback"));
  const page = await screen.findByTestId("page-feedback");
  return { user, page };
}

describe("the 反馈 page", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("regression: 反馈 shows exactly what goes along, sends the report through the shell and says so", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const core = backend();
    renderApp({ backend: core });
    const { user, page } = await openFeedback();
    const attached = within(page).getByTestId("feedback-attached");
    await waitFor(() => {
      expect(within(attached).queryByText("正在读取要附带的信息…")).toBeNull();
    });
    // The facts the report carries, worded, and nothing that names a host.
    expect(attached.querySelector('[data-diagnostic="app_version"]')).toHaveTextContent("0.0.1");
    expect(attached.querySelector('[data-diagnostic="os"]')).toHaveTextContent("Windows");
    expect(attached.querySelector('[data-diagnostic="locale"]')).toHaveTextContent("zh-CN");
    expect(attached.querySelector('[data-diagnostic="asr_provider"]')).toHaveTextContent(
      "内置服务",
    );
    expect(attached.querySelector('[data-diagnostic="llm_provider"]')).toHaveTextContent(
      "内置服务",
    );
    expect(attached.querySelector('[data-diagnostic="output_mode"]')).toHaveTextContent("整段输出");
    expect(attached.querySelector('[data-diagnostic="local_model"]')).toBeNull();
    expect(attached).toHaveTextContent("不含主机名、密钥和听写内容");
    expect(page.textContent).not.toMatch(/https?:|example|\.app\b/);
    // Nothing to send until there are words.
    const send = within(page).getByTestId("feedback-send");
    expect(send).toBeDisabled();
    await user.click(within(page).getByRole("radio", { name: "建议" }));
    await user.type(within(page).getByRole("textbox", { name: "描述" }), "希望支持鼠标侧键说话");
    expect(within(page).getByTestId("feedback-count")).toHaveTextContent("10 / 5000");
    await user.type(
      within(page).getByRole("textbox", { name: "联系方式（可选）" }),
      " me@example.test ",
    );
    await user.click(send);
    expect(send).toHaveTextContent("发送中…");
    act(() => {
      vi.advanceTimersByTime(MOCK_FEEDBACK_MS);
    });
    expect(await screen.findByText("反馈已发送，谢谢")).toBeInTheDocument();
    // The page stays, with an empty form ready for the next report.
    expect(within(page).getByRole("textbox", { name: "描述" })).toHaveValue("");
    expect(within(page).getByRole("textbox", { name: "联系方式（可选）" })).toHaveValue("");
    expect(screen.getByTestId("sidebar-feedback")).toHaveAttribute("aria-current", "page");
    expect(screen.getAllByRole("heading", { name: "反馈", level: 1 }).length).toBeGreaterThan(0);
    expect(core.feedbackSent).toEqual([
      {
        kind: "idea",
        message: "希望支持鼠标侧键说话",
        contact: "me@example.test",
        locale: "zh-CN",
      },
    ]);
    // It no longer opens the repository page: the report goes to the feedback endpoint.
    expect(core.linksOpened).toEqual([]);
  });

  it("regression: a failed submission says why and keeps what was written", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const core = backend({ feedback: "rate_limited" });
    renderApp({ backend: core });
    const { user, page } = await openFeedback();
    await user.type(within(page).getByRole("textbox", { name: "描述" }), "粘贴没有生效");
    await user.click(within(page).getByTestId("feedback-send"));
    act(() => {
      vi.advanceTimersByTime(MOCK_FEEDBACK_MS);
    });
    expect(await within(page).findByRole("alert")).toHaveTextContent("发送太频繁了，请稍后再试。");
    expect(within(page).getByRole("textbox", { name: "描述" })).toHaveValue("粘贴没有生效");
    expect(within(page).getByTestId("feedback-send")).toBeEnabled();
    expect(core.feedbackSent).toEqual([]);
  });

  it("a build without a feedback address offers the repository's issue page instead", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const core = backend({ feedback: "not_configured" });
    renderApp({ backend: core });
    const { user, page } = await openFeedback();
    expect(await within(page).findByTestId("feedback-not-configured")).toHaveTextContent(
      "这个构建没有配置反馈地址",
    );
    expect(within(page).queryByTestId("feedback-send")).toBeNull();
    await user.click(within(page).getByRole("button", { name: "在 GitHub 上反馈" }));
    expect(core.linksOpened).toEqual(["feedback"]);
  });

  it("words the diagnostics and maps the shell's refusals", () => {
    const { t } = zhT;
    expect(diagnosticValue("os", "linux", t, "zh-CN")).toBe("Linux");
    expect(diagnosticValue("os", "haiku", t, "zh-CN")).toBe("haiku");
    expect(diagnosticValue("asr_provider", "local", t, "zh-CN")).toBe("本机");
    expect(diagnosticValue("llm_provider", "someone", t, "zh-CN")).toBe("someone");
    expect(diagnosticValue("compute", "gpu", t, "zh-CN")).toBe("GPU");
    expect(diagnosticValue("compute", "npu", t, "zh-CN")).toBe("npu");
    expect(diagnosticValue("output_mode", "live_inject", t, "zh-CN")).toBe("实时注入");
    expect(diagnosticValue("output_mode", "odd", t, "zh-CN")).toBe("odd");
    expect(diagnosticValue("arch", "aarch64", t, "zh-CN")).toBe("aarch64");
    expect(feedbackError(new Error("timeout"))).toBe("timeout");
    expect(feedbackError("rate_limited")).toBe("rate_limited");
    expect(feedbackError(new Error("feedback: 请在电脑上反馈"))).toBe("server");
  });
});
