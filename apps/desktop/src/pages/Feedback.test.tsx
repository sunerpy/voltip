import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MOCK_FEEDBACK_MS, MockBackend, sampleDevices } from "@voltip/shared/mock";
import { FEEDBACK_MAX_VIDEO_BYTES, zhT } from "@voltip/shared";
import { renderApp } from "../test/render";
import {
  attachmentError,
  attachmentType,
  diagnosticValue,
  feedbackError,
  precheckAttachment,
} from "./Feedback";

/** A file of `size` bytes; the bytes are only read when the page stages it. */
function file(name: string, type: string, size = 4): File {
  return new File([new Uint8Array(size)], name, { type });
}

function backend(options: ConstructorParameters<typeof MockBackend>[0] = {}) {
  return new MockBackend({
    devices: sampleDevices(1_758_700_000),
    now: () => 1_758_700_000_000,
    ...options,
  });
}

/** 反馈 from the sidebar: a dialog over the page it was opened from (user decision 2026-09-28). */
async function openFeedback() {
  const user = userEvent.setup({ advanceTimers: (ms) => vi.advanceTimersByTime(ms) });
  await screen.findByRole("heading", { name: "首页", level: 1 });
  await user.click(screen.getByTestId("sidebar-feedback"));
  const page = await screen.findByRole("dialog", { name: "反馈" });
  return { user, page };
}

describe("the 反馈 dialog", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("regression: 反馈 opens over the page it came from, shows exactly what goes along, sends the report through the shell and says so", async () => {
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
    // Sent: the dialog closes back to the page beneath, and the next report starts empty.
    await waitFor(() => {
      expect(screen.queryByRole("dialog", { name: "反馈" })).toBeNull();
    });
    expect(screen.getByRole("heading", { name: "首页", level: 1 })).toBeInTheDocument();
    await user.click(screen.getByTestId("sidebar-feedback"));
    const again = await screen.findByRole("dialog", { name: "反馈" });
    expect(within(again).getByRole("textbox", { name: "描述" })).toHaveValue("");
    expect(within(again).getByRole("textbox", { name: "联系方式（可选）" })).toHaveValue("");
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

  it("regression: screenshots and recordings go along with the report, picked or pasted, and each can be removed (user feedback 2026-09-28)", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const core = backend();
    renderApp({ backend: core });
    const { user, page } = await openFeedback();
    const input = within(page).getByTestId("feedback-attach-input");
    expect(input).toHaveAttribute("accept", expect.stringContaining("video/quicktime"));
    expect(within(page).getByTestId("feedback-attachments")).toHaveTextContent(
      "最多 3 个：图片（PNG、JPEG、GIF、WebP）不超过 5 MB，视频（MP4、WebM、MOV）不超过 20 MB，合计不超过 25 MB。也可以直接粘贴截图。",
    );
    await user.upload(input, [
      file("设置页.png", "image/png", 2048),
      file("录屏.mp4", "video/mp4"),
    ]);
    await waitFor(() => {
      expect(within(page).getAllByTestId("feedback-attachment")).toHaveLength(2);
    });
    const rows = within(page).getAllByTestId("feedback-attachment");
    expect(rows[0]).toHaveTextContent("设置页.png");
    expect(rows[0]).toHaveTextContent("2 KB");
    expect(within(rows[0] as HTMLElement).getByRole("img", { name: "图片" })).toBeInTheDocument();
    expect(within(rows[1] as HTMLElement).getByRole("img", { name: "视频" })).toBeInTheDocument();
    // A pasted screenshot is staged like a picked one.
    fireEvent.paste(within(page).getByRole("textbox", { name: "描述" }), {
      clipboardData: { files: [file("image.png", "image/png")] },
    });
    await waitFor(() => {
      expect(within(page).getAllByTestId("feedback-attachment")).toHaveLength(3);
    });
    expect(within(page).getByTestId("feedback-attach")).toBeDisabled();
    await user.click(within(page).getByRole("button", { name: "移除 录屏.mp4" }));
    expect(within(page).getAllByTestId("feedback-attachment")).toHaveLength(2);
    expect(core.feedbackStaged.map((a) => a.name)).toEqual(["设置页.png", "image.png"]);
    await user.type(within(page).getByRole("textbox", { name: "描述" }), "设置页错位");
    await user.click(within(page).getByTestId("feedback-send"));
    act(() => {
      vi.advanceTimersByTime(MOCK_FEEDBACK_MS);
    });
    expect(await screen.findByText("反馈已发送，谢谢")).toBeInTheDocument();
    expect(core.feedbackSent).toEqual([
      {
        kind: "bug",
        message: "设置页错位",
        contact: null,
        locale: "zh-CN",
        attachments: ["attachment-1", "attachment-3"],
      },
    ]);
    // Sent: the dialog closed, and the next one starts without the files.
    await waitFor(() => {
      expect(screen.queryByRole("dialog", { name: "反馈" })).toBeNull();
    });
    expect(core.feedbackStaged).toEqual([]);
    await user.click(screen.getByTestId("sidebar-feedback"));
    const next = await screen.findByRole("dialog", { name: "反馈" });
    expect(within(next).queryByTestId("feedback-attachment")).toBeNull();
  });

  it("regression: a file over the limit is refused before it is read, and a wrong type says what is taken", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const core = backend();
    const add = vi.spyOn(core, "feedbackAttachmentAdd");
    renderApp({ backend: core });
    const { user, page } = await openFeedback();
    const input = within(page).getByTestId("feedback-attach-input");
    const big = file("会议.mov", "video/quicktime", 8);
    Object.defineProperty(big, "size", { value: FEEDBACK_MAX_VIDEO_BYTES + 1 });
    const read = vi.spyOn(big, "arrayBuffer");
    await user.upload(input, big);
    expect(await within(page).findByTestId("feedback-attach-error")).toHaveTextContent(
      "「会议.mov」不能附上：图片不超过 5 MB，视频不超过 20 MB，也不能是空文件。",
    );
    expect(read).not.toHaveBeenCalled();
    expect(add).not.toHaveBeenCalled();
    // `accept` is a hint the picker may ignore; a document is still turned away.
    const lax = userEvent.setup({
      applyAccept: false,
      advanceTimers: (ms) => vi.advanceTimersByTime(ms),
    });
    await lax.upload(input, file("日志.txt", "text/plain"));
    expect(within(page).getByTestId("feedback-attach-error")).toHaveTextContent(
      "「日志.txt」不能附上：只支持 PNG、JPEG、GIF、WebP 图片和 MP4、WebM、MOV 视频。",
    );
    await user.upload(input, file("ok.webp", "image/webp"));
    await waitFor(() => {
      expect(core.feedbackStaged).toHaveLength(1);
    });
    expect(within(page).queryByTestId("feedback-attach-error")).toBeNull();
  });

  it("regression: closing the dialog keeps the draft and its files until they are sent or cleared, and the window starts with nothing staged (user decision 2026-09-28)", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const core = backend();
    // A file a previous life of the window staged: the window starts by dropping it.
    await core.feedbackAttachmentAdd({
      name: "stale.png",
      type: "image/png",
      bytes: new Uint8Array(1),
    });
    renderApp({ backend: core });
    const { user, page } = await openFeedback();
    await waitFor(() => {
      expect(core.feedbackStaged).toEqual([]);
    });
    // The description has the focus: typing starts the report.
    expect(within(page).getByRole("textbox", { name: "描述" })).toHaveFocus();
    await user.click(within(page).getByRole("radio", { name: "建议" }));
    await user.type(within(page).getByRole("textbox", { name: "描述" }), "希望支持侧键");
    await user.upload(
      within(page).getByTestId("feedback-attach-input"),
      file("a.png", "image/png"),
    );
    await waitFor(() => {
      expect(within(page).getAllByTestId("feedback-attachment")).toHaveLength(1);
    });
    // Esc closes; the scrim closes too; neither loses a thing.
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: "反馈" })).toBeNull();
    expect(core.feedbackStaged).toHaveLength(1);
    await user.click(screen.getByTestId("sidebar-feedback"));
    let reopened = await screen.findByRole("dialog", { name: "反馈" });
    expect(within(reopened).getByRole("textbox", { name: "描述" })).toHaveValue("希望支持侧键");
    expect(within(reopened).getByRole("radio", { name: "建议" })).toBeChecked();
    expect(within(reopened).getAllByTestId("feedback-attachment")).toHaveLength(1);
    await user.click(screen.getByTestId("feedback-scrim"));
    expect(screen.queryByRole("dialog", { name: "反馈" })).toBeNull();
    // Another page in between changes nothing either.
    await user.click(within(screen.getByRole("navigation", { name: "主导航" })).getByText("词典"));
    await user.click(screen.getByTestId("sidebar-feedback"));
    reopened = await screen.findByRole("dialog", { name: "反馈" });
    expect(screen.getByRole("heading", { name: "词典", level: 1 })).toBeInTheDocument();
    expect(within(reopened).getByRole("textbox", { name: "描述" })).toHaveValue("希望支持侧键");
    // 清空 starts over, the staged file included.
    await user.click(within(reopened).getByTestId("feedback-discard"));
    expect(within(reopened).getByRole("textbox", { name: "描述" })).toHaveValue("");
    expect(within(reopened).getByRole("radio", { name: "问题" })).toBeChecked();
    expect(within(reopened).queryByTestId("feedback-attachment")).toBeNull();
    await waitFor(() => {
      expect(core.feedbackStaged).toEqual([]);
    });
    expect(within(reopened).getByTestId("feedback-discard")).toBeDisabled();
    expect(core.feedbackSent).toEqual([]);
  });

  it("regression: when the report went out but a file did not follow, the dialog says so and does not offer to send it twice", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const core = backend({ feedback: "attachments" });
    renderApp({ backend: core });
    const { user, page } = await openFeedback();
    await user.upload(
      within(page).getByTestId("feedback-attach-input"),
      file("a.gif", "image/gif"),
    );
    await waitFor(() => {
      expect(within(page).getAllByTestId("feedback-attachment")).toHaveLength(1);
    });
    await user.type(within(page).getByRole("textbox", { name: "描述" }), "动图里是问题");
    await user.click(within(page).getByTestId("feedback-send"));
    act(() => {
      vi.advanceTimersByTime(MOCK_FEEDBACK_MS);
    });
    expect(await within(page).findByTestId("feedback-error")).toHaveTextContent(
      "反馈已发送，但附件没有传完整。",
    );
    expect(within(page).getByRole("textbox", { name: "描述" })).toHaveValue("");
    expect(within(page).queryByTestId("feedback-attachment")).toBeNull();
    expect(core.feedbackSent).toHaveLength(1);
  });

  it("types a file without one by its extension and checks the limits before reading it", () => {
    expect(attachmentType({ name: "a.PNG", type: "" })).toBe("image/png");
    expect(attachmentType({ name: "clip.mov", type: "" })).toBe("video/quicktime");
    expect(attachmentType({ name: "noext", type: "" })).toBe("");
    expect(attachmentType({ name: "a.png", type: "image/x-icon" })).toBe("image/x-icon");
    const staged = [{ id: "1", name: "a.mp4", type: "video/mp4", size: FEEDBACK_MAX_VIDEO_BYTES }];
    expect(precheckAttachment("image/png", 1, [])).toBeUndefined();
    expect(precheckAttachment("image/heic", 1, [])).toBe("attachment_type");
    expect(precheckAttachment("video/webm", FEEDBACK_MAX_VIDEO_BYTES, staged)).toBe(
      "attachment_total",
    );
    expect(precheckAttachment("image/png", 1, [...staged, ...staged, ...staged])).toBe(
      "attachment_too_many",
    );
    expect(attachmentError(new Error("attachment_total"))).toBe("attachment_total");
    expect(attachmentError(new Error("feedback: 请在电脑上反馈"))).toBe("attachment_type");
    expect(feedbackError(new Error("storage_full"))).toBe("storage_full");
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
