import {
  MOCK_AUDIO_DEVICES,
  MOCK_AUDIO_OUTPUTS,
  MOCK_METER_INTERVAL_MS,
  MockBackend,
} from "@voltip/shared/mock";
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

describe("Settings · 录音来源 (was 麦克风)", () => {
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
    expect(await screen.findByText(/出错了 · 麦克风标识须为 1–1024 字节/)).toBeInTheDocument();
  });

  it("the source decides the rows: the computer's sound shows the output device and hides the microphone, mixing shows both and the echo cancellation switch with its hint; an output is chosen and followed back to the default (docs/dictation.md section 22)", async () => {
    const user = userEvent.setup();
    const { backend } = renderApp({ path: "/settings/microphone" });
    const pane = await openPane();
    expect(within(pane).queryByRole("combobox", { name: "输出设备" })).toBeNull();
    // One source has no echo to cancel: the switch is for mixed takes only.
    expect(within(pane).queryByRole("switch", { name: "消除扬声器回声" })).toBeNull();
    const switcher = within(pane).getByTestId("recording-source");
    await user.click(within(switcher).getByRole("radio", { name: "电脑声音" }));
    expect(backend.peek().settings.recording.source).toBe("system");
    const output = await within(pane).findByRole("combobox", { name: "输出设备" });
    expect(within(pane).queryByRole("combobox", { name: "输入设备" })).toBeNull();
    expect(within(pane).queryByRole("switch", { name: "消除扬声器回声" })).toBeNull();
    expect(within(pane).queryByTestId("microphone-test")).toBeNull();
    expect(
      within(output)
        .getAllByRole("option")
        .map((o) => o.textContent),
    ).toEqual([
      `跟随系统默认（${MOCK_AUDIO_OUTPUTS[0]?.name ?? ""}）`,
      ...MOCK_AUDIO_OUTPUTS.map((d) => d.name),
    ]);
    await user.selectOptions(output, "Sony WH-1000XM5");
    await waitFor(() => {
      expect(backend.peek().settings.recording.output_device).toBe("Sony WH-1000XM5");
    });
    await user.click(within(switcher).getByRole("radio", { name: "混合" }));
    expect(backend.peek().settings.recording).toMatchObject({
      source: "mixed",
      output_device: "Sony WH-1000XM5",
    });
    expect(within(pane).getByRole("combobox", { name: "输入设备" })).toBeInTheDocument();
    expect(within(pane).getByRole("combobox", { name: "输出设备" })).toHaveValue("Sony WH-1000XM5");
    // Since 2026-10-01 (docs/dictation.md §22.6, the user's request of 2026-09-30) a mixed take
    // cancels the speakers' echo by default and says so; switched off, the headphones hint is back.
    const echo = within(pane).getByRole("switch", { name: "消除扬声器回声" });
    expect(echo).toHaveAttribute("aria-checked", "true");
    expect(pane).toHaveTextContent("已消除扬声器回声。外放音量很大时，仍建议佩戴耳机。");
    await user.click(echo);
    await waitFor(() => {
      expect(backend.peek().settings.recording.echo_cancel).toBe(false);
    });
    expect(within(pane).getByRole("switch", { name: "消除扬声器回声" })).toHaveAttribute(
      "aria-checked",
      "false",
    );
    expect(pane).toHaveTextContent("混合录制时请佩戴耳机");
    await user.selectOptions(within(pane).getByRole("combobox", { name: "输出设备" }), "");
    await waitFor(() => {
      expect(backend.peek().settings.recording.output_device).toBeNull();
    });
  });

  it("an unplugged output stays chosen and is named as such; where the computer's sound cannot be recorded the pane says why", async () => {
    renderApp({
      path: "/settings/microphone",
      mock: {
        settings: {
          recording: {
            source: "system",
            output_device: "HDMI",
            max_minutes: 10,
            echo_cancel: true,
          },
        },
      },
    });
    const dialog = await screen.findByRole("dialog", { name: "设置" });
    const pane = await within(dialog).findByTestId("microphone-pane");
    const output = await within(pane).findByRole("combobox", { name: "输出设备" });
    await waitFor(() => {
      expect(within(output).getByRole("option", { name: "HDMI · 未连接" })).toBeInTheDocument();
    });
    expect(output).toHaveValue("HDMI");
    expect(pane).toHaveTextContent("所选输出设备未连接，听写会先录制系统默认输出");
  });

  it("without a sound server the computer's sound is off, the reason under the choice", async () => {
    renderApp({
      path: "/settings/microphone",
      mock: { audioOutputs: { system_audio: { state: "no_sound_server" }, devices: [] } },
    });
    const pane = await openPane();
    const switcher = within(pane).getByTestId("recording-source");
    await waitFor(() => {
      expect(within(switcher).getByRole("radio", { name: "电脑声音" })).toBeDisabled();
    });
    expect(pane).toHaveTextContent(
      "录制电脑声音需要 PulseAudio 或 PipeWire 音频服务，当前未检测到。",
    );
  });
});
