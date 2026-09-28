import {
  MOCK_CONNECTIVITY_MS,
  MOCK_PUBLIC_KEYS,
  MockBackend,
  sampleDevices,
} from "@voltip/shared/mock";
import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../test/render";

/** Fixed pixel panel sizes and two-fixed-column grids broke the 1440 / 1920 px windows (Windows
 *  test 2026-09-24); `max-w-[…]` / `min-w-[…]` caps stay allowed (the root is `max-w-[1440px]`). */
const FIXED_SIZE = /(?:^|\s)w-\[\d+px\]|(?:^|\s)h-\[604px\]|grid-cols-\[[^\]]*\d+px_\d+px[^\]]*\]/;

/** Allow-list: the QR code placeholder keeps the QR's 168 px module size while the session is
 *  being created ; the rendered QrCode itself sizes through its SVG attributes. */
function isAllowedFixedSize(el: Element): boolean {
  return el.getAttribute("aria-label") === "正在生成二维码";
}

function fixedSizeOffenders(root: HTMLElement): string[] {
  return [...root.querySelectorAll("*")]
    .filter((el) => !isAllowedFixedSize(el))
    .map((el) => el.getAttribute("class") ?? "")
    .filter((cls) => FIXED_SIZE.test(cls));
}

function mount(options: { devices?: ReturnType<typeof sampleDevices> } = {}) {
  const backend = new MockBackend({
    devices: options.devices ?? sampleDevices(Math.floor(Date.now() / 1000)),
  });
  return renderApp({ path: "/devices", backend });
}

describe("Devices page", () => {
  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("regression: devices is fluid (no fixed-width panels)", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    mount();
    const page = await screen.findByTestId("page-devices");
    expect(page).toHaveClass("mx-auto", "w-full", "max-w-[1440px]", "p-6", "grid-cols-1");
    expect(page.className).toMatch(/lg:grid-cols-\[minmax\(0,1fr\)_minmax\(300px,380px\)\]/);
    expect(fixedSizeOffenders(page)).toEqual([]);
    // The pairing panel's QR / code / countdown state and the key-exchange state are fluid too.
    await user.click(screen.getByRole("button", { name: "开始配对" }));
    expect(fixedSizeOffenders(page)).toEqual([]);
    expect(page.querySelector('[aria-label="正在生成二维码"]')).toHaveClass("w-[168px]");
    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });
    expect(screen.getByRole("img", { name: "配对二维码" })).toBeInTheDocument();
    expect(fixedSizeOffenders(page)).toEqual([]);
  });

  it("renders the device table with the design's columns and readouts", async () => {
    mount();
    const table = await screen.findByRole("table", { name: "已配对设备" });
    const headers = within(table)
      .getAllByRole("columnheader")
      .map((h) => h.textContent);
    expect(headers).toEqual(["设备", "局域网地址", "最近在线", "状态", ""]);
    // The table exists before `core_state` resolves (empty placeholder); wait for the first row.
    expect(await within(table).findByText("Pixel 8 · Android")).toBeInTheDocument();
    expect(within(table).getByText("在线 · 直连")).toBeInTheDocument();
    expect(within(table).getByText("离线")).toBeInTheDocument();
    expect(within(table).getByText("3 分钟前")).toBeInTheDocument();
    expect(screen.getByText("2 台已配对 · 1 在线")).toBeInTheDocument();
    expect(screen.getByText("Pixel 8 在线 · 在手机上按住「按住说话」")).toBeInTheDocument();
    expect(screen.getByText("经这条通道传输")).toBeInTheDocument();
    expect(screen.getByText(/局限 · 不是什么/)).toBeInTheDocument();
  });

  it("regression: the phone microphone panel follows a phone's take — its name, the timer, the meter, the result — and nothing on the page is marked not wired", async () => {
    const { backend } = mount();
    const panel = await screen.findByTestId("live-panel");
    await within(panel).findByText("Pixel 8 在线 · 在手机上按住「按住说话」");
    const meter = within(panel).getByRole("meter", { name: "来自手机的声音强度" });
    expect(meter).toHaveAttribute("aria-valuenow", "0");
    expect(within(panel).getByText("Opus · 16 kHz 单声道")).toBeInTheDocument();
    expect(within(panel).getByText("直连")).toBeInTheDocument();
    act(() => {
      backend.simulatePhoneTake("Pixel 8");
    });
    expect(panel).toHaveAttribute("data-remote", "Pixel 8");
    expect(await within(panel).findByText(/^Pixel 8 · 正在收音 00:0\d$/)).toBeInTheDocument();
    // The meter shows the phone's audio while it records (the shell feeds its levels).
    await waitFor(() => {
      expect(
        Number(within(panel).getByRole("meter").getAttribute("aria-valuenow")),
      ).toBeGreaterThan(0);
    });
    act(() => {
      backend.simulatePhoneTakeStop();
    });
    expect(await within(panel).findByText("Pixel 8 · 正在识别")).toBeInTheDocument();
    expect(
      await within(panel).findByText(/^Pixel 8 · 已插入 \d+ 字$/, {}, { timeout: 5000 }),
    ).toBeInTheDocument();
    // Opus used to be promised for a later phase; the phone streams it now (docs/dictation.md §20),
    // so the readout above names it and only the placeholder wording is banned.
    expect(document.body.textContent).not.toMatch(/尚未接入|计划|第二阶段/);
    expect(screen.queryByTestId("deferred-badge")).toBeNull();
    // The channel carries exactly text, the phone's audio and the recognised text.
    expect(screen.getByText("手机录音")).toBeInTheDocument();
    expect(screen.getByText("识别结果")).toBeInTheDocument();
    expect(screen.getByText("解除配对")).toBeInTheDocument();
    expect(screen.getByText("两边都删除记录")).toBeInTheDocument();
    mount({ devices: [] });
    expect((await screen.findAllByText("当前没有手机在线")).length).toBeGreaterThan(0);
  });

  it("regression: the pairing footer says how devices connect and shows the relay's real state; no self-check chips that probe nothing", async () => {
    mount();
    const footer = await screen.findByTestId("pairing-connect");
    expect(footer).toHaveTextContent(
      "配对用二维码或 6 位码；之后同一局域网内直连，跨网络经中继，中继只转发密文。",
    );
    expect(within(footer).getByText(/^中继 · /)).toBeInTheDocument();
    expect(screen.queryByTestId("self-check")).toBeNull();
    expect(document.body.textContent).not.toMatch(/mDNS|防火墙规则|AP 客户端隔离/);
  });

  it("regression: the connection check probes and reports this computer's LAN host, the relay, and each phone's channel and LAN addresses", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    const { backend } = mount();
    const check = await screen.findByTestId("connectivity");
    expect(within(check).queryByRole("list")).toBeNull();
    await user.click(within(check).getByRole("button", { name: "开始自检" }));
    expect(within(check).getByRole("button", { name: "自检中…" })).toBeDisabled();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(MOCK_CONNECTIVITY_MS);
    });
    expect(within(check).getByTestId("connectivity-lan")).toHaveTextContent(
      "本机局域网监听 · 192.168.1.30:47831",
    );
    expect(within(check).getByTestId("connectivity-relay")).toHaveTextContent("没有配置中继");
    const peers = within(check).getAllByTestId("connectivity-peer");
    expect(peers).toHaveLength(2);
    expect(peers[0]).toHaveTextContent("Pixel 8 · 直连 · 加密往返 6 ms");
    expect(peers[0]).toHaveTextContent("192.168.1.37:47831 · 可连接 · 5 ms");
    expect(peers[1]).toHaveTextContent("MacBook Pro · 离线");
    expect(peers[1]).toHaveTextContent("没有记录它的局域网地址");
    expect(within(check).getByRole("button", { name: "重新自检" })).toBeEnabled();
    expect(backend.peek().connectivity.report?.peers).toHaveLength(2);
  });

  it("walks the full pairing flow: start → code + QR + countdown → verification → confirm → trusted → device online", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    const { backend } = mount({ devices: [] });
    expect(await screen.findByText("还没有配对的手机")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "开始配对" }));
    expect(screen.getByTestId("pairing-panel")).toHaveAttribute("data-phase", "creating_session");
    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });
    expect(screen.getByTestId("pairing-panel")).toHaveAttribute("data-phase", "waiting_for_peer");
    expect(screen.getByTestId("pairing-code")).toHaveTextContent(/^\d{3} \d{3}$/);
    expect(screen.getByRole("img", { name: "配对二维码" })).toBeInTheDocument();
    expect(screen.getByTestId("pairing-countdown")).toHaveTextContent("02:00 / 02:00");
    expect(screen.getByText(/剩余 02:00/)).toBeInTheDocument();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(13_000);
    });
    expect(screen.getByTestId("pairing-countdown")).toHaveTextContent("01:47 / 02:00");

    act(() => {
      backend.simulatePeerJoined();
    });
    expect(screen.getByText(/正在协商密钥/)).toBeInTheDocument();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(400);
    });
    expect(screen.getByTestId("pairing-panel")).toHaveAttribute(
      "data-phase",
      "awaiting_verification",
    );
    expect(screen.getByRole("list", { name: "安全码" }).children).toHaveLength(4);
    expect(screen.getByText("Pixel 8")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "确认配对" }));
    expect(screen.getByRole("button", { name: "等待对方确认…" })).toBeDisabled();
    act(() => {
      backend.simulatePeerConfirmed();
    });
    expect(screen.getByTestId("pairing-panel")).toHaveAttribute("data-phase", "trusted");
    expect(screen.getAllByText("已信任 · Pixel 8").length).toBeGreaterThan(0);
    const table = screen.getByRole("table", { name: "已配对设备" });
    expect(within(table).getByText("在线 · 直连")).toBeInTheDocument();
    expect(screen.getByText("1 台已配对 · 1 在线")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "完成" }));
    expect(screen.getByTestId("pairing-panel")).toHaveAttribute("data-phase", "idle");
  });

  it("regression: countdown reaching zero shows the expired state with a 重新生成 action that restarts", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    mount({ devices: [] });
    await user.click(await screen.findByRole("button", { name: "开始配对" }));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(300 + 119_000);
    });
    expect(screen.getByTestId("pairing-countdown")).toHaveTextContent("00:01 / 02:00");
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1000);
    });
    expect(screen.getByTestId("pairing-panel")).toHaveAttribute("data-phase", "expired");
    expect(screen.getByTestId("pairing-countdown")).toHaveTextContent("00:00 / 02:00");
    expect(screen.getAllByText("已过期").length).toBeGreaterThan(0);
    expect(screen.getByRole("img", { name: "配对二维码" })).toHaveClass("opacity-40");
    await user.click(screen.getByRole("button", { name: "重新生成" }));
    expect(screen.getByTestId("pairing-panel")).toHaveAttribute("data-phase", "creating_session");
    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });
    expect(screen.getByTestId("pairing-panel")).toHaveAttribute("data-phase", "waiting_for_peer");
  });

  it("regression: Ctrl R starts a code when idle, regenerates a shown one and never interrupts a handshake", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    const { backend } = mount({ devices: [] });
    await screen.findByText("还没有配对的手机");
    const panel = () => screen.getByTestId("pairing-panel");
    await user.keyboard("{Control>}r{/Control}");
    expect(panel()).toHaveAttribute("data-phase", "creating_session");
    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });
    expect(panel()).toHaveAttribute("data-phase", "waiting_for_peer");
    const code = screen.getByTestId("pairing-code").textContent;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5000);
    });
    await user.keyboard("{Control>}r{/Control}");
    expect(panel()).toHaveAttribute("data-phase", "creating_session");
    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });
    expect(screen.getByTestId("pairing-countdown")).toHaveTextContent("02:00 / 02:00");
    expect(screen.getByTestId("pairing-code").textContent).not.toBe(code);
    act(() => {
      backend.simulatePeerJoined();
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(400);
    });
    expect(panel()).toHaveAttribute("data-phase", "awaiting_verification");
    await user.keyboard("{Control>}r{/Control}");
    expect(panel()).toHaveAttribute("data-phase", "awaiting_verification");
  });

  it("regression: rejection ends the session and the peer's rejection is reflected too", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    const { backend } = mount({ devices: [] });
    await user.click(await screen.findByRole("button", { name: "开始配对" }));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });
    act(() => {
      backend.simulatePeerJoined();
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(400);
    });
    await user.click(screen.getByRole("button", { name: "拒绝" }));
    expect(screen.getByText("配对已被拒绝，会话已销毁。")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "重新开始" }));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });
    await user.click(screen.getByRole("button", { name: "取消" }));
    expect(screen.getByText(/配对失败 · 已取消/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "重新开始" }));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });
    act(() => {
      backend.simulatePeerLeft();
    });
    expect(screen.getAllByText(/对端已离开/).length).toBeGreaterThan(0);
  });

  it("copies the link and fingerprint while waiting", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    const writeText = vi.fn(() => Promise.resolve());
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    mount({ devices: [] });
    await user.click(await screen.findByRole("button", { name: "开始配对" }));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });
    await user.click(screen.getByRole("button", { name: "复制链接" }));
    expect(writeText).toHaveBeenCalledWith(expect.stringMatching(/^voltip:\/\/pair/));
    expect(await screen.findByText("已复制配对链接")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "复制指纹" }));
    expect(writeText).toHaveBeenCalledWith("A7:C4:19:8E · 3D:F2:61:09");
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText: () => Promise.reject(new Error("denied")) },
      configurable: true,
    });
    await user.click(screen.getByRole("button", { name: "复制链接" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("无法访问剪贴板 · 未复制");
  });

  it("regression: an identity change raises a banner, marks the row and never auto-trusts; forgetting clears it", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    const { backend } = mount();
    await screen.findByRole("table", { name: "已配对设备" });
    act(() => {
      backend.simulateIdentityChanged(MOCK_PUBLIC_KEYS.phone, "FF:00:11:22 · 33:44:55:66");
    });
    const banner = await screen.findByText("「Pixel 8」出示了不同的身份密钥");
    expect(banner).toBeInTheDocument();
    expect(screen.getByText(/本次出示 FF:00:11:22 · 33:44:55:66/)).toBeInTheDocument();
    expect(
      within(screen.getByRole("table", { name: "已配对设备" })).getByText("身份已变化"),
    ).toBeInTheDocument();
    expect(
      backend.peek().devices.find((d) => d.device.public_key === MOCK_PUBLIC_KEYS.phone)?.device
        .fingerprint,
    ).toBe("5B:0F:E2:91 · C3:7A:0D:44");
    await user.click(screen.getByRole("button", { name: "稍后处理" }));
    expect(screen.queryByText("「Pixel 8」出示了不同的身份密钥")).toBeNull();
    act(() => {
      backend.simulateIdentityChanged(MOCK_PUBLIC_KEYS.laptop, "AA:BB");
    });
    await user.click(await screen.findByRole("button", { name: "忘记设备" }));
    await user.click(
      screen.getByRole("dialog").querySelector("button[data-variant='danger']") as HTMLElement,
    );
    await waitFor(() => {
      expect(
        backend.peek().devices.some((d) => d.device.public_key === MOCK_PUBLIC_KEYS.laptop),
      ).toBe(false);
    });
    expect(await screen.findByText("已忘记 MacBook Pro")).toBeInTheDocument();
  });

  it("regression: relay reconnecting shows a readout with the attempt count; the relay toggle drives settings", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    const { backend } = mount();
    await screen.findByRole("table", { name: "已配对设备" });
    act(() => {
      backend.simulateRelay({
        endpoint: "wss://relay.example.test",
        state: "reconnecting",
        attempts: 3,
      });
    });
    expect((await screen.findAllByText("中继 · 重连中 · 第 3 次")).length).toBeGreaterThan(0);
    act(() => {
      backend.simulateRelay({ state: "connected", attempts: 0 });
    });
    expect((await screen.findAllByText("中继 · 已连接")).length).toBeGreaterThan(0);
    await user.click(screen.getByRole("switch", { name: /允许经中继连接/ }));
    await waitFor(() => {
      expect(backend.peek().settings.relay_enabled).toBe(false);
    });
    expect(screen.getAllByText("中继 · 未配置").length).toBeGreaterThan(0);
  });

  it("regression: the LAN discovery switch drives settings, and while it is on a waiting pairing says phones nearby can pick this computer", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    const { backend } = mount();
    await screen.findByRole("table", { name: "已配对设备" });
    expect(screen.getByRole("switch", { name: /局域网发现/ })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    expect(screen.getByText(/局域网发现在同一局域网里公布这台电脑的名称/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "开始配对" }));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });
    expect(screen.getByTestId("pairing-lan-note")).toHaveTextContent("「附近的电脑」");
    await user.click(screen.getByRole("switch", { name: /局域网发现/ }));
    await waitFor(() => {
      expect(backend.peek().settings.lan_discovery).toBe(false);
    });
    expect(screen.getByRole("switch", { name: /局域网发现/ })).toHaveAttribute(
      "aria-checked",
      "false",
    );
    expect(screen.queryByTestId("pairing-lan-note")).toBeNull();
  });

  it("regression: always-on pairing keeps the window open: the switch drives settings, the code renews before it lapses, the switch replaces 取消, and turning it off closes the window", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    const { backend } = mount();
    const panel = await screen.findByTestId("pairing-panel");
    expect(panel).toHaveAttribute("data-phase", "idle");
    await user.click(within(panel).getByRole("switch", { name: "常开配对" }));
    await waitFor(() => {
      expect(backend.peek().settings.pairing_always_on).toBe(true);
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });
    expect(panel).toHaveAttribute("data-phase", "waiting_for_peer");
    expect(within(panel).getByText(/^常开 · 本码剩余 /)).toBeInTheDocument();
    expect(within(panel).queryByRole("button", { name: "取消" })).toBeNull();
    const first = backend.peek().pairing.session_id;
    // 120 s session, renewed with 10 s left: a new code, never the expired screen.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(110_000);
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });
    expect(panel).toHaveAttribute("data-phase", "waiting_for_peer");
    expect(backend.peek().pairing.session_id).not.toBe(first);
    // A phone pairs; the safety code is still confirmed here, then the next window opens.
    act(() => {
      backend.simulatePeerJoined();
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(400);
    });
    await user.click(within(panel).getByRole("button", { name: "确认配对" }));
    act(() => {
      backend.simulatePeerConfirmed();
    });
    expect(panel).toHaveAttribute("data-phase", "trusted");
    expect(within(panel).getByText("常开配对：稍后自动开始下一次配对。")).toBeInTheDocument();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(4_300);
    });
    expect(panel).toHaveAttribute("data-phase", "waiting_for_peer");
    await user.click(within(panel).getByRole("switch", { name: "常开配对" }));
    await waitFor(() => {
      expect(panel).toHaveAttribute("data-phase", "idle");
    });
    expect(backend.peek().settings.pairing_always_on).toBe(false);
    expect(within(panel).queryByText("常开配对已打开，连上中继或局域网后自动开始。")).toBeNull();
  });

  it("sends a test message to an online device and forgets through the confirm dialog", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    const { backend } = mount();
    await screen.findByRole("table", { name: "已配对设备" });
    // One icon button per row; only the online device's is enabled.
    const sendButtons = screen.getAllByRole("button", { name: "发测试消息" });
    expect(sendButtons).toHaveLength(2);
    expect(sendButtons[1]).toBeDisabled();
    await user.click(sendButtons[0] as HTMLElement);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(200);
    });
    expect(await screen.findByText("Pixel 8：来自电脑的测试消息")).toBeInTheDocument();
    const forgetButtons = screen.getAllByRole("button", { name: "忘记" });
    await user.click(forgetButtons[1] as HTMLElement);
    expect(screen.getByRole("dialog", { name: "忘记「MacBook Pro」？" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "取消" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(backend.peek().devices).toHaveLength(2);
  });
});
