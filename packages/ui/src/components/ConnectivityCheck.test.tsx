import { type ConnectivityReport, zhT } from "@voltip/shared";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ConnectivityCheck, peerText, probeText } from "./ConnectivityCheck";

const REPORT: ConnectivityReport = {
  checked_at: 1_758_700_600_000,
  relay: { configured: true, result: { result: "failed", reason: "", kind: "other" } },
  peers: [
    { public_key: "1".repeat(64), name: "Surface-Laptop", online: true },
    { public_key: "2".repeat(64), name: "iPad", online: false },
  ],
};

describe("ConnectivityCheck", () => {
  it("names every probe outcome and each device's channel", () => {
    const t = zhT.t;
    expect(probeText({ result: "ok", ms: 12 }, t)).toBe("可连接 · 12 ms");
    expect(probeText({ result: "timeout" }, t)).toBe("没有响应");
    expect(probeText({ result: "refused" }, t)).toBe("拒绝连接");
    expect(probeText({ result: "failed", reason: "tls", kind: "tls" }, t)).toBe("连接失败 · tls");
    expect(probeText({ result: "failed", reason: "", kind: "other" }, t)).toBe("连接失败");
    // The build's own relay's failure has no words (issue #193): its kind says what happened.
    expect(probeText({ result: "failed", reason: "", kind: "dns" }, t)).toBe(
      "连接失败 · 无法解析中继地址，请检查网络",
    );
    expect(probeText({ result: "failed", reason: "", kind: "not_relay" }, zhT.t)).toBe(
      "连接失败 · 该地址不是 Voltip 中继",
    );
    const peer = { public_key: "1".repeat(64), name: "Pixel 8", online: false };
    expect(peerText(peer, t)).toBe("Pixel 8 · 离线");
    expect(peerText({ ...peer, online: true, rtt_ms: 9 }, t)).toBe(
      "Pixel 8 · 在线 · 加密往返 9 ms",
    );
    expect(peerText({ ...peer, online: true }, t)).toBe("Pixel 8 · 在线 · 加密通道没有回应");
  });

  it("regression: the built-in relay's failure names no host, and each device has its own line", async () => {
    const user = userEvent.setup();
    const onRun = vi.fn();
    render(<ConnectivityCheck status={{ running: false, report: REPORT }} onRun={onRun} />);
    const check = screen.getByTestId("connectivity");
    expect(within(check).getByTestId("connectivity-relay")).toHaveTextContent(/^中继 · 连接失败$/);
    const peers = within(check).getAllByTestId("connectivity-peer");
    expect(peers.map((p) => p.textContent)).toEqual([
      "Surface-Laptop · 在线 · 加密通道没有回应",
      "iPad · 离线",
    ]);
    expect(within(check).queryByText(/局域网/)).toBeNull();
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
