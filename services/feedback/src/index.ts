/** The feedback endpoint (docs/feedback.md): the desktop app's 反馈 page posts here, a Cloudflare
 *  Worker validates the report and stores it in D1. The app sends only what the page showed the
 *  user; the client address is kept as a salted hash, for the rate limit. Screenshots and screen
 *  recordings the report declares follow in 1 MiB chunks into a files database of their own. */
import {
  type AttachmentRow,
  type AttachmentStore,
  type D1Like,
  type FeedbackRow,
  type FeedbackStore,
  d1Files,
  d1Store,
} from "./store";
import { ATTACHMENT_CHUNK_BYTES, chunkCount, validate } from "./validate";

export interface Env {
  DB: D1Like;
  /** The files database (schema-files.sql); without it the endpoint takes no attachments. */
  FILES?: D1Like;
  /** The application token the desktop build carries (Bearer); unset accepts any client. */
  FEEDBACK_TOKEN?: string;
  /** Reading feedback back (`GET /v1/feedback`); unset turns reading off. */
  ADMIN_TOKEN?: string;
  /** Salts the client-address hash. */
  IP_SALT?: string;
}

export const MAX_BODY_BYTES = 32 * 1024;
/** Per client address, per hour. */
export const PER_CLIENT_PER_HOUR = 10;
/** Everyone together, per day: a ceiling on what a flood can cost. */
export const TOTAL_PER_DAY = 5000;
export const LIST_LIMIT_MAX = 200;
const HOUR_MS = 3_600_000;
const DAY_MS = 24 * HOUR_MS;
/** Attachment bytes one client address may declare a day. */
export const ATTACHMENT_BYTES_PER_CLIENT_PER_DAY = 60 * 1024 * 1024;
/** What the files database may hold: under the free plan's 500 MB per database. */
export const ATTACHMENT_STORE_MAX_BYTES = 400 * 1024 * 1024;
/** How long after the report its attachments may still arrive. */
export const UPLOAD_WINDOW_MS = HOUR_MS;

function json(status: number, body: unknown, headers: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json; charset=utf-8", ...headers },
  });
}

/** `a === b` in time that does not depend on where they differ. */
function sameToken(a: string, b: string): boolean {
  const x = new TextEncoder().encode(a);
  const y = new TextEncoder().encode(b);
  let diff = x.length ^ y.length;
  for (let i = 0; i < Math.max(x.length, y.length); i++) diff |= (x[i] ?? 0) ^ (y[i] ?? 0);
  return diff === 0;
}

function bearer(request: Request): string | undefined {
  const header = request.headers.get("authorization") ?? "";
  return header.startsWith("Bearer ") ? header.slice("Bearer ".length).trim() : undefined;
}

async function sha256Hex(text: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

function hashClient(address: string, salt: string): Promise<string> {
  return sha256Hex(`${salt}:${address}`);
}

/** A fresh upload token: 32 random bytes, hex. */
function randomToken(): string {
  return [...crypto.getRandomValues(new Uint8Array(32))]
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

export interface Deps {
  store: FeedbackStore;
  /** The files database's store; without it the endpoint takes no attachments. */
  files?: AttachmentStore;
  now: () => number;
  id: () => string;
  /** The upload token a report with attachments gets back. */
  token?: () => string;
}

/** The application token check shared by the report and its uploads; `undefined` passes. */
function refuseAppToken(request: Request, env: Env): Response | undefined {
  if (env.FEEDBACK_TOKEN === undefined || env.FEEDBACK_TOKEN.length === 0) return undefined;
  const token = bearer(request);
  return token === undefined || !sameToken(token, env.FEEDBACK_TOKEN)
    ? json(401, { error: "unauthorized" })
    : undefined;
}

async function submit(request: Request, env: Env, deps: Deps): Promise<Response> {
  const refused = refuseAppToken(request, env);
  if (refused !== undefined) return refused;
  if (!(request.headers.get("content-type") ?? "").toLowerCase().startsWith("application/json"))
    return json(415, { error: "unsupported_media_type" });
  const declared = Number(request.headers.get("content-length") ?? "0");
  if (declared > MAX_BODY_BYTES) return json(413, { error: "too_large" });
  const text = await request.text();
  if (new TextEncoder().encode(text).length > MAX_BODY_BYTES)
    return json(413, { error: "too_large" });
  let body: unknown;
  try {
    body = JSON.parse(text);
  } catch (_error) {
    return json(400, { error: "invalid", field: "body" });
  }
  const checked = validate(body);
  if (!checked.ok) return json(400, { error: "invalid", field: checked.field });

  const now = deps.now();
  const address = request.headers.get("cf-connecting-ip") ?? "unknown";
  const ipHash = await hashClient(address, env.IP_SALT ?? "");
  if ((await deps.store.countFrom(ipHash, now - HOUR_MS)) >= PER_CLIENT_PER_HOUR)
    return json(429, { error: "rate_limited" }, { "retry-after": "3600" });
  if ((await deps.store.countSince(now - DAY_MS)) >= TOTAL_PER_DAY)
    return json(429, { error: "rate_limited" }, { "retry-after": "3600" });

  const { attachments, ...report } = checked.feedback;
  const attachmentBytes = attachments.reduce((n, a) => n + a.size, 0);
  const files = deps.files;
  if (attachments.length > 0) {
    if (files === undefined) return json(503, { error: "attachments_unavailable" });
    // Declarations whose upload window closed unfinished hold no quota.
    await files.pruneUnfinished(now - UPLOAD_WINDOW_MS);
    const fromClient = await files.bytesFrom(ipHash, now - DAY_MS);
    if (fromClient + attachmentBytes > ATTACHMENT_BYTES_PER_CLIENT_PER_DAY)
      return json(429, { error: "rate_limited" }, { "retry-after": "3600" });
    if ((await files.totalBytes()) + attachmentBytes > ATTACHMENT_STORE_MAX_BYTES)
      return json(507, { error: "storage_full" });
  }

  const row: FeedbackRow = {
    id: deps.id(),
    created_at: now,
    ...report,
    ip_hash: ipHash,
    country: request.headers.get("cf-ipcountry"),
  };
  await deps.store.insert(row);
  if (attachments.length === 0 || files === undefined) return json(201, { id: row.id });
  const token = (deps.token ?? randomToken)();
  const tokenHash = await sha256Hex(token);
  await files.insert(
    attachments.map((a, idx) => ({
      report_id: row.id,
      idx,
      name: a.name,
      type: a.type,
      size: a.size,
      sha256: a.sha256,
      token_hash: tokenHash,
      ip_hash: ipHash,
      created_at: now,
      uploaded_at: null,
    })),
  );
  return json(201, { id: row.id, upload: { token, chunk_bytes: ATTACHMENT_CHUNK_BYTES } });
}

/** `PUT /v1/feedback/<id>/attachments/<index>/<seq>`: one chunk of a declared attachment, with the
 *  report's upload token. A retried chunk replaces itself; the attachment is complete once every
 *  chunk is in. */
async function upload(
  request: Request,
  env: Env,
  deps: Deps,
  reportId: string,
  idx: number,
  seq: number,
): Promise<Response> {
  const refused = refuseAppToken(request, env);
  if (refused !== undefined) return refused;
  const files = deps.files;
  if (files === undefined) return json(503, { error: "attachments_unavailable" });
  const attachment = await files.get(reportId, idx);
  const token = request.headers.get("x-upload-token") ?? "";
  if (attachment === undefined || !sameToken(await sha256Hex(token), attachment.token_hash))
    return json(404, { error: "not_found" });
  if (attachment.uploaded_at !== null) return json(409, { error: "complete" });
  const now = deps.now();
  if (now > attachment.created_at + UPLOAD_WINDOW_MS) return json(410, { error: "expired" });
  const chunks = chunkCount(attachment.size);
  if (seq >= chunks) return json(400, { error: "invalid", field: "seq" });
  const expected =
    seq === chunks - 1 ? attachment.size - seq * ATTACHMENT_CHUNK_BYTES : ATTACHMENT_CHUNK_BYTES;
  const declared = Number(request.headers.get("content-length") ?? String(expected));
  if (declared !== expected) return json(400, { error: "invalid", field: "length" });
  const data = new Uint8Array(await request.arrayBuffer());
  if (data.length !== expected) return json(400, { error: "invalid", field: "length" });
  const received = await files.putChunk(reportId, idx, seq, data);
  const complete = received >= chunks;
  if (complete) await files.markUploaded(reportId, idx, now);
  return json(200, { received, complete });
}

/** What a reader sees of an attachment (never its token hash or the client address hash). */
function attachmentView(row: AttachmentRow) {
  return {
    index: row.idx,
    name: row.name,
    type: row.type,
    size: row.size,
    sha256: row.sha256,
    complete: row.uploaded_at !== null,
  };
}

/** The admin token check of the reading routes; `undefined` passes. */
function refuseAdmin(request: Request, env: Env): Response | undefined {
  if (env.ADMIN_TOKEN === undefined || env.ADMIN_TOKEN.length === 0)
    return json(404, { error: "not_found" });
  const token = bearer(request);
  return token === undefined || !sameToken(token, env.ADMIN_TOKEN)
    ? json(401, { error: "unauthorized" })
    : undefined;
}

/** `GET /v1/feedback/<id>/attachments/<index>`: a complete attachment's bytes, for the owner. */
async function download(
  request: Request,
  env: Env,
  deps: Deps,
  reportId: string,
  idx: number,
): Promise<Response> {
  const refused = refuseAdmin(request, env);
  if (refused !== undefined) return refused;
  const attachment = await deps.files?.get(reportId, idx);
  if (attachment === undefined) return json(404, { error: "not_found" });
  if (attachment.uploaded_at === null) return json(409, { error: "incomplete" });
  const chunks = (await deps.files?.read(reportId, idx)) ?? [];
  const body = new Uint8Array(chunks.reduce((n, c) => n + c.length, 0));
  let at = 0;
  for (const chunk of chunks) {
    body.set(chunk, at);
    at += chunk.length;
  }
  return new Response(body, {
    headers: {
      "content-type": attachment.type,
      "content-disposition": `attachment; filename*=UTF-8''${encodeURIComponent(attachment.name)}`,
      "x-content-type-options": "nosniff",
    },
  });
}

async function list(request: Request, env: Env, deps: Deps): Promise<Response> {
  const refused = refuseAdmin(request, env);
  if (refused !== undefined) return refused;
  const url = new URL(request.url);
  const limit = Math.min(
    LIST_LIMIT_MAX,
    Math.max(1, Number.parseInt(url.searchParams.get("limit") ?? "50", 10) || 50),
  );
  const before = Number.parseInt(url.searchParams.get("before") ?? "", 10);
  const rows = await deps.store.list(limit, Number.isFinite(before) ? before : deps.now() + 1);
  const attachments = (await deps.files?.listFor(rows.map((r) => r.id))) ?? [];
  // The address hash is for the rate limit, not for readers.
  return json(200, {
    items: rows.map(({ ip_hash: _hash, ...rest }) => ({
      ...rest,
      attachments: attachments.filter((a) => a.report_id === rest.id).map(attachmentView),
    })),
  });
}

const CHUNK_PATH = /^\/v1\/feedback\/([0-9A-Za-z-]{1,64})\/attachments\/(\d)\/(\d{1,3})$/u;
const ATTACHMENT_PATH = /^\/v1\/feedback\/([0-9A-Za-z-]{1,64})\/attachments\/(\d)$/u;

/** Routes one request; `deps` are the Worker's own unless a test passes its own. */
export async function handle(request: Request, env: Env, deps: Deps): Promise<Response> {
  const { pathname } = new URL(request.url);
  if (pathname === "/healthz") return new Response("ok");
  const chunk = CHUNK_PATH.exec(pathname);
  if (chunk !== null) {
    if (request.method !== "PUT")
      return json(405, { error: "method_not_allowed" }, { allow: "PUT" });
    return upload(request, env, deps, chunk[1] ?? "", Number(chunk[2]), Number(chunk[3]));
  }
  const attachment = ATTACHMENT_PATH.exec(pathname);
  if (attachment !== null) {
    if (request.method !== "GET")
      return json(405, { error: "method_not_allowed" }, { allow: "GET" });
    return download(request, env, deps, attachment[1] ?? "", Number(attachment[2]));
  }
  if (pathname !== "/v1/feedback") return json(404, { error: "not_found" });
  if (request.method === "POST") return submit(request, env, deps);
  if (request.method === "GET") return list(request, env, deps);
  return json(405, { error: "method_not_allowed" }, { allow: "GET, POST" });
}

export default {
  fetch(request: Request, env: Env): Promise<Response> {
    return handle(request, env, {
      store: d1Store(env.DB),
      files: env.FILES === undefined ? undefined : d1Files(env.FILES),
      now: () => Date.now(),
      id: () => crypto.randomUUID(),
    });
  },
};
