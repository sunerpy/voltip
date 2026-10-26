import {
  AUDIO_CACHE_KEEP_OPTIONS,
  AUDIO_CACHE_MB_OPTIONS,
  clampAudioKeep,
  megabytesLabel,
  recordingOfEntry,
  retakingRecording,
  storageSize,
  unrecognisedRecordings,
  withSavedOption,
} from "./audio-cache";
import {
  AUDIO_CACHE_MAX_KEEP,
  AUDIO_CACHE_MAX_MB,
  AUDIO_CACHE_MIN_KEEP,
  AUDIO_CACHE_MIN_MB,
  type AudioCacheView,
  type DictationStatus,
  type KeptRecording,
  audioCacheSettingsSchema,
  audioCacheViewSchema,
  defaultAudioCacheSettings,
} from "./schema";

function recording(id: string, status: KeptRecording["status"]): KeptRecording {
  return { id, at_ms: 1, duration_ms: 3000, bytes: 96_044, status };
}

const CACHE: AudioCacheView = {
  available: true,
  bytes: 5 * 96_044,
  recordings: [
    recording("pending", { kind: "pending" }),
    recording("recognized", { kind: "recognized", entry: "e1" }),
    recording("failed", { kind: "failed", code: "asr", message: "asr: 503" }),
    recording("cancelled", { kind: "cancelled" }),
    recording("interrupted", { kind: "interrupted" }),
  ],
};

describe("audio cache (docs/dictation.md §26)", () => {
  it("offers choices within the core's bounds, and a saved value outside them stays selectable", () => {
    expect(
      AUDIO_CACHE_KEEP_OPTIONS.every((n) => n >= AUDIO_CACHE_MIN_KEEP && n <= AUDIO_CACHE_MAX_KEEP),
    ).toBe(true);
    expect(AUDIO_CACHE_MB_OPTIONS[0]).toBe(AUDIO_CACHE_MIN_MB);
    expect(AUDIO_CACHE_MB_OPTIONS.at(-1)).toBe(AUDIO_CACHE_MAX_MB);
    expect(AUDIO_CACHE_KEEP_OPTIONS).toContain(defaultAudioCacheSettings().keep);
    expect(AUDIO_CACHE_MB_OPTIONS).toContain(defaultAudioCacheSettings().max_mb);
    expect(withSavedOption(AUDIO_CACHE_KEEP_OPTIONS, 20)).toEqual([...AUDIO_CACHE_KEEP_OPTIONS]);
    expect(withSavedOption([5, 10, 20], 7)).toEqual([5, 7, 10, 20]);
    expect([clampAudioKeep(0), clampAudioKeep(7.4), clampAudioKeep(999)]).toEqual([1, 7, 200]);
  });

  it("names sizes in the decimal units the settings count in", () => {
    expect(["50 MB", "200 MB", "1 GB", "10 GB", "1500 MB"]).toEqual(
      [50, 200, 1000, 10_000, 1500].map(megabytesLabel),
    );
    expect(storageSize(0)).toBe("0 KB");
    expect(storageSize(44)).toBe("1 KB");
    expect(storageSize(192_088)).toBe("192 KB");
    expect(storageSize(12_345_678)).toBe("12.3 MB");
    expect(storageSize(1_234_000_000)).toBe("1.2 GB");
    expect(storageSize(-5)).toBe("0 KB");
  });

  it("the history page lists what gave no text; a recognised one belongs to its entry", () => {
    expect(unrecognisedRecordings(CACHE).map((r) => r.id)).toEqual([
      "failed",
      "cancelled",
      "interrupted",
    ]);
    expect(recordingOfEntry(CACHE, "e1")?.id).toBe("recognized");
    expect(recordingOfEntry(CACHE, "e2")).toBeUndefined();
  });

  it("knows which recording is being recognised again, only while it is", () => {
    const status = (
      phase: DictationStatus["phase"],
      extra: Partial<DictationStatus>,
    ): DictationStatus => ({
      session: 1,
      phase,
      kind: "dictation",
      ...extra,
    });
    const processing = { phase: "processing", stage: "transcribing", started_at: 1 } as const;
    expect(retakingRecording(status(processing, { recording: "r1", retake: true }))).toBe("r1");
    expect(retakingRecording(status(processing, { recording: "r1" }))).toBeUndefined();
    expect(
      retakingRecording(status({ phase: "idle" }, { recording: "r1", retake: true })),
    ).toBeUndefined();
  });

  it("parses the core's view and settings, and defaults what an older core does not send", () => {
    expect(audioCacheViewSchema.parse(CACHE)).toEqual(CACHE);
    expect(audioCacheSettingsSchema.parse({})).toEqual({ enabled: true, keep: 20, max_mb: 200 });
    expect(
      audioCacheSettingsSchema.safeParse({ enabled: true, keep: 0, max_mb: 200 }).success,
    ).toBe(false);
    expect(
      audioCacheSettingsSchema.safeParse({ enabled: true, keep: 20, max_mb: 20_000 }).success,
    ).toBe(false);
    expect(
      audioCacheViewSchema.safeParse({
        ...CACHE,
        recordings: [recording("x", { kind: "failed", code: "nope", message: "" } as never)],
      }).success,
    ).toBe(false);
  });
});
