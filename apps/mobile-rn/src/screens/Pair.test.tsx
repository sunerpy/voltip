// Pairing on the phone (docs/pairing.md): the camera for the computer's QR code, the link pasted
// when there is no camera, the page while the pairing runs or failed, 核对安全码 and where each
// outcome leads, and the toasts for what the core reports about paired devices.
import { act, fireEvent, screen, waitFor } from "@testing-library/react-native";
import { type Snapshot, idleSnapshot } from "@voltip/shared";
import { MockBackend } from "@voltip/shared/mock";

import { hold, nth, renderApp } from "../test/render";

interface MockCamera {
  permission: { granted: boolean; canAskAgain: boolean } | null;
  ask: jest.Mock;
  scanned?: ((result: { data: string }) => void) | undefined;
}

const mockCamera: MockCamera = { permission: null, ask: jest.fn() };

jest.mock("expo-camera", () => ({
  CameraView: (props: { onBarcodeScanned: (result: { data: string }) => void }) => {
    mockCamera.scanned = props.onBarcodeScanned;
    return null;
  },
  useCameraPermissions: () => [mockCamera.permission, mockCamera.ask],
}));

beforeEach(() => {
  mockCamera.permission = { granted: true, canAskAgain: true };
  mockCamera.ask = jest.fn(() => Promise.resolve(mockCamera.permission));
  mockCamera.scanned = undefined;
});

const LINK = "voltip://pair?v=1&s=abc&t=def";
const PEER = {
  device_id: "00000000-0000-4000-8000-000000000001",
  name: "Surface-Laptop",
  platform: "windows" as const,
};

async function openPair(backend = new MockBackend({ role: "phone" })) {
  await renderApp({ backend });
  await fireEvent.press(await screen.findByTestId("welcome-start"));
  await fireEvent.press(await screen.findByText("配对电脑"));
  expect(await screen.findByTestId("phone-pair")).toBeOnTheScreen();
  return { backend, invoke: jest.spyOn(backend, "invoke") };
}

async function publish(backend: MockBackend, snapshot: Partial<Snapshot>) {
  await act(async () => {
    backend.publish({ type: "pairing", ...idleSnapshot(), ...snapshot });
    await Promise.resolve();
  });
}

describe("扫码", () => {
  it("joins with the first Voltip link the camera sees, ignoring anything else, and goes back", async () => {
    const { backend } = await openPair();
    // The core takes the link and pairs nothing here: the page it goes back to stays put.
    const invoke = hold(backend, "pairing_join_ticket");
    await fireEvent.press(screen.getByTestId("pair-open-camera"));
    expect(await screen.findByText("对准电脑屏幕上的二维码")).toBeOnTheScreen();
    expect(mockCamera.scanned).toBeDefined();
    await act(async () => {
      mockCamera.scanned?.({ data: "https://example.test/" });
      await Promise.resolve();
    });
    expect(invoke).not.toHaveBeenCalledWith("pairing_join_ticket", expect.anything());
    const scanned = mockCamera.scanned;
    await act(async () => {
      scanned?.({ data: `  ${LINK} ` });
      scanned?.({ data: `${LINK}&again` });
      await Promise.resolve();
    });
    expect(invoke).toHaveBeenCalledWith("pairing_join_ticket", { uri: LINK });
    expect(invoke.mock.calls.filter(([name]) => name === "pairing_join_ticket")).toHaveLength(1);
    expect(await screen.findByTestId("phone-pair")).toBeOnTheScreen();
  });

  it("closes with the cross, and asks for the camera when it may", async () => {
    mockCamera.permission = { granted: false, canAskAgain: true };
    await openPair();
    await fireEvent.press(screen.getByTestId("pair-open-camera"));
    await waitFor(() => {
      expect(mockCamera.ask).toHaveBeenCalled();
    });
    // Without the camera the frame stays, over a note and the way back.
    expect(await screen.findByText("当前设备没有可用的相机")).toBeOnTheScreen();
    await fireEvent.press(screen.getByTestId("scanner-close"));
    expect(await screen.findByTestId("phone-pair")).toBeOnTheScreen();
    await fireEvent.press(screen.getByTestId("pair-open-camera"));
    await fireEvent.press(await screen.findByText("返回"));
    expect(await screen.findByText("已取消扫码或未授予相机权限")).toBeOnTheScreen();
    expect(await screen.findByTestId("phone-pair")).toBeOnTheScreen();
  });

  it("does not ask while the permission is unknown", async () => {
    mockCamera.permission = null;
    await openPair();
    await fireEvent.press(screen.getByTestId("pair-open-camera"));
    expect(await screen.findByText("当前设备没有可用的相机")).toBeOnTheScreen();
    expect(mockCamera.ask).not.toHaveBeenCalled();
  });

  it("takes the computer's link pasted when the camera was refused for good", async () => {
    mockCamera.permission = { granted: false, canAskAgain: false };
    const { invoke } = await openPair();
    expect(screen.queryByTestId("pair-open-camera")).toBeNull();
    expect(
      screen.getByText("粘贴电脑上「复制链接」得到的 voltip://pair?… 链接。"),
    ).toBeOnTheScreen();
    const field = screen.getByTestId("pair-link");
    await fireEvent.changeText(field, "https://example.test/");
    expect(screen.getByText("不是 Voltip 配对链接")).toBeOnTheScreen();
    expect(screen.getByText("加入")).toBeDisabled();
    await fireEvent.changeText(field, ` ${LINK}`);
    expect(screen.queryByText("不是 Voltip 配对链接")).toBeNull();
    await fireEvent.press(screen.getByText("加入"));
    expect(invoke).toHaveBeenCalledWith("pairing_join_ticket", { uri: LINK });
  });
});

describe("the pairing page while it runs", () => {
  it("says it is joining, then negotiating, and holds the camera until then", async () => {
    const { backend } = await openPair();
    await publish(backend, { state: { state: "creating_session" } });
    expect(await screen.findByText("正在加入配对…")).toBeOnTheScreen();
    expect(screen.getByTestId("pair-open-camera")).toBeDisabled();
    await publish(backend, { state: { state: "key_exchange" } });
    expect(await screen.findByText("正在建立加密连接…")).toBeOnTheScreen();
  });

  it("says why a pairing failed, ran out or was refused, and starts over", async () => {
    const { backend, invoke } = await openPair();
    await publish(backend, {
      state: { state: "failed", reason: { kind: "relay", code: "invalid_code" } },
    });
    expect(await screen.findByText("验证码不正确")).toBeOnTheScreen();
    await fireEvent.press(screen.getByText("重试"));
    expect(invoke).toHaveBeenCalledWith("pairing_reset");
    await publish(backend, { state: { state: "expired" } });
    expect(await screen.findByText("配对已过期")).toBeOnTheScreen();
    await publish(backend, { state: { state: "rejected" } });
    expect(await screen.findByText("对方拒绝了配对")).toBeOnTheScreen();
    // With the code: the reason under the field, and 清除并重试 empties it.
    await fireEvent.press(screen.getByTestId("pair-method-code"));
    await fireEvent.changeText(await screen.findByTestId("pair-code-input"), "12");
    invoke.mockClear();
    await fireEvent.press(screen.getByText("清除并重试"));
    expect(invoke).toHaveBeenCalledWith("pairing_reset");
    expect(screen.getByTestId("pair-code-input")).toHaveDisplayValue("");
  });
});

describe("核对安全码", () => {
  const code = {
    words: ["amber", "canyon", "lantern", "orbit"] as [string, string, string, string],
    fingerprint: "5B:0F",
  };

  it("shows the computer and the code, says when the computer confirmed, and rejects", async () => {
    const { backend, invoke } = await openPair();
    await publish(backend, {
      state: { state: "awaiting_verification" },
      safety_code: code,
      peer: PEER,
      peer_confirmed: true,
    });
    expect(await screen.findByTestId("phone-verify")).toBeOnTheScreen();
    expect(screen.getByText("Surface-Laptop")).toBeOnTheScreen();
    expect(screen.getByText("Windows · 已建立加密连接")).toBeOnTheScreen();
    expect(screen.getByText("对方已确认")).toBeOnTheScreen();
    await fireEvent.press(screen.getByText("拒绝"));
    expect(invoke).toHaveBeenCalledWith("pairing_reject");
    await publish(backend, { state: { state: "rejected" } });
    expect(await screen.findByText("配对已被拒绝，本次配对已取消。")).toBeOnTheScreen();
    await fireEvent.press(screen.getByText("重新配对"));
    expect(invoke).toHaveBeenCalledWith("pairing_reset");
    expect(await screen.findByTestId("phone-pair")).toBeOnTheScreen();
  });

  it("names an unknown computer plainly, cancels the pairing on back, and says a failed or vanished one is over", async () => {
    const { backend, invoke } = await openPair();
    await publish(backend, { state: { state: "awaiting_verification" } });
    expect(await screen.findByTestId("phone-verify")).toBeOnTheScreen();
    expect(screen.getByText("电脑")).toBeOnTheScreen();
    expect(screen.getByText("— · 已建立加密连接")).toBeOnTheScreen();
    await fireEvent.press(nth(screen.getAllByLabelText("返回"), 0));
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("pairing_cancel");
    });
    expect(await screen.findByTestId("phone-pair")).toBeOnTheScreen();

    await publish(backend, { state: { state: "awaiting_verification" } });
    expect(await screen.findByTestId("phone-verify")).toBeOnTheScreen();
    await publish(backend, { state: { state: "failed", reason: { kind: "timeout" } } });
    expect(await screen.findByText("配对失败，本次配对已取消。")).toBeOnTheScreen();
    await publish(backend, { state: { state: "idle" } });
    expect(await screen.findByText("没有进行中的配对。")).toBeOnTheScreen();
  });

  it("goes back to 说话 once trusted, from 核对安全码 or from anywhere else", async () => {
    const { backend, invoke } = await openPair();
    await publish(backend, { state: { state: "awaiting_verification" }, peer: PEER });
    expect(await screen.findByTestId("phone-verify")).toBeOnTheScreen();
    await publish(backend, { state: { state: "trusted" }, peer: PEER });
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("pairing_reset");
    });
    expect(await screen.findByTestId("tab-dock")).toBeOnTheScreen();
    invoke.mockClear();
    await publish(backend, { state: { state: "trusted" }, peer: PEER });
    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("pairing_reset");
    });
  });
});

describe("what the core reports about paired devices", () => {
  const device = {
    device_id: PEER.device_id,
    name: "Studio",
    platform: "windows" as const,
    public_key: "1".repeat(64),
    fingerprint: "AB:CD",
    trusted_at: 1_758_700_000,
    last_seen: 1_758_700_600,
    sync: true,
    sync_gen: 0,
  };

  // One event a test: each says so in a toast, and toasts wait their turn.
  it.each([
    ["a device was trusted", { type: "trusted", ...device }, "已信任 · Studio"],
    [
      "a device unpaired this phone",
      { type: "unpaired", ...device },
      "「Studio」解除了与这台设备的配对",
    ],
    [
      "a message arrived",
      { type: "message", from: device.public_key, body: "你好" },
      "收到消息：你好",
    ],
    [
      "a device changed its identity",
      { type: "identity_changed", previous: device, presented_fingerprint: "99:88" },
      "Studio 的身份密钥已变化，未自动信任",
    ],
  ] as const)("says %s", async (_what, event, text) => {
    const { backend } = await renderApp();
    await screen.findByTestId("tab-dock");
    await act(async () => {
      backend.publish(event);
      await Promise.resolve();
    });
    expect(await screen.findByText(text)).toBeOnTheScreen();
  });
});
