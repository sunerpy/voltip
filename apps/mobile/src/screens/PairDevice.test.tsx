import type { NearbyDevice } from "@voltip/shared";
import { MOCK_NEARBY, MockBackend } from "@voltip/shared/mock";
import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../test/render";
import { nearbyComputers } from "./PairDevice";

function device(patch: Partial<NearbyDevice>): NearbyDevice {
  return {
    fingerprint: "0",
    name: "x",
    platform: "windows",
    pairing: false,
    trusted: false,
    ...patch,
  };
}

describe("附近的电脑 (docs/pairing.md 「局域网发现」)", () => {
  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("regression: a computer waiting nearby pairs with one tap, and the safety code is still compared", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    const backend = new MockBackend({ role: "phone" });
    const invoke = vi.spyOn(backend, "invoke");
    renderApp({ backend, initialScreen: "pair" });
    const card = await screen.findByTestId("nearby");
    const studio = within(card).getByTestId("nearby-computer");
    expect(studio).toHaveTextContent("Studio");
    expect(studio).toHaveTextContent("macOS");
    await user.click(within(card).getByRole("button", { name: "与 Studio 配对" }));
    expect(invoke).toHaveBeenCalledWith("pairing_join_nearby", {
      fingerprint: MOCK_NEARBY[0]?.fingerprint,
    });
    expect(await screen.findByText("正在加入配对…")).toBeInTheDocument();
    expect(within(card).getByRole("button", { name: "与 Studio 配对" })).toBeDisabled();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(700);
    });
    expect(screen.getByRole("heading", { name: "核对安全码" })).toBeInTheDocument();
  });

  it("lists computers that are not paired yet, waiting ones first; a phone or a paired computer is left out", () => {
    const list = nearbyComputers([
      device({ fingerprint: "1", name: "Beta" }),
      device({ fingerprint: "2", name: "Pixel", platform: "android", pairing: true }),
      device({ fingerprint: "3", name: "Alpha" }),
      device({ fingerprint: "4", name: "Studio", platform: "macos", pairing: true }),
      device({ fingerprint: "5", name: "Paired", pairing: true, trusted: true }),
    ]);
    expect(list.map((d) => d.name)).toEqual(["Studio", "Alpha", "Beta"]);
  });

  it("a computer that is not pairing says so and offers no button; none nearby says how to start", async () => {
    const backend = new MockBackend({ role: "phone" });
    renderApp({ backend, initialScreen: "pair" });
    const card = await screen.findByTestId("nearby");
    act(() => {
      backend.publish({ type: "nearby", devices: [device({ name: "Office PC" })] });
    });
    expect(await within(card).findByText(/尚未开始配对/)).toBeInTheDocument();
    expect(within(card).queryByRole("button")).toBeNull();
    act(() => {
      backend.publish({ type: "nearby", devices: [] });
    });
    expect(await within(card).findByText("正在查找同一局域网里的电脑…")).toBeInTheDocument();
    expect(card).toHaveTextContent("点「开始配对」");
  });

  it("regression: the switch on 本机 turns discovery off and on; while off the card says where to turn it on", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    const backend = new MockBackend({ role: "phone" });
    renderApp({ backend, initialScreen: "device" });
    const toggle = await screen.findByRole("switch", { name: "局域网发现" });
    expect(toggle).toHaveAttribute("aria-checked", "true");
    await user.click(toggle);
    await waitFor(() => {
      expect(backend.peek().settings.lan_discovery).toBe(false);
    });
    expect(backend.peek().nearby).toEqual([]);
    await user.click(screen.getByRole("button", { name: "配对电脑" }));
    const card = await screen.findByTestId("nearby");
    expect(card).toHaveTextContent("局域网发现已关闭");
    expect(within(card).queryByTestId("nearby-computer")).toBeNull();
  });
});
