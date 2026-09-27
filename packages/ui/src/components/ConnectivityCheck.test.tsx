import { type ConnectivityReport, zhT } from "@voltip/shared";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ConnectivityCheck, blockedOnSubnet, peerText, probeText } from "./ConnectivityCheck";

const REPORT: ConnectivityReport = {
  checked_at: 1_758_700_600_000,
  lan: { listening: false, addresses: [] },
  relay: { configured: true, result: { result: "failed", reason: "" } },
  peers: [
    {
      public_key: "1".repeat(64),
      name: "Surface-Laptop",
      via: "relay",
      addresses: [
        { address: "192.168.1.24:47831", same_subnet: true, result: { result: "timeout" } },
        { address: "10.0.0.7:47831", same_subnet: false, result: { result: "refused" } },
      ],
    },
  ],
};

describe("ConnectivityCheck", () => {
  it("names every probe outcome and each device's channel", () => {
    const t = zhT.t;
    expect(probeText({ result: "ok", ms: 12 }, t)).toBe("可连接 · 12 ms");
    expect(probeText({ result: "timeout" }, t)).toBe("没有响应");
    expect(probeText({ result: "refused" }, t)).toBe("拒绝连接");
    expect(probeText({ result: "failed", reason: "tls" }, t)).toBe("连接失败 · tls");
    expect(probeText({ result: "failed", reason: "" }, t)).toBe("连接失败");
    const peer = { public_key: "1".repeat(64), name: "Pixel 8", addresses: [] };
    expect(peerText(peer, t, "zh-CN")).toBe("Pixel 8 · 离线");
    expect(peerText({ ...peer, via: "direct", rtt_ms: 9 }, t, "zh-CN")).toBe(
      "Pixel 8 · 直连 · 加密往返 9 ms",
    );
    expect(peerText({ ...peer, via: "relay" }, t, "zh-CN")).toBe(
      "Pixel 8 · 中继 · 加密通道没有回应",
    );
    expect(blockedOnSubnet(REPORT.peers[0]?.addresses ?? [])).toBe(true);
    expect(
      blockedOnSubnet([
        { address: "10.0.0.7:47831", same_subnet: false, result: { result: "refused" } },
      ]),
    ).toBe(false);
  });

  it("regression: a failed LAN address on the same subnet points at a firewall or Wi-Fi isolation; the built-in relay's failure names no host", async () => {
    const user = userEvent.setup();
    const onRun = vi.fn();
    render(<ConnectivityCheck status={{ running: false, report: REPORT }} onRun={onRun} />);
    const check = screen.getByTestId("connectivity");
    expect(within(check).getByTestId("connectivity-lan")).toHaveTextContent(
      "本机没有开启局域网监听",
    );
    expect(within(check).getByTestId("connectivity-relay")).toHaveTextContent(/^中继 · 连接失败$/);
    const peer = within(check).getByTestId("connectivity-peer");
    expect(peer).toHaveTextContent("Surface-Laptop · 中继 · 加密通道没有回应");
    expect(peer).toHaveTextContent("192.168.1.24:47831 · 没有响应");
    expect(peer).toHaveTextContent("10.0.0.7:47831 · 拒绝连接");
    expect(peer).toHaveTextContent("同一网段却连不上");
    await user.click(within(check).getByRole("button", { name: "重新自检" }));
    expect(onRun).toHaveBeenCalledTimes(1);
  });

  it("offers the first check without a report, and no second one while it runs", () => {
    const { rerender } = render(<ConnectivityCheck status={{ running: false }} onRun={() => {}} />);
    expect(screen.getByRole("button", { name: "开始自检" })).toBeEnabled();
    expect(screen.queryByRole("list")).toBeNull();
    rerender(<ConnectivityCheck status={{ running: true }} onRun={() => {}} />);
    expect(screen.getByRole("button", { name: "自检中…" })).toBeDisabled();
  });
});
