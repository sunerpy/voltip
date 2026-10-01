import { FEEDBACK_MAX_IMAGE_BYTES } from "@voltip/shared";
import { MockBackend } from "@voltip/shared/mock";
import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../test/render";

// 反馈 on the phone (docs/feedback.md; user decision 2026-10-01: the phone sends feedback of its
// own, it pointed to the computer before).
function png(name: string, size = 3): File {
  return new File([new Uint8Array(size)], name, { type: "image/png" });
}

async function openFeedback(user: ReturnType<typeof userEvent.setup>) {
  await user.click(await screen.findByTestId("settings-feedback"));
  return screen.findByTestId("phone-feedback");
}

describe("the phone's feedback", () => {
  it("shows what goes along, stages a screenshot and sends the report", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ role: "phone" });
    renderApp({ backend, initialScreen: "settings" });
    const page = await openFeedback(user);
    expect(screen.getByRole("heading", { name: "反馈", level: 1 })).toBeInTheDocument();
    const attached = within(page).getByTestId("feedback-attached");
    await waitFor(() => {
      expect(within(attached).getByText("Android")).toBeInTheDocument();
    });
    expect(attached.querySelector('[data-diagnostic="session"]')).toBeNull();
    const send = within(page).getByTestId("feedback-send");
    expect(send).toBeDisabled();

    await user.click(within(page).getByRole("radio", { name: "建议" }));
    await user.type(within(page).getByLabelText("描述"), "希望能在手机上导出全部记录");
    expect(within(page).getByTestId("feedback-count")).toHaveTextContent("13 / 5000");
    await user.type(within(page).getByLabelText("联系方式（可选）"), "  me@example.test  ");
    await user.upload(within(page).getByTestId("feedback-attach-input"), [
      png("截图.png"),
      png("b.png"),
    ]);
    expect(await within(page).findAllByTestId("feedback-attachment")).toHaveLength(2);
    await user.click(within(page).getByRole("button", { name: "移除 b.png" }));
    expect(within(page).getAllByTestId("feedback-attachment")).toHaveLength(1);

    await user.click(send);
    await waitFor(() => {
      expect(backend.feedbackSent).toHaveLength(1);
    });
    expect(backend.feedbackSent[0]).toMatchObject({
      kind: "idea",
      message: "希望能在手机上导出全部记录",
      contact: "me@example.test",
      locale: "zh-CN",
    });
    expect(backend.feedbackSent[0]?.attachments).toHaveLength(1);
    expect(await screen.findByText("反馈已发送，谢谢")).toBeInTheDocument();
    expect(await screen.findByTestId("phone-settings")).toBeInTheDocument();
    // Leaving the page takes its files along.
    expect(backend.feedbackStaged).toEqual([]);
    backend.destroy();
  });

  it("words a refused file and a failed report, and starts over when only a file did not follow", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ role: "phone", feedback: "attachments" });
    renderApp({ backend, initialScreen: "settings" });
    const page = await openFeedback(user);
    const input = within(page).getByTestId("feedback-attach-input");
    await user.upload(input, png("big.png", FEEDBACK_MAX_IMAGE_BYTES + 1));
    expect(await within(page).findByTestId("feedback-attach-error")).toHaveTextContent(
      "「big.png」不能附上",
    );
    vi.spyOn(backend, "feedbackAttachmentAdd").mockRejectedValueOnce(new Error("attachment_name"));
    await user.upload(input, png("x.png"));
    expect(await within(page).findByTestId("feedback-attach-error")).toHaveTextContent(
      "文件名无法使用",
    );
    await user.upload(input, png("ok.png"));
    expect(await within(page).findAllByTestId("feedback-attachment")).toHaveLength(1);
    await user.type(within(page).getByLabelText("描述"), "录音中断");
    await user.click(within(page).getByTestId("feedback-send"));
    expect(await within(page).findByTestId("feedback-error")).toHaveTextContent(
      "反馈已发送，但附件没有传完整。",
    );
    // The report went out: sending again would send it twice.
    expect(within(page).getByLabelText("描述")).toHaveValue("");
    expect(within(page).queryAllByTestId("feedback-attachment")).toHaveLength(0);
    backend.destroy();

    const failing = new MockBackend({ role: "phone", feedback: "network" });
    renderApp({ backend: failing, initialScreen: "settings" });
    const second = (await screen.findAllByTestId("phone-settings")).at(-1) as HTMLElement;
    await user.click(within(second).getByTestId("settings-feedback"));
    const page2 = (await screen.findAllByTestId("phone-feedback")).at(-1) as HTMLElement;
    await user.type(within(page2).getByLabelText("描述"), "x");
    await user.click(within(page2).getByTestId("feedback-send"));
    expect(await within(page2).findByTestId("feedback-error")).toHaveTextContent("连不上反馈服务");
    expect(within(page2).getByLabelText("描述")).toHaveValue("x");
    failing.destroy();
  });

  it("a build without an endpoint offers the issue page", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ role: "phone", feedback: "not_configured" });
    renderApp({ backend, initialScreen: "settings" });
    const page = await openFeedback(user);
    expect(await within(page).findByTestId("feedback-not-configured")).toBeInTheDocument();
    expect(within(page).queryByTestId("feedback-attachments")).toBeNull();
    await user.click(within(page).getByRole("button", { name: "在 GitHub 上反馈" }));
    expect(backend.linksOpened).toEqual(["feedback"]);
    vi.spyOn(backend, "projectLinkOpen").mockRejectedValueOnce(new Error("没有浏览器"));
    await user.click(within(page).getByRole("button", { name: "在 GitHub 上反馈" }));
    expect(await screen.findByText("出错了 · 没有浏览器")).toBeInTheDocument();
    backend.destroy();
  });

  it("the diagnostics that cannot be read are a failure to send", async () => {
    const user = userEvent.setup();
    const backend = new MockBackend({ role: "phone" });
    vi.spyOn(backend, "feedbackDiagnostics").mockRejectedValue(new Error("down"));
    renderApp({ backend, initialScreen: "settings" });
    const page = await openFeedback(user);
    expect(await within(page).findByTestId("feedback-error")).toHaveTextContent("反馈服务出错了");
    expect(within(page).getByText("正在读取要附带的信息…")).toBeInTheDocument();
    backend.destroy();
  });
});
