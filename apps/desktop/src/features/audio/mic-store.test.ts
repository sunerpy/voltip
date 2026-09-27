import {
  MICROPHONE_READOUT_MAX,
  microphoneReadoutValue,
  publishMicrophone,
  resetMicrophoneReadout,
  shortMicrophoneName,
  useMicrophoneReadout,
} from "./mic-store";
import { act, renderHook } from "@testing-library/react";

describe("microphone readout store", () => {
  afterEach(() => {
    resetMicrophoneReadout();
  });

  it("regression: the title bar names the metered device, never a fixture — pending, then device, then failure", () => {
    const { result } = renderHook(() => useMicrophoneReadout());
    expect(microphoneReadoutValue(result.current)).toBe("枚举中…");
    act(() => {
      publishMicrophone({
        device: { id: "x", name: "Fifine K669 USB Microphone", is_default: true },
        error: undefined,
      });
    });
    expect(microphoneReadoutValue(result.current)).toBe("Fifine K669");
    act(() => {
      publishMicrophone({ device: undefined, error: "没有可用的麦克风" });
    });
    expect(microphoneReadoutValue(result.current)).toBe("不可用");
    // Publishing the same value again does not notify subscribers.
    const before = result.current;
    act(() => {
      publishMicrophone({ device: undefined, error: "没有可用的麦克风" });
    });
    expect(result.current).toBe(before);
  });

  it("regression: a long backend device name is shortened for the title bar instead of filling it", () => {
    // Real Linux smoke (2026-09-25): the PulseAudio default device name spanned the whole bar.
    expect(shortMicrophoneName("Playback/recording through the PulseAudio sound server")).toBe(
      "PulseAudio",
    );
    expect(shortMicrophoneName("Fifine K669 USB Microphone")).toBe("Fifine K669");
    expect(shortMicrophoneName("Microphone Array (Realtek(R) Audio)")).toBe("Microphone Array");
    expect(shortMicrophoneName("Headset Microphone")).toBe("Headset");
    const long = shortMicrophoneName("A very long device name that keeps going on and on forever");
    expect(long.length).toBeLessThanOrEqual(MICROPHONE_READOUT_MAX);
    expect(long.endsWith("…")).toBe(true);
    // Nothing left after stripping: keep the original rather than an empty readout.
    expect(shortMicrophoneName("Microphone")).toBe("Microphone");
  });
});
