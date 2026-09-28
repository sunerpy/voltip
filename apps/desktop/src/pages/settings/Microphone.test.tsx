import { MOCK_AUDIO_DEVICES, MOCK_METER_INTERVAL_MS, MockBackend } from "@voltip/shared/mock";
import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MIC_TEST_MS } from "../../features/audio/useMicrophoneTest";
import { renderApp } from "../../test/render";

async function openPane() {
  const dialog = await screen.findByRole("dialog", { name: "设置" });
  const pane = await within(dialog).findByTestId("microphone-pane");
  await waitFor(() => {
    expect(within(pane).getByRole("combobox", { name: "输入设备" })).toBeEnabled();
  });
  return pane;
}

describe("Settings · 麦克风", () => {
  it("regression: the input device is chosen here and written through settings_set_microphone; 跟随系统默认 writes null (user feedback 2026-09-28)", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/settings/microphone" });
    const pane = await openPane();
    const menu = within(pane).getByRole("combobox", { name: "输入设备" });
    expect(
      within(menu)
        .getAllByRole("option")
        .map((o) => o.textContent),
    ).toEqual([
      `跟随系统默认（${MOCK_AUDIO_DEVICES[0]?.name ?? ""}）`,
      ...MOCK_AUDIO_DEVICES.map((d) => d.name),
    ]);
    expect(menu).toHaveValue("");
    await user.selectOptions(menu, "Realtek(R) Audio");
    await waitFor(() => {
      expect(backend.peek().settings.microphone).toBe("Realtek(R) Audio");
    });
    expect(menu).toHaveValue("Realtek(R) Audio");
    expect(pane).toHaveTextContent("48 kHz · 2 声道");
    await user.selectOptions(menu, "");
    await waitFor(() => {
      expect(backend.peek().settings.microphone).toBeNull();
    });
    // Nothing meters the microphone while the pane just sits there.
    expect(backend.activeMeters()).toBe(0);
  });

  it("测试麦克风 meters the chosen device and closes by itself after 15 s", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime.bind(vi) });
    try {
      const backend = new MockBackend({ settings: { microphone: "Realtek(R) Audio" } });
      const meter = vi.spyOn(backend, "meter");
      renderApp({ path: "/settings/microphone", backend });
      const pane = await openPane();
      await user.click(within(pane).getByTestId("microphone-test"));
      await waitFor(() => {
        expect(backend.activeMeters()).toBe(1);
      });
      expect(meter).toHaveBeenCalledWith("Realtek(R) Audio", expect.any(Function));
      act(() => {
        vi.advanceTimersByTime(MOCK_METER_INTERVAL_MS * 2 + 1);
      });
      expect(within(pane).getByTestId("microphone-strength")).toHaveTextContent(/-\d+\.\d dBFS/);
      expect(within(pane).getByTestId("microphone-test")).toHaveTextContent(/停止测试 · 1[45]/);
      act(() => {
        vi.advanceTimersByTime(MIC_TEST_MS + 500);
      });
      await waitFor(() => {
        expect(backend.activeMeters()).toBe(0);
      });
      expect(within(pane).getByTestId("microphone-test")).toHaveTextContent("测试麦克风");
    } finally {
      vi.useRealTimers();
    }
  });

  it("regression: an unplugged choice stays chosen, is listed as not connected and says takes use the default", async () => {
    renderApp({ path: "/settings/microphone", mock: { settings: { microphone: "Blue Yeti" } } });
    const pane = await openPane();
    const menu = within(pane).getByRole("combobox", { name: "输入设备" });
    expect(menu).toHaveValue("Blue Yeti");
    expect(within(menu).getByRole("option", { name: "Blue Yeti · 未连接" })).toBeInTheDocument();
    expect(pane).toHaveTextContent("所选麦克风未连接，听写会先用系统默认输入");
  });

  it("an empty device id is refused like the core does and the choice stays", async () => {
    const backend = new MockBackend({ settings: { microphone: "Realtek(R) Audio" } });
    renderApp({ path: "/settings/microphone", backend });
    await openPane();
    await act(async () => {
      await backend.invoke("settings_set_microphone", { device: " " });
    });
    expect(backend.peek().settings.microphone).toBe("Realtek(R) Audio");
    expect(await screen.findByText(/microphone: a device id of 1–1024 bytes/)).toBeInTheDocument();
  });
});
