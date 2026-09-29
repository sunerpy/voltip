import { MAX_PHONE_TEXT_CHARS, zhT } from "@voltip/shared";
import {
  MOCK_PHONE_CLIPBOARD,
  MOCK_TEXT_MS,
  MockBackend,
  sampleDevices,
} from "@voltip/shared/mock";
import { act, fireEvent, screen, within } from "@testing-library/react";
import { renderApp } from "../test/render";
import { sentTextLine } from "./SendText";

function desktops(state: "online" | "offline") {
  const now = Math.floor(Date.now() / 1000);
  const desktop = sampleDevices(now)[1];
  if (!desktop) throw new Error("fixture");
  return [
    {
      ...desktop,
      connection:
        state === "online"
          ? { state: "online" as const, via: "direct" as const }
          : { state: "offline" as const },
    },
  ];
}

async function advance(ms: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

describe("Send text to the computer (docs/dictation.md §20.6)", () => {
  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("regression: a typed text goes to the online computer, the box empties, and the list follows the answer", async () => {
    const backend = new MockBackend({ role: "phone", devices: desktops("online") });
    renderApp({ backend });
    const card = await screen.findByTestId("send-text");
    const box = within(card).getByRole("textbox", { name: "要发送的文字" });
    const send = within(card).getByRole("button", { name: "发送到 MacBook Pro" });
    expect(send).toBeDisabled();
    fireEvent.change(box, { target: { value: "会议改到三点" } });
    expect(within(card).getByTestId("send-text-count")).toHaveTextContent(
      `6/${MAX_PHONE_TEXT_CHARS} 字`,
    );
    fireEvent.click(send);
    await advance(0);
    expect(box).toHaveValue("");
    const item = within(card).getByTestId("sent-text");
    expect(item).toHaveTextContent("会议改到三点");
    expect(item).toHaveTextContent("输入 · MacBook Pro");
    expect(item).toHaveAttribute("data-state", "sending");
    await advance(MOCK_TEXT_MS);
    expect(within(card).getByTestId("sent-text")).toHaveTextContent("已送到 MacBook Pro");
    expect(backend.peek().sent_texts[0]?.source).toBe("typed");
  });

  it("sends the clipboard as it is, says so when it is empty, and the list can be cleared", async () => {
    const backend = new MockBackend({ role: "phone", devices: desktops("online") });
    renderApp({ backend });
    const card = await screen.findByTestId("send-text");
    fireEvent.click(within(card).getByRole("button", { name: "发送剪贴板" }));
    await advance(MOCK_TEXT_MS);
    const item = within(card).getByTestId("sent-text");
    expect(item).toHaveTextContent(MOCK_PHONE_CLIPBOARD);
    expect(item).toHaveTextContent("剪贴板 · MacBook Pro");
    backend.phoneClipboard = null;
    fireEvent.click(within(card).getByRole("button", { name: "发送剪贴板" }));
    expect(await screen.findByText("剪贴板里没有文字")).toBeInTheDocument();
    expect(within(card).getAllByTestId("sent-text")).toHaveLength(1);
    fireEvent.click(within(card).getByRole("button", { name: "清空" }));
    await advance(0);
    expect(within(card).queryByTestId("sent-text")).toBeNull();
  });

  it("a text over the limit cannot be sent, and nothing is offered without an online computer", async () => {
    const backend = new MockBackend({ role: "phone", devices: desktops("online") });
    const { unmount } = renderApp({ backend });
    const card = await screen.findByTestId("send-text");
    fireEvent.change(within(card).getByRole("textbox", { name: "要发送的文字" }), {
      target: { value: "字".repeat(MAX_PHONE_TEXT_CHARS + 1) },
    });
    expect(within(card).getByRole("button", { name: "发送到 MacBook Pro" })).toBeDisabled();
    unmount();
    renderApp({ backend: new MockBackend({ role: "phone", devices: desktops("offline") }) });
    const offline = await screen.findByTestId("send-text");
    expect(offline).toHaveTextContent("已配对的电脑上线后");
    expect(within(offline).queryByRole("textbox")).toBeNull();
  });

  it("every state has its line, each failure its own words", () => {
    const { t } = zhT;
    const text = (state: Parameters<typeof sentTextLine>[0]["state"]) => ({
      id: 1,
      device: "ab",
      device_name: "Studio",
      body: "x",
      source: "typed" as const,
      sent_at: 0,
      state,
    });
    expect(sentTextLine(text({ state: "sending" }), t)).toBe("发送中…");
    expect(sentTextLine(text({ state: "queued" }), t)).toBe("Studio 正在听写，结束后送出");
    expect(sentTextLine(text({ state: "delivered", pasted: false }), t)).toBe(
      "已放到 Studio 的剪贴板",
    );
    expect(
      sentTextLine(text({ state: "failed", code: "no_answer", message: "电脑没有回应" }), t),
    ).toBe("电脑没有回应");
    expect(sentTextLine(text({ state: "failed", code: "busy", message: "满了" }), t)).toContain(
      "排队的文字太多",
    );
  });
});
