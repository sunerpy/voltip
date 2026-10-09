/** The request body the desktop app sends (`apps/desktop/src-tauri/src/feedback.rs`). */
export const KINDS = ["bug", "idea", "other"] as const;
export type Kind = (typeof KINDS)[number];

/** In UTF-16 code units, as the dialog's `maxLength` counts them. */
export const MAX_MESSAGE_CHARS = 5000;
export const MAX_CONTACT_CHARS = 200;
export const MAX_DIAGNOSTIC_CHARS = 64;

/** The diagnostics the app attaches, all non-sensitive: versions, platform, which provider kind
 *  is in use. Anything else is dropped, so a newer app can add a field without an older endpoint
 *  refusing its feedback. */
export const DIAGNOSTIC_KEYS = [
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
] as const;
export type DiagnosticKey = (typeof DIAGNOSTIC_KEYS)[number];
export type Diagnostics = Partial<Record<DiagnosticKey, string>>;

/** Attachments (screenshots and screen recordings, docs/feedback.md): what the report declares.
 *  The bytes follow in chunks (`PUT …/attachments/<index>/<seq>`); the limits are the app's
 *  (`apps/desktop/src-tauri/src/feedback.rs`, `packages/shared/src/schema.ts`). */
export const ATTACHMENT_TYPES = [
  "image/png",
  "image/jpeg",
  "image/gif",
  "image/webp",
  "video/mp4",
  "video/webm",
  "video/quicktime",
] as const;
export type AttachmentType = (typeof ATTACHMENT_TYPES)[number];
export const MAX_ATTACHMENTS = 3;
export const MAX_IMAGE_BYTES = 5 * 1024 * 1024;
export const MAX_VIDEO_BYTES = 20 * 1024 * 1024;
export const MAX_ATTACHMENT_TOTAL_BYTES = 25 * 1024 * 1024;
export const MAX_ATTACHMENT_NAME_CHARS = 120;
/** One upload request carries one chunk of this size (the last one the rest): a small request
 *  keeps every step well inside a Worker's limits, and a failed chunk is retried on its own. */
export const ATTACHMENT_CHUNK_BYTES = 1024 * 1024;

export interface Attachment {
  name: string;
  type: AttachmentType;
  size: number;
  /** Hex SHA-256 of the bytes, as the app computed it; kept for the reader to check. */
  sha256: string;
}

/** The limit for one attachment of `type`. */
export function maxBytesFor(type: AttachmentType): number {
  return type.startsWith("video/") ? MAX_VIDEO_BYTES : MAX_IMAGE_BYTES;
}

/** How many chunks an attachment of `size` bytes arrives in. */
export function chunkCount(size: number): number {
  return Math.ceil(size / ATTACHMENT_CHUNK_BYTES);
}

export interface Feedback {
  kind: Kind;
  message: string;
  contact: string | null;
  diagnostics: Diagnostics;
  attachments: Attachment[];
}

export type Validation = { ok: true; feedback: Feedback } | { ok: false; field: string };

/** A diagnostics value: short, printable, no markup. */
const DIAGNOSTIC_VALUE = /^[\w.+\- :/()]+$/u;
const SHA256 = /^[0-9a-f]{64}$/u;
/** Control characters and the characters a file name must not carry (path separators, quotes). */
// oxlint-disable-next-line no-control-regex -- control characters are exactly what is refused
const BAD_NAME = /[\u0000-\u001f\u007f"/\\]/u;

function isAttachmentType(value: unknown): value is AttachmentType {
  return typeof value === "string" && (ATTACHMENT_TYPES as readonly string[]).includes(value);
}

/** The declared attachments, or the first bad field. */
function attachments(value: unknown): Attachment[] | string {
  if (value === undefined || value === null) return [];
  if (!Array.isArray(value) || value.length > MAX_ATTACHMENTS) return "attachments";
  const out: Attachment[] = [];
  let total = 0;
  for (const [i, entry] of value.entries()) {
    if (!isRecord(entry)) return `attachments.${i}`;
    const name = typeof entry.name === "string" ? entry.name.trim() : "";
    if (name.length === 0 || name.length > MAX_ATTACHMENT_NAME_CHARS || BAD_NAME.test(name))
      return `attachments.${i}.name`;
    if (!isAttachmentType(entry.type)) return `attachments.${i}.type`;
    const size = entry.size;
    if (
      typeof size !== "number" ||
      !Number.isInteger(size) ||
      size < 1 ||
      size > maxBytesFor(entry.type)
    )
      return `attachments.${i}.size`;
    if (typeof entry.sha256 !== "string" || !SHA256.test(entry.sha256))
      return `attachments.${i}.sha256`;
    total += size;
    out.push({ name, type: entry.type, size, sha256: entry.sha256 });
  }
  return total > MAX_ATTACHMENT_TOTAL_BYTES ? "attachments" : out;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isKind(value: unknown): value is Kind {
  return typeof value === "string" && (KINDS as readonly string[]).includes(value);
}

/** Checks a parsed body; the first bad field is named, never echoed. */
export function validate(body: unknown): Validation {
  if (!isRecord(body)) return { ok: false, field: "body" };
  if (!isKind(body.kind)) return { ok: false, field: "kind" };
  const message = typeof body.message === "string" ? body.message.trim() : "";
  if (message.length === 0 || message.length > MAX_MESSAGE_CHARS)
    return { ok: false, field: "message" };
  let contact: string | null = null;
  if (body.contact !== undefined && body.contact !== null) {
    if (typeof body.contact !== "string") return { ok: false, field: "contact" };
    const trimmed = body.contact.trim();
    if (trimmed.length > MAX_CONTACT_CHARS) return { ok: false, field: "contact" };
    contact = trimmed.length > 0 ? trimmed : null;
  }
  const diagnostics: Diagnostics = {};
  if (body.diagnostics !== undefined) {
    if (!isRecord(body.diagnostics)) return { ok: false, field: "diagnostics" };
    for (const key of DIAGNOSTIC_KEYS) {
      const value = body.diagnostics[key];
      if (value === undefined || value === null) continue;
      if (
        typeof value !== "string" ||
        value.length > MAX_DIAGNOSTIC_CHARS ||
        !DIAGNOSTIC_VALUE.test(value)
      )
        return { ok: false, field: `diagnostics.${key}` };
      diagnostics[key] = value;
    }
  }
  const declared = attachments(body.attachments);
  if (typeof declared === "string") return { ok: false, field: declared };
  return {
    ok: true,
    feedback: { kind: body.kind, message, contact, diagnostics, attachments: declared },
  };
}
