// 说话 once a computer is paired (docs/pairing.md): each computer's card with its state, its copy and
// its actions, the connection self-check, and the built-in polish service's notice.
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react-native";
import { type PeerCheck, type ProbeResult, zhT } from "@voltip/shared";
import { MockBackend, mockMirrorProfile } from "@voltip/shared/mock";

import { renderApp } from "../test/render";
import { peerText, probeText } from "./Devices";

const KEY = "1".repeat(64);

function computer(state: "online" | "offline" | "connecting") {
  return {
    device: {
      device_id: "0f3f1a1e-8d4b-4c8e-9f7a-1c2d3e4f5a6b",
      public_key: KEY,
      name: "Studio",
      platform: "windows",
      fingerprint: "AB:CD:EF",
      trusted_at: 1_758_700_000,
      last_seen: 1_758_700_600,
    },
    connection: { state },
  } as never;
}

describe("a paired computer's card", () => {
  it("shows the computer's state, its copy on the phone, and sends a test message", async () => {
    const { backend } = await renderApp({
      mock: {
        devices: [computer("online")],
        mirrors: [
          {
            view: { desktop: KEY, name: "Studio", state: "syncing", entries: 3 },
            history: [],
            profile: mockMirrorProfile(),
          },
        ],
      },
    });
    const card = within(await screen.findByTestId("device-card"));
    expect(card.getByLabelText("Studio 详情")).toHaveTextContent(/Windows/);
    expect(card.getByTestId("device-sync")).toHaveTextContent("正在同步 · 已收到 3 条");
    expect(card.getByText("AB:CD:EF")).toBeOnTheScreen();
    const invoke = jest.spyOn(backend, "invoke");
    await fireEvent.press(card.getByText("发测试消息"));
    expect(invoke).toHaveBeenCalledWith("send_text", {
      publicKey: KEY,
      body: "来自手机的测试消息",
    });
  });

  it("offers no test while connecting, warns of a changed identity, and forgets the computer after asking", async () => {
    const { backend } = await renderApp({ mock: { devices: [computer("connecting")] } });
    const card = within(await screen.findByTestId("device-card"));
    expect(card.queryByText("发测试消息")).toBeNull();
    await act(async () => {
      backend.simulateIdentityChanged(KEY, "99:88:77");
      await Promise.resolve();
    });
    expect(
      await card.findByText(
        "这台电脑出示了不同的身份密钥（99:88:77），未自动信任；如果重装了系统，请忘记后重新配对。",
      ),
    ).toBeOnTheScreen();
    await fireEvent.press(card.getByLabelText("忘记 Studio"));
    expect(await screen.findByText("忘记「Studio」？")).toBeOnTheScreen();
    await fireEvent.press(screen.getByTestId("shell-confirm"));
    expect(await screen.findByText("已忘记 Studio")).toBeOnTheScreen();
    await waitFor(() => {
      expect(screen.queryByTestId("device-card")).toBeNull();
    });
    expect(backend.peek().devices).toEqual([]);
  });
});

describe("连接自检", () => {
  it("runs the check and lists the relay and each computer", async () => {
    const backend = new MockBackend({
      role: "phone",
      devices: [computer("online")],
      relay: { state: "connected", attempts: 0, source: "builtin" },
    });
    await renderApp({ backend });
    const check = within(await screen.findByTestId("connectivity"));
    await fireEvent.press(check.getByText("开始自检"));
    expect(await check.findByText("中继 · 可连接 · 48 ms")).toBeOnTheScreen();
    expect(check.getByText("Studio · 在线 · 加密往返 61 ms")).toBeOnTheScreen();
    expect(check.getByText(/^检查于 /)).toBeOnTheScreen();
    expect(check.getByText("重新自检")).toBeOnTheScreen();
  });

  it("says when no relay is set up, and that an offline computer is offline", async () => {
    await renderApp({ mock: { devices: [computer("offline")] } });
    const check = within(await screen.findByTestId("connectivity"));
    await fireEvent.press(check.getByText("开始自检"));
    expect(await check.findByText("未配置中继")).toBeOnTheScreen();
    expect(check.getByText("Studio · 离线")).toBeOnTheScreen();
  });

  it("words every probe result and every peer", () => {
    const { t } = zhT;
    const probe = (result: ProbeResult) => probeText(result, t);
    expect(probe({ result: "ok", ms: 12 })).toBe("可连接 · 12 ms");
    expect(probe({ result: "timeout" })).toBe("没有响应");
    expect(probe({ result: "refused" })).toBe("拒绝连接");
    expect(probe({ result: "failed", reason: "tls handshake", kind: "tls" })).toBe(
      "连接失败 · tls handshake",
    );
    // The build's own relay names no host: its kind speaks.
    expect(probe({ result: "failed", reason: "", kind: "dns" })).toBe(
      "连接失败 · 无法解析中继地址，请检查网络",
    );
    expect(probe({ result: "failed", reason: "", kind: "other" })).toBe("连接失败");
    const peer = (p: Omit<PeerCheck, "public_key">) => peerText({ public_key: KEY, ...p }, t);
    expect(peer({ name: "Studio", online: false })).toBe("Studio · 离线");
    expect(peer({ name: "Studio", online: true })).toBe("Studio · 在线 · 加密通道没有回应");
    expect(peer({ name: "Studio", online: true, rtt_ms: 7 })).toBe("Studio · 在线 · 加密往返 7 ms");
  });
});

describe("the built-in polish service's notice", () => {
  it("says the service is busy or out of quota, opens AI 模型, and closes for a day", async () => {
    const { backend } = await renderApp({ mock: { devices: [computer("online")] } });
    await screen.findByTestId("device-card");
    expect(screen.queryByTestId("refine-notice")).toBeNull();
    await act(async () => {
      backend.simulateRefineNotice("rate_limited");
      await Promise.resolve();
    });
    const notice = within(await screen.findByTestId("refine-notice"));
    expect(notice.getByText("内置 AI 润色服务当前繁忙")).toBeOnTheScreen();
    await act(async () => {
      backend.simulateRefineNotice("quota");
      await Promise.resolve();
    });
    expect(await notice.findByText("内置 AI 润色服务的额度已用完")).toBeOnTheScreen();
    const invoke = jest.spyOn(backend, "invoke");
    await fireEvent.press(notice.getByText("关闭"));
    expect(invoke).toHaveBeenCalledWith("refine_notice_close");
    await act(async () => {
      backend.simulateRefineNotice("quota");
      await Promise.resolve();
    });
    await fireEvent.press(
      within(await screen.findByTestId("refine-notice")).getByText("打开 AI 模型"),
    );
    expect(await screen.findByTestId("phone-ai")).toBeOnTheScreen();
  });
});
