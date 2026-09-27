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

export interface Feedback {
  kind: Kind;
  message: string;
  contact: string | null;
  diagnostics: Diagnostics;
}

export type Validation = { ok: true; feedback: Feedback } | { ok: false; field: string };

/** A diagnostics value: short, printable, no markup. */
const DIAGNOSTIC_VALUE = /^[\w.+\- :/()]+$/u;

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
  return { ok: true, feedback: { kind: body.kind, message, contact, diagnostics } };
}
