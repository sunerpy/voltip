import {
  MOCK_ASR_MS,
  MOCK_CONNECTIVITY_MS,
  MOCK_DICTATION_TEXT,
  MOCK_MIC_READY_MS,
  MOCK_REFINE_MS,
  MockBackend,
  sampleDevices,
} from "@voltip/shared/mock";
import { PHONE_TAKE_FAILURES, zhT } from "@voltip/shared";
import { act, fireEvent, screen, within } from "@testing-library/react";
import { renderApp } from "../test/render";
import { phoneTakeLine } from "./PhoneMic";

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

/** The button sits at (0,0)–(300,96) for the off-button test (jsdom lays nothing out). */
function layOut(button: HTMLElement) {
  button.getBoundingClientRect = () => ({
    left: 0,
    top: 0,
    right: 300,
    bottom: 96,
    width: 300,
    height: 96,
    x: 0,
    y: 0,
    toJSON: () => ({}),
  });
}

describe("Phone as microphone (docs/dictation.md §20)", () => {
  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("hold streams to the online computer, release delivers, and the line follows the computer", async () => {
    const backend = new MockBackend({ role: "phone", devices: desktops("online") });
    renderApp({ backend });
    const card = await screen.findByTestId("phone-mic");
    const hold = within(card).getByTestId("phone-mic-hold");
    layOut(hold);
    expect(hold).toHaveTextContent("按住说话");
    expect(hold).toHaveTextContent("MacBook Pro");
    fireEvent.pointerDown(hold, { pointerId: 1, clientX: 10, clientY: 10 });
    expect(hold).toHaveAttribute("aria-pressed", "true");
    expect(hold).toHaveTextContent("松开发送");
    await advance(0);
    expect(within(card).getByTestId("phone-mic-state")).toHaveAttribute("data-state", "starting");
    await advance(MOCK_MIC_READY_MS);
    expect(within(card).getByTestId("phone-mic-state")).toHaveTextContent(/正在录音 · 00:0\d/);
    // docs/dictation.md §20.1: the computer decodes Opus, so the rest of the take is compressed.
    expect(within(card).getByTestId("phone-mic-codec")).toHaveTextContent("Opus 压缩传输");
    fireEvent.pointerUp(hold, { pointerId: 1, clientX: 10, clientY: 10 });
    await advance(0);
    expect(within(card).getByTestId("phone-mic-state")).toHaveTextContent("电脑正在识别…");
    await advance(MOCK_ASR_MS + MOCK_REFINE_MS);
    expect(within(card).getByTestId("phone-mic-state")).toHaveTextContent(
      `已插入电脑：${MOCK_DICTATION_TEXT}`,
    );
    expect(within(card).queryByTestId("phone-mic-codec")).toBeNull();
    expect(backend.peek().phone_take?.device).toBe(desktops("online")[0]?.device.public_key);
  });

  it("regression: the phone shows the level of its own take while it records, and no meter otherwise", async () => {
    const backend = new MockBackend({ role: "phone", devices: desktops("online") });
    renderApp({ backend });
    const card = await screen.findByTestId("phone-mic");
    const hold = within(card).getByTestId("phone-mic-hold");
    layOut(hold);
    expect(within(card).queryByRole("meter")).toBeNull();
    fireEvent.pointerDown(hold, { pointerId: 1, clientX: 10, clientY: 10 });
    await advance(MOCK_MIC_READY_MS);
    await advance(200);
    const meter = within(card).getByRole("meter", { name: "输入强度" });
    expect(Number(meter.getAttribute("aria-valuenow"))).toBeGreaterThan(0);
    expect(backend.activeMeters()).toBe(1);
    fireEvent.pointerUp(hold, { pointerId: 1, clientX: 10, clientY: 10 });
    await advance(0);
    expect(within(card).queryByRole("meter")).toBeNull();
    expect(backend.activeMeters()).toBe(0);
  });

  it("regression: the phone runs the connection check and sees the computer's channel and LAN addresses", async () => {
    const backend = new MockBackend({ role: "phone", devices: desktops("online") });
    renderApp({ backend });
    const check = await screen.findByTestId("connectivity");
    fireEvent.click(within(check).getByRole("button", { name: "开始自检" }));
    await advance(MOCK_CONNECTIVITY_MS);
    expect(within(check).getByTestId("connectivity-lan")).toHaveTextContent(
      "本机局域网服务 · 192.168.1.52:47831",
    );
    const peers = within(check).getAllByTestId("connectivity-peer");
    expect(peers).toHaveLength(1);
    expect(peers[0]).toHaveTextContent("MacBook Pro · 直连 · 加密往返 6 ms");
  });

  it("slide off the button before letting go cancels; a quick tap still stops what it started", async () => {
    const backend = new MockBackend({ role: "phone", devices: desktops("online") });
    renderApp({ backend });
    const hold = await screen.findByTestId("phone-mic-hold");
    layOut(hold);
    fireEvent.pointerDown(hold, { pointerId: 1, clientX: 10, clientY: 10 });
    await advance(MOCK_MIC_READY_MS);
    fireEvent.pointerMove(hold, { pointerId: 1, clientX: 10, clientY: -80 });
    expect(hold).toHaveTextContent("松开取消");
    expect(hold).toHaveAttribute("data-cancel", "true");
    fireEvent.pointerUp(hold, { pointerId: 1, clientX: 10, clientY: -80 });
    await advance(0);
    expect(screen.getByTestId("phone-mic-state")).toHaveTextContent("已取消");
    // Press and release at once: the stop waits for the start and reaches the same take.
    fireEvent.pointerDown(hold, { pointerId: 2, clientX: 10, clientY: 10 });
    fireEvent.pointerUp(hold, { pointerId: 2, clientX: 10, clientY: 10 });
    await advance(0);
    const take = backend.peek().phone_take;
    expect(take?.take).toBe(2);
    expect(take?.state.state).toBe("processing");
  });

  // Changed by the user's request of 2026-09-30 (item 10, docs/dictation.md §20.7): with no paired
  // computer online the phone transcribes the take itself; it showed a note and no button before.
  it("without an online computer the phone transcribes the take itself and copies the result", async () => {
    const backend = new MockBackend({ role: "phone", devices: desktops("offline") });
    renderApp({ backend });
    const card = await screen.findByTestId("phone-mic");
    expect(card).toHaveAttribute("data-route", "phone");
    expect(card).toHaveTextContent("已配对的电脑都不在线，现在在手机上识别。");
    const hold = within(card).getByTestId("phone-mic-hold");
    layOut(hold);
    expect(within(hold).getByTestId("phone-mic-route")).toHaveTextContent("在手机上识别");
    fireEvent.pointerDown(hold, { pointerId: 1, clientX: 10, clientY: 10 });
    expect(hold).toHaveTextContent("松开识别");
    await advance(MOCK_MIC_READY_MS);
    await advance(600);
    expect(within(card).getByTestId("phone-mic-state")).toHaveTextContent(/正在录音 · 00:0\d/);
    const meter = within(card).getByRole("meter", { name: "输入强度" });
    expect(Number(meter.getAttribute("aria-valuenow"))).toBeGreaterThan(0);
    fireEvent.pointerUp(hold, { pointerId: 1, clientX: 10, clientY: 10 });
    await advance(0);
    expect(within(card).getByTestId("phone-mic-state")).toHaveTextContent("正在识别…");
    await advance(MOCK_ASR_MS + MOCK_REFINE_MS);
    expect(within(card).getByTestId("phone-mic-state")).toHaveTextContent(
      `已复制到剪贴板：${MOCK_DICTATION_TEXT}`,
    );
    expect(backend.phoneClipboard).toBe(MOCK_DICTATION_TEXT);
    expect(backend.peek().phone_take).toBeUndefined();
    // The result is in the phone's own list, newest first.
    const recent = screen.getByTestId("phone-recent");
    expect(within(recent).getAllByTestId("phone-recent-row")[0]).toHaveTextContent(
      MOCK_DICTATION_TEXT,
    );
  });

  it("a take keeps its route: a computer coming online in the middle moves nothing", async () => {
    const backend = new MockBackend({ role: "phone", devices: desktops("offline") });
    renderApp({ backend });
    const hold = await screen.findByTestId("phone-mic-hold");
    layOut(hold);
    fireEvent.pointerDown(hold, { pointerId: 1, clientX: 10, clientY: 10 });
    await advance(MOCK_MIC_READY_MS);
    act(() => {
      backend.publish({ type: "devices", devices: desktops("online") });
    });
    expect(screen.getByTestId("phone-mic")).toHaveAttribute("data-route", "phone");
    fireEvent.pointerUp(hold, { pointerId: 1, clientX: 10, clientY: 10 });
    await advance(MOCK_ASR_MS + MOCK_REFINE_MS);
    expect(backend.phoneClipboard).toBe(MOCK_DICTATION_TEXT);
    // Once the take is over, the next one goes to the computer that came online.
    await advance(3000);
    expect(screen.getByTestId("phone-mic")).toHaveAttribute("data-route", "computer");
    expect(
      within(screen.getByTestId("phone-mic-hold")).getByTestId("phone-mic-route"),
    ).toHaveTextContent("发送到 MacBook Pro");
  });

  it("a result in the list is copied again or handed to the share sheet", async () => {
    const backend = new MockBackend({ role: "phone" });
    renderApp({ backend });
    const hold = await screen.findByTestId("phone-mic-hold");
    layOut(hold);
    fireEvent.pointerDown(hold, { pointerId: 1, clientX: 10, clientY: 10 });
    await advance(MOCK_MIC_READY_MS + 600);
    fireEvent.pointerUp(hold, { pointerId: 1, clientX: 10, clientY: 10 });
    await advance(MOCK_ASR_MS + MOCK_REFINE_MS);
    backend.phoneClipboard = null;
    const row = within(screen.getByTestId("phone-recent")).getAllByTestId("phone-recent-row")[0];
    if (!row) throw new Error("no result row");
    fireEvent.click(within(row).getByRole("button", { name: /^复制「/ }));
    expect(await screen.findByText("已复制")).toBeInTheDocument();
    expect(backend.phoneClipboard).toBe(MOCK_DICTATION_TEXT);
    fireEvent.click(within(row).getByRole("button", { name: /^分享「/ }));
    await advance(0);
    expect(backend.shared).toEqual([MOCK_DICTATION_TEXT]);
  });

  it("a refused start (the microphone permission) is a toast and leaves the button idle", async () => {
    const backend = new MockBackend({ role: "phone", devices: desktops("online") });
    vi.spyOn(backend, "invoke").mockRejectedValueOnce(new Error("microphone: 麦克风权限被拒绝"));
    renderApp({ backend });
    const hold = await screen.findByTestId("phone-mic-hold");
    fireEvent.pointerDown(hold, { pointerId: 1 });
    await advance(0);
    expect(await screen.findByText("出错了 · 麦克风权限被拒绝")).toBeInTheDocument();
    expect(hold).toHaveAttribute("aria-pressed", "false");
    fireEvent.pointerUp(hold, { pointerId: 1 });
    await advance(0);
    expect(backend.peek().phone_take).toBeUndefined();
  });

  it("with two computers online the take goes to the one picked; the keyboard holds too, and a lost pointer cancels", async () => {
    const [first] = desktops("online");
    if (!first) throw new Error("fixture");
    const second = {
      ...first,
      device: {
        ...first.device,
        public_key: "ab".repeat(32),
        name: "Studio PC",
        fingerprint: "AB:CD",
      },
    };
    const backend = new MockBackend({ role: "phone", devices: [first, second] });
    renderApp({ backend });
    const card = await screen.findByTestId("phone-mic");
    const select = within(card).getByRole("combobox", { name: "发送到" });
    fireEvent.change(select, { target: { value: second.device.public_key } });
    const hold = within(card).getByTestId("phone-mic-hold");
    expect(hold).toHaveTextContent("Studio PC");
    fireEvent.keyDown(hold, { key: " " });
    await advance(0);
    expect(backend.peek().phone_take?.device).toBe(second.device.public_key);
    expect(select).toBeDisabled();
    fireEvent.keyUp(hold, { key: " " });
    await advance(0);
    expect(backend.peek().phone_take?.state.state).toBe("processing");
    await advance(MOCK_ASR_MS + MOCK_REFINE_MS);
    fireEvent.pointerDown(hold, { pointerId: 3 });
    await advance(0);
    fireEvent.pointerCancel(hold, { pointerId: 3 });
    await advance(0);
    expect(backend.peek().phone_take?.state.state).toBe("cancelled");
  });

  it("every take state has its line, each failure its own words", () => {
    const t = zhT.t;
    expect(phoneTakeLine({ state: "done", text: "你好", pasted: false }, 0, t)).toBe(
      "已放到电脑剪贴板：你好",
    );
    const lines = PHONE_TAKE_FAILURES.map((code) =>
      phoneTakeLine({ state: "failed", code, message: "x" }, 0, t),
    );
    expect(new Set(lines).size).toBe(PHONE_TAKE_FAILURES.length);
    expect(phoneTakeLine({ state: "failed", code: "busy", message: "" }, 0, t)).toBe(
      "电脑正在听写，稍后再试",
    );
    expect(phoneTakeLine({ state: "listening" }, 65_000, t)).toBe("正在录音 · 01:05");
  });
});
