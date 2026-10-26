// The audio cache as the interface shows it (docs/dictation.md §26): the choices the settings offer,
// sizes in the decimal units the settings count in, and which kept recording belongs where — the
// unrecognised ones on the history page, a recognised one with the entry made from it.
import {
  AUDIO_CACHE_MAX_KEEP,
  AUDIO_CACHE_MAX_MB,
  AUDIO_CACHE_MB,
  AUDIO_CACHE_MIN_KEEP,
  AUDIO_CACHE_MIN_MB,
  type AudioCacheView,
  type DictationStatus,
  type EngineSettings,
  type KeptRecording,
  type PolishChoice,
  type RecognitionChoice,
} from "./schema";

/** How many recordings the select offers. */
export const AUDIO_CACHE_KEEP_OPTIONS = [5, 10, 20, 50, 100, AUDIO_CACHE_MAX_KEEP] as const;
/** How much space the select offers, in MB. */
export const AUDIO_CACHE_MB_OPTIONS = [
  AUDIO_CACHE_MIN_MB,
  100,
  200,
  500,
  1000,
  2000,
  5000,
  AUDIO_CACHE_MAX_MB,
] as const;

/** `options` with `saved` among them (a value saved by hand or by another build), in order. */
export function withSavedOption(options: readonly number[], saved: number): number[] {
  const all = options.includes(saved) ? [...options] : [...options, saved];
  all.sort((a, b) => a - b);
  return all;
}

/** `keep` within the core's bounds. */
export function clampAudioKeep(keep: number): number {
  return Math.min(AUDIO_CACHE_MAX_KEEP, Math.max(AUDIO_CACHE_MIN_KEEP, Math.round(keep)));
}

/** A space limit as the select names it: `200 MB`, `1 GB`, `10 GB`. */
export function megabytesLabel(mb: number): string {
  return mb >= 1000 && mb % 1000 === 0 ? `${mb / 1000} GB` : `${mb} MB`;
}

/** A size in decimal units, as the settings count them: `320 KB`, `12.3 MB`, `1.2 GB`. */
export function storageSize(bytes: number): string {
  const n = Math.max(0, bytes);
  if (n < AUDIO_CACHE_MB) return `${Math.max(n > 0 ? 1 : 0, Math.round(n / 1000))} KB`;
  if (n < 1000 * AUDIO_CACHE_MB) return `${(n / AUDIO_CACHE_MB).toFixed(1)} MB`;
  return `${(n / (1000 * AUDIO_CACHE_MB)).toFixed(1)} GB`;
}

/** The kept recordings no text came from — failed, cancelled, or interrupted when Voltip closed —
 *  newest first: the history page lists them for 重新识别. One still being processed (`pending`,
 *  its take is running) is not among them. */
export function unrecognisedRecordings(cache: AudioCacheView): KeptRecording[] {
  return cache.recordings.filter(
    (r) => r.status.kind !== "recognized" && r.status.kind !== "pending",
  );
}

/** The kept recording the entry `entryId` was made from, if the cache still holds it. */
export function recordingOfEntry(
  cache: AudioCacheView,
  entryId: string,
): KeptRecording | undefined {
  return cache.recordings.find((r) => r.status.kind === "recognized" && r.status.entry === entryId);
}

/** The models a 重新识别 runs on instead of the settings' (`RetakeModels`, docs/dictation.md
 *  §26.4), for that take only; a part left out keeps the settings' choice. */
export interface RetakeModels {
  asr?: RecognitionChoice;
  refine?: PolishChoice;
}

/** `engines` with these choices made, as switching to them in the settings would; the endpoint a
 *  provider was set up with stays (`RetakeModels::apply`). */
export function applyRetakeModels(engines: EngineSettings, models: RetakeModels): EngineSettings {
  const providers = { ...engines.providers };
  const next: EngineSettings = { ...engines, providers };
  const { asr, refine } = models;
  if (asr?.kind === "remote") {
    next.asr_provider = asr.provider;
    providers[asr.provider] = { ...providers[asr.provider], asr_model: asr.model };
  } else if (asr?.kind === "local") {
    next.asr_provider = "local";
    next.local_model = asr.id;
  }
  if (refine?.kind === "off") {
    next.refine_enabled = false;
  } else if (refine?.kind === "remote") {
    next.refine_enabled = true;
    next.llm_provider = refine.provider;
    providers[refine.provider] = { ...providers[refine.provider], llm_model: refine.model };
  }
  return next;
}

/** The recording a take is recognising again right now (`dictation.retake`), if any. */
export function retakingRecording(status: DictationStatus): string | undefined {
  const running = status.phase.phase === "processing";
  return running && status.retake === true ? status.recording : undefined;
}
