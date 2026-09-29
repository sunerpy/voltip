import { MOCK_PUBLIC_KEYS, MockBackend, sampleDevices } from "@voltip/shared/mock";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useMobileShell } from "./app/shell";
import { isPairingLink } from "./screens/PairDevice";
import { renderApp } from "./test/render";

describe("Mobile app flow", () => {
  it("regression: a computer that unpaired this phone is announced", async () => {
    const desktop = sampleDevices(1_758_700_000)[1]?.device;
    if (desktop === undefined) throw new Error("fixture has no desktop");
    const backend = new MockBackend({ role: "phone" });
    renderApp({ backend });
    await screen.findByRole("heading", { level: 1 });
    // The frame registers its event listener in an effect: let it run first.
    await act(async () => {
      await Promise.resolve();
    });
    act(() => {
      backend.publish({ type: "unpaired", ...desktop });
    });
    expect(
      await screen.findByText(`「${desktop.name}」解除了与这台设备的配对`),
    ).toBeInTheDocument();
  });

  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("walks Welcome → This Device → Pair (code) → Verify → Devices with the phone confirming first", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    const { backend } = renderApp();
    expect(await screen.findByRole("heading", { name: "Voltip" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "开始" }));
    expect(screen.getByRole("heading", { name: "本机" })).toBeInTheDocument();
    expect(screen.getByTestId("fingerprint")).toHaveTextContent("5B:0F:E2:91 · C3:7A:0D:44");
    expect(screen.getByText("Android")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "配对电脑" }));
    expect(screen.getByRole("heading", { name: "配对电脑" })).toBeInTheDocument();
    await user.click(screen.getByRole("radio", { name: "输入 6 位验证码" }));
    await user.type(screen.getByLabelText("六位配对码"), "483921");
    expect(await screen.findByText("正在加入配对…")).toBeInTheDocument();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });
    expect(screen.getByText(/正在建立加密连接/)).toBeInTheDocument();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(400);
    });
    expect(screen.getByRole("heading", { name: "核对安全码" })).toBeInTheDocument();
    expect(screen.getByText("Surface-Laptop")).toBeInTheDocument();
    expect(screen.getByRole("list", { name: "安全码" }).children).toHaveLength(4);
    await user.click(screen.getByRole("button", { name: "确认并信任" }));
    expect(screen.getByText("已确认 · 等待电脑确认…")).toBeInTheDocument();
    act(() => {
      backend.simulatePeerConfirmed();
    });
    expect(await screen.findByRole("heading", { name: "已配对设备" })).toBeInTheDocument();
    expect(screen.getByText("1 台已配对 · 1 在线")).toBeInTheDocument();
    const card = screen.getByTestId("device-card");
    expect(within(card).getByText("Windows")).toBeInTheDocument();
    expect(within(card).getAllByText("在线 · 直连")).toHaveLength(2);
    expect(within(card).getByText("直连")).toBeInTheDocument();
    for (const col of ["设备", "平台", "在线状态", "最近在线", "信任于", "连接方式"]) {
      expect(within(card).getByText(col)).toBeInTheDocument();
    }
    expect(backend.peek().pairing.state).toEqual({ state: "idle" });
    expect(await screen.findByText("已信任 · Surface-Laptop")).toBeInTheDocument();
  });

  it("regression: a wrong code shows the relay error under the cells and can be retried", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    renderApp({ mock: { role: "phone", expectedCode: "111222" }, initialScreen: "pair" });
    await user.click(await screen.findByRole("radio", { name: "输入 6 位验证码" }));
    await user.type(screen.getByLabelText("六位配对码"), "999999");
    expect(await screen.findByRole("alert")).toHaveTextContent("验证码不正确");
    await user.click(screen.getByRole("button", { name: "清除并重试" }));
    expect(screen.queryByRole("alert")).toBeNull();
    await user.type(screen.getByLabelText("六位配对码"), "11122");
    expect(screen.getByRole("button", { name: "加入" })).toBeDisabled();
    await user.type(screen.getByLabelText("六位配对码"), "2");
    expect(await screen.findByText("正在加入配对…")).toBeInTheDocument();
  });

  it("scan tab falls back to pasting a link without a scanner and validates it", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    renderApp({ initialScreen: "pair", scanner: undefined });
    expect(await screen.findByText("当前设备没有可用的相机")).toBeInTheDocument();
    const input = screen.getByLabelText("配对链接");
    await user.type(input, "https://evil.example");
    expect(screen.getByText("不是 Voltip 配对链接")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "加入" })).toBeDisabled();
    await user.clear(input);
    await user.type(input, "voltip://pair?v=1&s=abcd&t=0123");
    await user.click(screen.getByRole("button", { name: "加入" }));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(700);
    });
    expect(screen.getByRole("heading", { name: "核对安全码" })).toBeInTheDocument();
    expect(isPairingLink(" voltip://pair?x=1")).toBe(true);
    expect(isPairingLink("voltip://other")).toBe(false);
  });

  it("uses the barcode scanner when available, handles cancel and a foreign QR", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    const scan = vi.fn<() => Promise<string | undefined>>();
    scan
      .mockResolvedValueOnce(undefined)
      .mockResolvedValueOnce("https://not-voltip")
      .mockResolvedValueOnce("voltip://pair?v=1&s=1&t=2");
    renderApp({ initialScreen: "pair", scanner: { scan } });
    const button = await screen.findByRole("button", { name: "打开相机扫码" });
    await user.click(button);
    expect(await screen.findByText("已取消扫码或未授予相机权限")).toBeInTheDocument();
    await user.click(button);
    expect(await screen.findByText("通信数据无效")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "重试" }));
    await user.click(screen.getByRole("button", { name: "打开相机扫码" }));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(700);
    });
    expect(screen.getByRole("heading", { name: "核对安全码" })).toBeInTheDocument();
  });

  it("regression: rejecting on the phone destroys the session; back from verify cancels", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    const { backend } = renderApp({ initialScreen: "pair", scanner: undefined });
    await screen.findByRole("heading", { name: "配对电脑" });
    await act(async () => {
      await backend.invoke("pairing_join_code", { code: "483921" });
      await vi.advanceTimersByTimeAsync(700);
    });
    expect(await screen.findByRole("heading", { name: "核对安全码" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "拒绝" }));
    expect(screen.getByText("配对已被拒绝，本次配对已取消。")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "重新配对" }));
    expect(screen.getByRole("heading", { name: "配对电脑" })).toBeInTheDocument();
    expect(backend.peek().pairing.state).toEqual({ state: "idle" });
    await act(async () => {
      await backend.invoke("pairing_join_code", { code: "483921" });
      await vi.advanceTimersByTimeAsync(700);
    });
    await user.click(screen.getByRole("button", { name: "返回" }));
    expect(backend.peek().pairing.state).toEqual({
      state: "failed",
      reason: { kind: "cancelled" },
    });
    expect(screen.getByRole("heading", { name: "配对电脑" })).toBeInTheDocument();
    act(() => {
      backend.simulatePeerRejected();
    });
    expect(await screen.findByRole("alert")).toHaveTextContent("对方拒绝了配对");
  });

  it("devices screen lists paired computers, sends a message, flags identity change and forgets", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    const now = Math.floor(Date.now() / 1000);
    const desktop = sampleDevices(now)[1];
    if (!desktop) throw new Error("fixture");
    const online = { ...desktop, connection: { state: "online" as const, via: "relay" as const } };
    const backend = new MockBackend({ role: "phone", devices: [online] });
    renderApp({ backend });
    expect(await screen.findByRole("heading", { name: "已配对设备" })).toBeInTheDocument();
    expect(screen.getAllByText("MacBook Pro").length).toBeGreaterThan(0);
    expect(screen.getAllByText("在线 · 经中继")).toHaveLength(2);
    await user.click(screen.getByRole("button", { name: "发测试消息" }));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(200);
    });
    expect(await screen.findByText("收到消息：来自手机的测试消息")).toBeInTheDocument();
    act(() => {
      backend.simulateIdentityChanged(MOCK_PUBLIC_KEYS.laptop, "00:11:22:33 · 44:55:66:77");
    });
    expect(
      await screen.findByText(/出示了不同的身份密钥（00:11:22:33 · 44:55:66:77）/),
    ).toBeInTheDocument();
    expect(screen.getAllByText("身份已变化")).toHaveLength(2);
    await user.click(screen.getByRole("button", { name: "忘记 MacBook Pro" }));
    await user.click(screen.getByRole("button", { name: "取消" }));
    await user.click(screen.getByRole("button", { name: "忘记 MacBook Pro" }));
    await user.click(within(screen.getByRole("dialog")).getByRole("button", { name: "忘记" }));
    expect(await screen.findByText("尚未配对电脑")).toBeInTheDocument();
    expect(await screen.findByText("已忘记 MacBook Pro")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "配对新电脑" }));
    expect(screen.getByRole("heading", { name: "配对电脑" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "返回" }));
    expect(screen.getByRole("heading", { name: "本机" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "查看已配对设备" }));
    await user.click(screen.getByRole("button", { name: "本机" }));
    expect(screen.getByRole("heading", { name: "本机" })).toBeInTheDocument();
  });

  it("renames this device, surfaces backend errors as toasts and applies the theme", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    const { backend } = renderApp({
      initialScreen: "device",
      mock: { role: "phone", settings: { theme: "graphite" } },
    });
    await screen.findByTestId("fingerprint");
    await waitFor(() => {
      expect(document.documentElement.dataset.theme).toBe("graphite");
    });
    await user.click(screen.getByRole("button", { name: "重命名" }));
    const input = screen.getByLabelText("名称");
    await user.clear(input);
    await user.type(input, "Pixel 8 Pro");
    await user.click(screen.getByRole("button", { name: "保存" }));
    await waitFor(() => {
      expect(backend.peek().identity?.name).toBe("Pixel 8 Pro");
    });
    expect(await screen.findByText("名称已更新")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "重命名" }));
    await user.click(screen.getByRole("button", { name: "取消" }));
    act(() => {
      backend.simulateError("relay unreachable");
    });
    expect(await screen.findByRole("alert")).toHaveTextContent("出错了 · relay unreachable");
    await user.click(screen.getByRole("button", { name: "查看已配对设备" }));
    expect(screen.getByRole("heading", { name: "已配对设备" })).toBeInTheDocument();
  });

  it("mounts straight onto the verify screen when the handshake finished before core_state loaded", async () => {
    const backend = new MockBackend({ role: "phone" });
    await backend.invoke("pairing_join_code", { code: "483921" });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(700);
    });
    renderApp({ backend });
    expect(await screen.findByRole("heading", { name: "核对安全码" })).toBeInTheDocument();
  });

  it("shows a splash before core_state resolves, an error when it fails, and guards the shell hook", async () => {
    const inner = new MockBackend({ role: "phone" });
    const backend = {
      getState: () => new Promise<Awaited<ReturnType<MockBackend["getState"]>>>(() => undefined),
      invoke: inner.invoke.bind(inner),
      on: inner.on.bind(inner),
      audioDevices: inner.audioDevices.bind(inner),
      meter: inner.meter.bind(inner),
      updateStatus: inner.updateStatus.bind(inner),
      vocabularyPreview: inner.vocabularyPreview.bind(inner),
      rulesExport: inner.rulesExport.bind(inner),
      recentApps: inner.recentApps.bind(inner),
      permissionsStatus: inner.permissionsStatus.bind(inner),
      permissionsRequest: inner.permissionsRequest.bind(inner),
      injectPreflight: inner.injectPreflight.bind(inner),
      pasteText: inner.pasteText.bind(inner),
      providerConsoleOpen: inner.providerConsoleOpen.bind(inner),
      projectLinkOpen: inner.projectLinkOpen.bind(inner),
      feedbackDiagnostics: inner.feedbackDiagnostics.bind(inner),
      feedbackSubmit: inner.feedbackSubmit.bind(inner),
      feedbackAttachmentAdd: inner.feedbackAttachmentAdd.bind(inner),
      feedbackAttachmentRemove: inner.feedbackAttachmentRemove.bind(inner),
      feedbackAttachmentsClear: inner.feedbackAttachmentsClear.bind(inner),
      phoneClipboardRead: inner.phoneClipboardRead.bind(inner),
      presetsBuiltin: inner.presetsBuiltin.bind(inner),
      scenesBuiltin: inner.scenesBuiltin.bind(inner),
    };
    const first = render(<TestApp backend={backend} />);
    expect(await screen.findByText("正在启动…")).toBeInTheDocument();
    first.unmount();
    render(
      <TestApp backend={{ ...backend, getState: () => Promise.reject(new Error("core down")) }} />,
    );
    expect(await screen.findByText("启动失败：core down")).toBeInTheDocument();
    const identityless = new MockBackend({ role: "phone" });
    const noIdentity = {
      getState: async () => ({ ...(await identityless.getState()), identity: null }),
      invoke: identityless.invoke.bind(identityless),
      on: identityless.on.bind(identityless),
      audioDevices: identityless.audioDevices.bind(identityless),
      meter: identityless.meter.bind(identityless),
      updateStatus: identityless.updateStatus.bind(identityless),
      vocabularyPreview: identityless.vocabularyPreview.bind(identityless),
      rulesExport: identityless.rulesExport.bind(identityless),
      recentApps: identityless.recentApps.bind(identityless),
      permissionsStatus: identityless.permissionsStatus.bind(identityless),
      permissionsRequest: identityless.permissionsRequest.bind(identityless),
      injectPreflight: identityless.injectPreflight.bind(identityless),
      pasteText: identityless.pasteText.bind(identityless),
      providerConsoleOpen: identityless.providerConsoleOpen.bind(identityless),
      projectLinkOpen: identityless.projectLinkOpen.bind(identityless),
      feedbackDiagnostics: identityless.feedbackDiagnostics.bind(identityless),
      feedbackSubmit: identityless.feedbackSubmit.bind(identityless),
      feedbackAttachmentAdd: identityless.feedbackAttachmentAdd.bind(identityless),
      feedbackAttachmentRemove: identityless.feedbackAttachmentRemove.bind(identityless),
      feedbackAttachmentsClear: identityless.feedbackAttachmentsClear.bind(identityless),
      phoneClipboardRead: identityless.phoneClipboardRead.bind(identityless),
      presetsBuiltin: identityless.presetsBuiltin.bind(identityless),
      scenesBuiltin: identityless.scenesBuiltin.bind(identityless),
    };
    render(<TestApp backend={noIdentity} />);
    expect(await screen.findByText("正在生成设备身份…")).toBeInTheDocument();
    const Probe = () => {
      useMobileShell();
      return null;
    };
    expect(() => render(<Probe />)).toThrow("useMobileShell must be used inside <App>");
  });
});

import { App } from "./App";
import type { Backend } from "@voltip/shared";

function TestApp({ backend }: { backend: Backend }) {
  return (
    <App
      backend={backend}
      loadScanner={() => Promise.reject(new Error("no camera"))}
      initialScreen="device"
      systemLanguage="zh-CN"
    />
  );
}

describe("locale", () => {
  const CJK = /[一-鿿]/;

  it("regression: every phone screen renders in English for settings.locale = en, and follows the OS for system", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({
      mock: { settings: { locale: "en" }, devices: sampleDevices(1_758_700_000) },
      initialScreen: "welcome",
    });
    expect(await screen.findByRole("button", { name: "Get started" })).toBeInTheDocument();
    // `lang` is set by an effect after the commit that shows the button: wait for it (a single read
    // raced it under the coverage run's load, 2026-09-26).
    await waitFor(() => expect(document.documentElement.lang).toBe("en-US"));
    expect(document.body.textContent).not.toMatch(CJK);
    await user.click(screen.getByRole("button", { name: "Get started" }));
    expect(screen.getByRole("heading", { name: "This device", level: 1 })).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(CJK);
    await user.click(screen.getByRole("button", { name: "Pair a computer" }));
    expect(screen.getByRole("heading", { name: "Pair a computer", level: 1 })).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(CJK);
    await user.click(screen.getByRole("button", { name: "Back" }));
    await user.click(screen.getByRole("button", { name: "View paired devices" }));
    expect(screen.getByRole("heading", { name: "Paired devices", level: 1 })).toBeInTheDocument();
    expect(screen.getByText(/2 paired · \d online/)).toBeInTheDocument();
    expect(document.body.textContent).not.toMatch(CJK);
    // The core's locale is shared: switching it back to Chinese re-renders the phone too.
    act(() => {
      backend.publish({ type: "settings", ...backend.peek().settings, locale: "zh-cn" });
    });
    expect(
      await screen.findByRole("heading", { name: "已配对设备", level: 1 }),
    ).toBeInTheDocument();
    await waitFor(() => expect(document.documentElement.lang).toBe("zh-CN"));
  });

  it("regression: under zh-CN no phone screen shows an ASCII-caps eyebrow or an English detail label or a raw relay state", async () => {
    // User 2026-09-25: one language per locale — no `THIS DEVICE` / `PEER` / `SAFETY CODE` eyebrows,
    // no `Last Seen` column names and no `relay disconnected` next to Chinese copy.
    const user = userEvent.setup();
    renderApp({ mock: { devices: sampleDevices(1_758_700_000) }, initialScreen: "device" });
    expect(await screen.findByRole("heading", { name: "本机", level: 1 })).toBeInTheDocument();
    const offending: string[] = [];
    const sweep = (screenName: string) => {
      for (const el of document.querySelectorAll('[class~="eyebrow"]')) {
        const text = (el.textContent ?? "").trim();
        if (/^[A-Z][A-Z0-9 &·/'-]{2,}$/.test(text)) offending.push(`${screenName}: ${text}`);
      }
    };
    sweep("device");
    // The card carries no eyebrow: the screen title already says 本机.
    expect(screen.getAllByText("本机")).toHaveLength(1);
    expect(screen.getByText(/^公钥 /)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "查看已配对设备" }));
    expect(screen.getByRole("heading", { name: "已配对设备", level: 1 })).toBeInTheDocument();
    sweep("devices");
    expect(screen.getByText("中继 · 未配置")).toBeInTheDocument();
    for (const col of ["设备", "平台", "在线状态", "最近在线", "信任于", "连接方式"]) {
      expect(screen.getAllByText(col).length).toBeGreaterThan(0);
    }
    expect(screen.queryByText(/Last Seen|Connection Type|^relay /)).toBeNull();
    expect(offending).toEqual([]);
  });

  it("system locale follows the phone's language", async () => {
    renderApp({ initialScreen: "welcome", systemLanguage: "en-US" });
    expect(await screen.findByRole("button", { name: "Get started" })).toBeInTheDocument();
  });
});
