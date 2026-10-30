// Importing audio and video files as the interface shows it (docs/dictation.md §31): which dropped
// files the import takes, where a job is in words, and its progress.
import type { Locale, TFunction } from "./i18n";
import { serviceReasonText } from "./labels";
import type { HistoryEntry, ImportFailure, ImportJob, ImportState, ImportsView } from "./schema";

/** The last part of `path`, by either separator: what the queue and the entry show. */
export function baseName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts.at(-1) ?? path;
}

/** `name` without its extension: an imported file's subtitles are offered under its own name. */
export function stem(name: string): string {
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(0, dot) : name;
}

/** The extension of `path`, lowercase, without the dot; `""` without one. */
export function extensionOf(path: string): string {
  const name = baseName(path);
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
}

/** Dropped `paths` split into the files the import takes (by the extensions the shell's decoder
 *  reads) and the others, each in order. */
export function splitDrop(
  paths: readonly string[],
  extensions: readonly string[],
): { accepted: string[]; rejected: string[] } {
  const known = new Set(extensions.map((e) => e.toLowerCase()));
  const accepted: string[] = [];
  const rejected: string[] = [];
  for (const path of paths) (known.has(extensionOf(path)) ? accepted : rejected).push(path);
  return { accepted, rejected };
}

/** Decoding, recognising or polishing. */
export function isImportRunning(state: ImportState): boolean {
  return state.kind === "decoding" || state.kind === "recognizing" || state.kind === "polishing";
}

/** The job that runs, if one does: the queue runs one at a time. */
export function runningImport(view: ImportsView): ImportJob | undefined {
  return view.jobs.find((job) => isImportRunning(job.state));
}

/** How far a running job is, 0–1; `undefined` while that is not known (decoding a file whose
 *  length is not known yet). Decoding is the first tenth, recognition the rest; the polish is
 *  measured on its own. */
export function importProgress(state: ImportState): number | undefined {
  switch (state.kind) {
    case "decoding":
      return state.pct === undefined ? undefined : state.pct / 1000;
    case "recognizing":
      return state.total === 0 ? 0.1 : 0.1 + (0.9 * state.done) / state.total;
    case "polishing":
      return state.total === 0 ? 1 : state.done / state.total;
    default:
      return undefined;
  }
}

/** The files a drop left out, by name when it is one (short enough for a toast). */
export function skippedFilesText(names: readonly string[], t: TFunction): string {
  const [only] = names;
  return names.length === 1 && only !== undefined
    ? t("history.imports.dropRejectedFile", { name: only })
    : t("history.imports.dropRejected", { n: names.length });
}

/** Why a job stopped, in words; a recognition failure gives the client's reason in the
 *  interface's language, as a dictation's does (docs/dictation.md §3.12). */
export function importFailureText(failure: ImportFailure, t: TFunction, locale: Locale): string {
  switch (failure.code) {
    case "recognition":
      return t("history.imports.failure.recognition", {
        detail: serviceReasonText(failure.detail, locale),
      });
    case "unreadable":
    case "corrupt":
    case "storage":
      return t(`history.imports.failure.${failure.code}`, { detail: failure.detail });
    case "unsupported_codec":
      return t("history.imports.failure.unsupported_codec", { codec: failure.codec });
    case "unsupported_format":
    case "no_audio":
    case "no_speech":
      return t(`history.imports.failure.${failure.code}`);
  }
}

/** Where a job is, in words. */
export function importStateText(state: ImportState, t: TFunction, locale: Locale): string {
  switch (state.kind) {
    case "queued":
      return t("history.imports.queue.queued");
    case "decoding":
      return state.pct === undefined
        ? t("history.imports.queue.decoding")
        : t("history.imports.queue.decodingPct", { pct: state.pct });
    case "recognizing":
      return t("history.imports.queue.recognizing", { done: state.done, total: state.total });
    case "polishing":
      return t("history.imports.queue.polishing", { done: state.done, total: state.total });
    case "done":
      return state.polish_failed > 0
        ? t("history.imports.queue.donePolishFailed", { n: state.polish_failed })
        : t("history.imports.queue.done");
    case "failed":
      return importFailureText(state.failure, t, locale);
  }
}

/** An entry made from an imported file (docs/dictation.md §31). */
export function isImported(entry: HistoryEntry): boolean {
  return entry.file !== undefined;
}

/** Whether `entry`'s subtitles can carry polished text: one of its segments has some. */
export function hasPolishedSegments(entry: HistoryEntry): boolean {
  return (entry.segments ?? []).some((s) => (s.refined ?? "").trim().length > 0);
}
