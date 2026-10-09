// The 反馈 page's pure helpers, shared by the desktop's dialog and the phone's page
// (docs/feedback.md; user decision 2026-10-01: the phone sends feedback of its own): how the
// diagnostics are worded, the wire names of a refusal, an attachment's type and the checks before
// its bytes are read, and the limits as the help line words them. The shell checks everything
// again. Moved here from `apps/desktop/src/pages/Feedback.tsx`.
import { type Locale, type TFunction } from "./i18n";
import { outputModeLabel } from "./labels";
import {
  FEEDBACK_ATTACHMENT_ERRORS,
  FEEDBACK_ATTACHMENT_TYPES,
  FEEDBACK_ERRORS,
  FEEDBACK_MAX_ATTACHMENTS,
  FEEDBACK_MAX_ATTACHMENT_TOTAL_BYTES,
  FEEDBACK_MAX_IMAGE_BYTES,
  FEEDBACK_MAX_VIDEO_BYTES,
  type FeedbackAttachmentError,
  type FeedbackDiagnostics,
  type FeedbackError,
  PROVIDER_IDS,
  type ProviderId,
  type StagedAttachment,
} from "./schema";

const OS_NAMES: Record<string, string> = {
  windows: "Windows",
  linux: "Linux",
  macos: "macOS",
  android: "Android",
  ios: "iOS",
};

/** The diagnostics in the order the page lists them. */
export const FEEDBACK_DIAGNOSTIC_ORDER = [
  "app_version",
  "os",
  "arch",
  "session",
  "locale",
  "asr_provider",
  "local_model",
  "compute",
  "llm_provider",
  "output_mode",
] as const satisfies readonly (keyof FeedbackDiagnostics)[];

function isProvider(value: string): value is ProviderId {
  return (PROVIDER_IDS as readonly string[]).includes(value);
}

function isOutputMode(value: string): value is Parameters<typeof outputModeLabel>[0] {
  return value === "whole_take" || value === "streaming_final" || value === "live_inject";
}

/** One diagnostics value as the page words it: provider and mode names, not wire ids. */
export function diagnosticValue(
  key: (typeof FEEDBACK_DIAGNOSTIC_ORDER)[number],
  value: string,
  t: TFunction,
  locale: Locale,
): string {
  switch (key) {
    case "os":
      return OS_NAMES[value] ?? value;
    case "asr_provider":
    case "llm_provider":
      return isProvider(value) ? t(`engines.provider.${value}`) : value;
    case "compute":
      return value === "auto" || value === "cpu" || value === "gpu"
        ? t(`engines.compute.${value}`)
        : value;
    case "output_mode":
      return isOutputMode(value) ? outputModeLabel(value, locale) : value;
    default:
      return value;
  }
}

/** The wire name of a failed submission, or `server` for anything else the shell said. */
export function feedbackError(error: unknown): FeedbackError {
  const text = error instanceof Error ? error.message : String(error);
  return FEEDBACK_ERRORS.find((e) => e === text) ?? "server";
}

/** The wire name of a refused attachment; anything else the shell said reads as a wrong type. */
export function attachmentError(error: unknown): FeedbackAttachmentError {
  const text = error instanceof Error ? error.message : String(error);
  return FEEDBACK_ATTACHMENT_ERRORS.find((e) => e === text) ?? "attachment_type";
}

/** The types a file with no type from the webview is taken for, by its extension. */
const EXTENSION_TYPES: Record<string, string> = {
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  gif: "image/gif",
  webp: "image/webp",
  mp4: "video/mp4",
  webm: "video/webm",
  mov: "video/quicktime",
};

/** A picked or pasted file's MIME type: the webview's, else the extension's, else none. */
export function attachmentType(file: { name: string; type: string }): string {
  if (file.type.length > 0) return file.type;
  const dot = file.name.lastIndexOf(".");
  return dot < 0 ? "" : (EXTENSION_TYPES[file.name.slice(dot + 1).toLowerCase()] ?? "");
}

/** The refusal the shell would give, told from the type and the size before the bytes are read
 *  (a large video is never loaded to be turned away); the shell checks again. */
export function precheckAttachment(
  type: string,
  size: number,
  staged: readonly StagedAttachment[],
): FeedbackAttachmentError | undefined {
  if (!(FEEDBACK_ATTACHMENT_TYPES as readonly string[]).includes(type)) return "attachment_type";
  const limit = type.startsWith("video/") ? FEEDBACK_MAX_VIDEO_BYTES : FEEDBACK_MAX_IMAGE_BYTES;
  if (size === 0 || size > limit) return "attachment_too_large";
  if (staged.length >= FEEDBACK_MAX_ATTACHMENTS) return "attachment_too_many";
  const total = staged.reduce((n, a) => n + a.size, 0) + size;
  return total > FEEDBACK_MAX_ATTACHMENT_TOTAL_BYTES ? "attachment_total" : undefined;
}

/** A limit in whole megabytes (`5 MB`), as the help line and the refusals word them. */
const megabytes = (bytes: number) => `${Math.round(bytes / 1_048_576)} MB`;

/** The attachment limits as the help line and the refusals word them. */
export const FEEDBACK_LIMITS = {
  count: FEEDBACK_MAX_ATTACHMENTS,
  image: megabytes(FEEDBACK_MAX_IMAGE_BYTES),
  video: megabytes(FEEDBACK_MAX_VIDEO_BYTES),
  total: megabytes(FEEDBACK_MAX_ATTACHMENT_TOTAL_BYTES),
};

/** `12.3 MB`, `640 KB`: an attachment's size as the list shows it. */
export function attachmentSize(bytes: number): string {
  if (bytes >= 1_048_576) return `${(bytes / 1_048_576).toFixed(1)} MB`;
  return `${Math.max(0, Math.round(bytes / 1024))} KB`;
}
