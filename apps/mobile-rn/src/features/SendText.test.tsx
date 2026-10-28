// 发送文字到电脑 on 说话 (docs/dictation.md §20.6): a typed text or the clipboard goes to an online
// computer, which inserts it at its cursor; the list below follows each text's answer.
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react-native";
import { MAX_PHONE_TEXT_CHARS, type SentText, zhT } from "@voltip/shared";
import { MOCK_PHONE_CLIPBOARD, type MockBackend } from "@voltip/shared/mock";

import { nth, renderApp } from "../test/render";
import { sentTextLine } from "./SendText";

function computer(key: string, name: string, online = true) {
  return {
    device: {
      device_id: `0f3f1a1e-8d4b-4c8e-9f7a-1c2d3e4f5a${key.slice(0, 2)}`,
      public_key: key.repeat(64 / key.length),
      name,
      platform: "windows",
      fingerprint: "AB:CD",
      trusted_at: 1_758_700_000,
      last_seen: 1_758_700_600,
    },
    connection: { state: online ? "online" : "offline" },
  } as never;
}

function sent(id: number, state: SentText["state"], over: Partial<SentText> = {}): SentText {
  return {
    id,
    device: "1".repeat(64),
    device_name: "Studio",
    body: `第 ${id} 条`,
    source: "typed",
    sent_at: Date.now(),
    state,
    ...over,
  };
}

/** The core reported the list (the mock only reports sending and delivered on its own). */
async function report(backend: MockBackend, texts: SentText[]) {
  await act(async () => {
    backend.publish({ type: "sent_texts", texts });
    await Promise.resolve();
  });
}

describe("发送文字到电脑", () => {
  it("sends a typed text to the online computer, empties the box, and lists the answer", async () => {
    const { backend } = await renderApp({ mock: { devices: [computer("1", "Studio")] } });
    const card = within(await screen.findByTestId("send-text"));
    expect(
      card.getByText(
        "输入或粘贴一段文字，电脑会把它插入光标处；也可以直接发送剪贴板。电脑正在听写时，文字排队等听写结束。",
      ),
    ).toBeOnTheScreen();
    // One computer: nothing to choose.
    expect(card.queryByLabelText("发送到")).toBeNull();
    const send = card.getByText("发送到 Studio");
    expect(send).toBeDisabled();
    await fireEvent.changeText(card.getByTestId("send-text-draft"), "会议改到三点");
    expect(card.getByText(`6/${MAX_PHONE_TEXT_CHARS} 字`)).toBeOnTheScreen();
    await fireEvent.press(send);
    await waitFor(() => {
      expect(card.getByTestId("send-text-draft")).toHaveDisplayValue("");
    });
    const item = card.getByTestId("sent-text");
    expect(item).toHaveTextContent(/会议改到三点/);
    expect(item).toHaveTextContent(/输入 · Studio/);
    expect(await within(item).findByText("已插入 Studio")).toBeOnTheScreen();
    expect(card.getByText("已发送 1 条")).toBeOnTheScreen();
    expect(backend.peek().sent_texts[0]).toMatchObject({ body: "会议改到三点", source: "typed" });
  });

  it("sends the clipboard as it is, says when it is empty, and clears the list", async () => {
    const { backend } = await renderApp({ mock: { devices: [computer("1", "Studio")] } });
    const card = within(await screen.findByTestId("send-text"));
    await fireEvent.press(card.getByText("发送剪贴板"));
    const item = await card.findByTestId("sent-text");
    expect(item).toHaveTextContent(new RegExp(MOCK_PHONE_CLIPBOARD.replaceAll(".", "\\.")));
    expect(item).toHaveTextContent(/剪贴板 · Studio/);
    backend.phoneClipboard = "   ";
    await fireEvent.press(card.getByText("发送剪贴板"));
    expect(await screen.findByText("剪贴板里没有文字")).toBeOnTheScreen();
    backend.phoneClipboard = null;
    await fireEvent.press(card.getByText("发送剪贴板"));
    expect(card.getAllByTestId("sent-text")).toHaveLength(1);
    await fireEvent.press(card.getByText("清空"));
    await waitFor(() => {
      expect(card.queryByTestId("sent-text")).toBeNull();
    });
  });

  it("refuses a text over the limit, picks among online computers, and offers nothing without one", async () => {
    const { backend } = await renderApp({
      mock: {
        devices: [computer("1", "Studio"), computer("2", "笔记本"), computer("3", "家里", false)],
      },
    });
    const card = within(await screen.findByTestId("send-text"));
    await fireEvent.changeText(
      card.getByTestId("send-text-draft"),
      "字".repeat(MAX_PHONE_TEXT_CHARS + 1),
    );
    expect(
      card.getByText(`${MAX_PHONE_TEXT_CHARS + 1}/${MAX_PHONE_TEXT_CHARS} 字`),
    ).toBeOnTheScreen();
    expect(card.getByText("发送到 Studio")).toBeDisabled();
    await fireEvent.changeText(card.getByTestId("send-text-draft"), "你好");
    // Only the online ones are offered.
    await fireEvent.press(card.getByLabelText("发送到"));
    expect(screen.queryByRole("radio", { name: "家里" })).toBeNull();
    await fireEvent.press(await screen.findByRole("radio", { name: "笔记本" }));
    await fireEvent.press(await card.findByText("发送到 笔记本"));
    await waitFor(() => {
      expect(backend.peek().sent_texts[0]).toMatchObject({
        device: "2".repeat(64),
        device_name: "笔记本",
      });
    });

    await act(async () => {
      backend.simulateDeviceConnection("1".repeat(64), { state: "offline" });
      backend.simulateDeviceConnection("2".repeat(64), { state: "offline" });
      await Promise.resolve();
    });
    expect(await card.findByText("已配对的电脑上线后，可以在这里把文字发过去。")).toBeOnTheScreen();
    expect(card.queryByTestId("send-text-draft")).toBeNull();
    // What was sent stays listed.
    expect(card.getByTestId("sent-text")).toBeOnTheScreen();
  });

  it("says why a text or the clipboard could not go", async () => {
    const { backend } = await renderApp({ mock: { devices: [computer("1", "Studio")] } });
    const card = within(await screen.findByTestId("send-text"));
    const invoke = jest.spyOn(backend, "invoke");
    invoke.mockRejectedValueOnce(new Error("电脑拒绝了"));
    await fireEvent.changeText(card.getByTestId("send-text-draft"), "你好");
    await fireEvent.press(card.getByText("发送到 Studio"));
    expect(await screen.findByText(/电脑拒绝了/)).toBeOnTheScreen();
    // The text stays in the box to try again.
    expect(card.getByTestId("send-text-draft")).toHaveDisplayValue("你好");
    invoke.mockRestore();
    jest.spyOn(backend, "phoneClipboardRead").mockRejectedValueOnce("读不到剪贴板");
    await fireEvent.press(card.getByText("发送剪贴板"));
    expect(await screen.findByText(/读不到剪贴板/)).toBeOnTheScreen();
  });

  it("lists each text with the line for where it is on the computer", async () => {
    const { backend } = await renderApp({ mock: { devices: [computer("1", "Studio")] } });
    const card = within(await screen.findByTestId("send-text"));
    await report(backend, [
      sent(4, { state: "queued" }),
      sent(3, { state: "delivered", pasted: false }, { source: "clipboard" }),
      sent(2, { state: "failed", code: "offline", message: "电脑不在线" }),
      sent(1, { state: "sending" }),
    ]);
    const items = card.getAllByTestId("sent-text");
    expect(items).toHaveLength(4);
    expect(within(nth(items, 0)).getByText("Studio 正在听写，结束后插入")).toBeOnTheScreen();
    expect(within(nth(items, 1)).getByText("已放到 Studio 的剪贴板")).toBeOnTheScreen();
    expect(within(nth(items, 2)).getByText("电脑不在线：电脑不在线")).toBeOnTheScreen();
    expect(within(nth(items, 3)).getByText("发送中…")).toBeOnTheScreen();
  });

  it("has a line for every state, each failure in its own words", () => {
    const { t } = zhT;
    expect(sentTextLine(sent(1, { state: "sending" }), t)).toBe("发送中…");
    expect(sentTextLine(sent(1, { state: "queued" }), t)).toBe("Studio 正在听写，结束后插入");
    expect(sentTextLine(sent(1, { state: "delivered", pasted: true }), t)).toBe("已插入 Studio");
    expect(sentTextLine(sent(1, { state: "delivered", pasted: false }), t)).toBe(
      "已放到 Studio 的剪贴板",
    );
    const failed = (code: "busy" | "unavailable" | "failed" | "offline" | "no_answer") =>
      sentTextLine(sent(1, { state: "failed", code, message: "原因" }), t);
    expect(failed("busy")).toBe("电脑上排队的文字太多：原因");
    expect(failed("unavailable")).toBe("对方不接收文字：原因");
    expect(failed("failed")).toBe("插入失败：原因");
    expect(failed("offline")).toBe("电脑不在线：原因");
    expect(failed("no_answer")).toBe("原因");
  });
});
