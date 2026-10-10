import { MockBackend } from "@voltip/shared/mock";
import { screen } from "@testing-library/react";
import { renderApp } from "../test/render";

describe("配对电脑 (docs/pairing.md 「只走中继」)", () => {
  it("regression: the pairing page offers the QR code and the six digits only, and 本机 has no LAN switch", async () => {
    const backend = new MockBackend({ role: "phone" });
    const view = renderApp({ backend, initialScreen: "pair" });
    expect(
      await screen.findByText(/扫描它的二维码，或输入它显示的 6 位验证码/),
    ).toBeInTheDocument();
    expect(screen.queryByTestId("nearby")).toBeNull();
    expect(screen.queryByText(/附近的电脑|局域网/)).toBeNull();
    view.unmount();
    renderApp({ backend, initialScreen: "device" });
    expect(await screen.findByRole("button", { name: "配对电脑" })).toBeInTheDocument();
    expect(screen.queryByRole("switch", { name: "局域网发现" })).toBeNull();
    expect(screen.queryByTestId("lan-discovery")).toBeNull();
  });
});
