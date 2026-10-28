// 设置 › 反馈 on the phone (docs/feedback.md): a kind, the words, a contact and up to three files
// from the photo picker, with the exact diagnostics shown before anything is sent. The shell posts
// the report; a build without an endpoint offers the repository's issue page instead.
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react-native";
import { MockBackend } from "@voltip/shared/mock";
import { File } from "expo-file-system";
import * as ImagePicker from "expo-image-picker";

import { nth, openTab, renderApp } from "../test/render";

const pick = jest.mocked(ImagePicker.launchImageLibraryAsync);
const file = jest.mocked(File);

/** The picker hands over these files; each reads as `bytes` bytes. */
function picked(...assets: { name: string; type: string; size: number }[]) {
  file.mockImplementation(
    (uri) =>
      ({
        arrayBuffer: () => {
          const asset = assets.find((a) => `file:///${a.name}` === uri);
          return Promise.resolve(new ArrayBuffer(asset?.size ?? 0));
        },
      }) as unknown as File,
  );
  pick.mockResolvedValueOnce({
    canceled: false,
    assets: assets.map((a) => ({
      uri: `file:///${a.name}`,
      fileName: a.name,
      mimeType: a.type,
      fileSize: a.size,
      width: 1,
      height: 1,
    })),
  });
}

async function openFeedback() {
  await openTab("settings");
  await fireEvent.press(await screen.findByTestId("settings-feedback"));
  return screen.findByTestId("phone-feedback");
}

afterEach(() => {
  pick.mockReset();
  pick.mockResolvedValue({ canceled: true, assets: null });
  file.mockReset();
});

describe("设置 › 反馈", () => {
  it("shows what goes along, sends the kind, the words and the contact, and goes back", async () => {
    const { backend } = await renderApp();
    const page = await openFeedback();
    const attached = within(page).getByTestId("feedback-attached");
    expect(await within(attached).findByText("版本")).toBeOnTheScreen();
    expect(within(attached).getByText("界面语言")).toBeOnTheScreen();
    expect(within(page).getByTestId("feedback-send")).toBeDisabled();
    await fireEvent.press(within(page).getByTestId("feedback-kind-idea"));
    await fireEvent.changeText(within(page).getByTestId("feedback-message"), "希望支持粤语");
    expect(within(page).getByTestId("feedback-count")).toHaveTextContent(/^6 \//);
    await fireEvent.changeText(
      within(page).getByTestId("feedback-contact"),
      "  user@example.test ",
    );
    await fireEvent.press(within(page).getByTestId("feedback-send"));
    expect(await screen.findByText("反馈已发送，谢谢")).toBeOnTheScreen();
    expect(backend.feedbackSent).toEqual([
      expect.objectContaining({
        kind: "idea",
        message: "希望支持粤语",
        contact: "user@example.test",
        locale: "zh-CN",
      }),
    ]);
    expect(backend.feedbackSent[0]).not.toHaveProperty("attachments");
    await waitFor(() => {
      expect(screen.queryByTestId("phone-feedback")).toBeNull();
    });
    expect(screen.getByTestId("phone-settings")).toBeOnTheScreen();
  });

  it("attaches files from the picker in order, refuses one it cannot take, and removes one", async () => {
    const { backend } = await renderApp();
    const page = await openFeedback();
    const list = within(page).getByTestId("feedback-attachments");
    // A picker closed without a choice changes nothing.
    await fireEvent.press(within(list).getByTestId("feedback-attach"));
    await waitFor(() => {
      expect(pick).toHaveBeenCalledTimes(1);
    });
    expect(within(list).queryAllByTestId("feedback-attachment")).toHaveLength(0);
    picked(
      { name: "screen.png", type: "image/png", size: 2048 },
      { name: "clip.mp4", type: "video/mp4", size: 4096 },
      { name: "notes.pdf", type: "application/pdf", size: 10 },
      { name: "after.png", type: "image/png", size: 10 },
    );
    await fireEvent.press(within(list).getByTestId("feedback-attach"));
    await waitFor(() => {
      expect(within(list).getAllByTestId("feedback-attachment")).toHaveLength(2);
    });
    expect(pick).toHaveBeenLastCalledWith(expect.objectContaining({ selectionLimit: 3 }));
    expect(within(page).getByTestId("feedback-refusal")).toHaveTextContent(
      "「notes.pdf」不能附上：只支持 PNG、JPEG、GIF、WebP 图片和 MP4、WebM、MOV 视频。",
    );
    expect(backend.feedbackStaged.map((a) => a.name)).toEqual(["screen.png", "clip.mp4"]);
    await fireEvent.press(within(list).getByLabelText("移除 screen.png"));
    await waitFor(() => {
      expect(backend.feedbackStaged.map((a) => a.name)).toEqual(["clip.mp4"]);
    });
    expect(within(page).queryByTestId("feedback-refusal")).toBeNull();
    // The shell refuses an empty file; the reason names it.
    picked({ name: "empty.png", type: "image/png", size: 0 });
    await fireEvent.press(within(list).getByTestId("feedback-attach"));
    expect(await within(page).findByTestId("feedback-refusal")).toHaveTextContent(/「empty.png」/);
    await fireEvent.changeText(within(page).getByTestId("feedback-message"), "录屏里能看到问题");
    await fireEvent.press(within(page).getByTestId("feedback-send"));
    expect(await screen.findByText("反馈已发送，谢谢")).toBeOnTheScreen();
    expect(backend.feedbackSent[0]?.attachments).toEqual(["attachment-2"]);
  });

  it("keeps the report when the service fails", async () => {
    await renderApp({ mock: { feedback: "server" } });
    const page = await openFeedback();
    await fireEvent.changeText(within(page).getByTestId("feedback-message"), "闪退了");
    await fireEvent.press(within(page).getByTestId("feedback-send"));
    expect(await within(page).findByTestId("feedback-error")).toHaveTextContent(
      "反馈服务出错了，请稍后重试。",
    );
    expect(within(page).getByTestId("feedback-message")).toHaveProp("value", "闪退了");
  });

  it("clears the report when it went out and only a file did not follow", async () => {
    const { backend: partial } = await renderApp({ mock: { feedback: "attachments" } });
    const page = await openFeedback();
    picked({ name: "screen.png", type: "image/png", size: 64 });
    await fireEvent.press(within(page).getByTestId("feedback-attach"));
    await waitFor(() => {
      expect(within(page).getAllByTestId("feedback-attachment")).toHaveLength(1);
    });
    await fireEvent.changeText(within(page).getByTestId("feedback-message"), "附件传不上");
    await fireEvent.press(within(page).getByTestId("feedback-send"));
    expect(await within(page).findByTestId("feedback-error")).toHaveTextContent(
      "反馈已发送，但附件没有传完整。",
    );
    // The report went out: sending again would send it twice.
    expect(partial.feedbackSent).toHaveLength(1);
    expect(within(page).getByTestId("feedback-message")).toHaveProp("value", "");
    expect(within(page).queryAllByTestId("feedback-attachment")).toHaveLength(0);
  });

  it("offers the issue page in a build without an endpoint, and says when the page cannot open", async () => {
    const backend = new MockBackend({ role: "phone", feedback: "not_configured" });
    await renderApp({ backend });
    const page = await openFeedback();
    expect(
      await within(page).findByText("此版本未配置反馈地址，可以在 GitHub 上提交 Issue。"),
    ).toBeOnTheScreen();
    expect(within(page).queryByTestId("feedback-attachments")).toBeNull();
    expect(within(page).queryByTestId("feedback-send")).toBeNull();
    await fireEvent.press(within(page).getByTestId("feedback-open-issue"));
    expect(backend.linksOpened).toEqual(["feedback"]);
    jest.spyOn(backend, "projectLinkOpen").mockRejectedValueOnce(new Error("没有浏览器"));
    await fireEvent.press(within(page).getByTestId("feedback-open-issue"));
    expect(await screen.findByText(/没有浏览器/)).toBeOnTheScreen();
  });

  it("says the service is unreachable when the diagnostics cannot be read", async () => {
    const backend = new MockBackend({ role: "phone" });
    jest.spyOn(backend, "feedbackDiagnostics").mockRejectedValue(new Error("offline"));
    await renderApp({ backend });
    const page = await openFeedback();
    expect(await within(page).findByTestId("feedback-error")).toHaveTextContent(
      "反馈服务出错了，请稍后重试。",
    );
    expect(within(page).getByText("正在读取要附带的信息…")).toBeOnTheScreen();
    await fireEvent.changeText(within(page).getByTestId("feedback-message"), "看不到诊断信息");
    expect(within(page).getByTestId("feedback-send")).toBeDisabled();
  });

  it("names a picked file that came without a name, a type or a size, and takes an empty pick as none", async () => {
    await renderApp();
    const page = await openFeedback();
    const list = within(page).getByTestId("feedback-attachments");
    pick.mockResolvedValueOnce({
      canceled: false,
      assets: [],
    } as unknown as ImagePicker.ImagePickerResult);
    await fireEvent.press(within(list).getByTestId("feedback-attach"));
    await waitFor(() => {
      expect(pick).toHaveBeenCalledTimes(1);
    });
    expect(within(page).queryByTestId("feedback-refusal")).toBeNull();
    pick.mockResolvedValueOnce({
      canceled: false,
      assets: [{ uri: "file:///x", width: 1, height: 1 }],
    });
    await fireEvent.press(within(list).getByTestId("feedback-attach"));
    expect(await within(page).findByTestId("feedback-refusal")).toHaveTextContent(
      /「attachment-1」/,
    );
  });

  it("takes a file off the list even when the shell could not drop it", async () => {
    const { backend } = await renderApp();
    const page = await openFeedback();
    picked({ name: "screen.png", type: "image/png", size: 64 });
    await fireEvent.press(within(page).getByTestId("feedback-attach"));
    await waitFor(() => {
      expect(within(page).getAllByTestId("feedback-attachment")).toHaveLength(1);
    });
    const remove = jest
      .spyOn(backend, "feedbackAttachmentRemove")
      .mockRejectedValueOnce(new Error("gone"));
    await fireEvent.press(within(page).getByLabelText("移除 screen.png"));
    await waitFor(() => {
      expect(remove).toHaveBeenCalled();
    });
    expect(within(page).queryAllByTestId("feedback-attachment")).toHaveLength(0);
  });

  it("ignores the diagnostics when the page was left before they came", async () => {
    const backend = new MockBackend({ role: "phone" });
    let answer:
      | ((info: Awaited<ReturnType<MockBackend["feedbackDiagnostics"]>>) => void)
      | undefined;
    let fail: ((e: unknown) => void) | undefined;
    const real = backend.feedbackDiagnostics.bind(backend);
    jest
      .spyOn(backend, "feedbackDiagnostics")
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            answer = resolve;
          }),
      )
      .mockImplementationOnce(
        () =>
          new Promise((_resolve, reject) => {
            fail = reject;
          }),
      );
    await renderApp({ backend });
    await openFeedback();
    await fireEvent.press(nth(screen.getAllByLabelText("返回"), 0));
    expect(await screen.findByTestId("phone-settings")).toBeOnTheScreen();
    await act(async () => {
      answer?.(await real("zh-CN"));
    });
    await openFeedback();
    await fireEvent.press(nth(screen.getAllByLabelText("返回"), 0));
    expect(await screen.findByTestId("phone-settings")).toBeOnTheScreen();
    await act(async () => {
      fail?.(new Error("late"));
      await Promise.resolve();
    });
    expect(screen.queryByTestId("phone-feedback")).toBeNull();
  });

  it("says why the issue page did not open when the shell gives a plain reason", async () => {
    const backend = new MockBackend({ role: "phone", feedback: "not_configured" });
    await renderApp({ backend });
    const page = await openFeedback();
    jest.spyOn(backend, "projectLinkOpen").mockRejectedValueOnce("没有浏览器");
    await fireEvent.press(await within(page).findByTestId("feedback-open-issue"));
    expect(await screen.findByText(/没有浏览器/)).toBeOnTheScreen();
  });
});
