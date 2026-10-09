import { MOCK_AUDIO_DEVICES, MOCK_METER_INTERVAL_MS, MockBackend } from "@voltip/shared/mock";
import { BackendProvider } from "@voltip/ui";
import { act, render, screen } from "@testing-library/react";
import { NO_INPUT_DEVICE, levelFraction, useAudioMeter } from "./useAudioMeter";

function Probe({ active, deviceId }: { active: boolean; deviceId?: string }) {
  const meter = useAudioMeter(active, deviceId);
  return (
    <div
      data-testid="probe"
      data-devices={meter.devices?.length ?? "pending"}
      data-device={meter.device?.name ?? ""}
      data-missing={String(meter.missing)}
      data-seq={meter.frame?.seq ?? ""}
      data-error={meter.error ?? ""}
    />
  );
}

describe("useAudioMeter", () => {
  it("enumerates devices, streams frames from the backend while active and stops on unmount", async () => {
    vi.useFakeTimers();
    try {
      const backend = new MockBackend();
      const view = render(
        <BackendProvider backend={backend}>
          <Probe active />
        </BackendProvider>,
      );
      await act(async () => {
        await Promise.resolve();
      });
      const probe = screen.getByTestId("probe");
      expect(probe.dataset.devices).toBe(String(MOCK_AUDIO_DEVICES.length));
      expect(probe.dataset.device).toBe(MOCK_AUDIO_DEVICES[0]?.name);
      expect(backend.activeMeters()).toBe(1);
      act(() => {
        vi.advanceTimersByTime(MOCK_METER_INTERVAL_MS * 2 + 1);
      });
      expect(probe.dataset.seq).toBe("2");
      view.rerender(
        <BackendProvider backend={backend}>
          <Probe active={false} />
        </BackendProvider>,
      );
      expect(backend.activeMeters()).toBe(0);
      expect(screen.getByTestId("probe").dataset.seq).toBe("");
      view.unmount();
    } finally {
      vi.useRealTimers();
    }
  });

  it("regression: a device the backend cannot open surfaces as the shell's error, never as a silent meter", async () => {
    const backend = new MockBackend();
    backend.meter = () =>
      Promise.reject(new Error("audio: device busy: Fifine K669 USB Microphone"));
    render(
      <BackendProvider backend={backend}>
        <Probe active deviceId="Fifine K669 USB Microphone" />
      </BackendProvider>,
    );
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(screen.getByTestId("probe").dataset.error).toMatch(/device busy/);
  });

  it("regression: a chosen microphone that is unplugged meters the default input and says so", async () => {
    vi.useFakeTimers();
    try {
      const backend = new MockBackend();
      render(
        <BackendProvider backend={backend}>
          <Probe active deviceId="Blue Yeti" />
        </BackendProvider>,
      );
      await act(async () => {
        await Promise.resolve();
        await Promise.resolve();
      });
      const probe = screen.getByTestId("probe");
      expect(probe.dataset.missing).toBe("true");
      expect(probe.dataset.device).toBe(MOCK_AUDIO_DEVICES[0]?.name);
      expect(probe.dataset.error).toBe("");
      act(() => {
        vi.advanceTimersByTime(MOCK_METER_INTERVAL_MS + 1);
      });
      expect(probe.dataset.seq).toBe("1");
    } finally {
      vi.useRealTimers();
    }
  });

  it("reports an empty device list as 没有可用的麦克风 and enumeration failures verbatim", async () => {
    const empty = new MockBackend();
    empty.audioDevices = () => Promise.resolve([]);
    render(
      <BackendProvider backend={empty}>
        <Probe active={false} />
      </BackendProvider>,
    );
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByTestId("probe").dataset.error).toBe(NO_INPUT_DEVICE);
    const broken = new MockBackend();
    broken.audioDevices = () => Promise.reject(new Error("WASAPI: 0x8889000A"));
    const second = render(
      <BackendProvider backend={broken}>
        <Probe active={false} />
      </BackendProvider>,
    );
    await act(async () => {
      await Promise.resolve();
    });
    expect(second.container.querySelector("[data-testid=probe]")?.getAttribute("data-error")).toBe(
      "WASAPI: 0x8889000A",
    );
  });

  it("levelFraction maps −60…0 dBFS onto 0…1 and clamps", () => {
    expect(levelFraction(-60)).toBe(0);
    expect(levelFraction(-30)).toBeCloseTo(0.5);
    expect(levelFraction(0)).toBe(1);
    expect(levelFraction(-90)).toBe(0);
    expect(levelFraction(3)).toBe(1);
  });
});
