/** The feedback endpoint (docs/feedback.md): the desktop app's 反馈 dialog posts here, a Cloudflare
 *  Worker validates the report and stores it in D1. The app sends only what the dialog showed the
 *  user; the client address is kept as a salted hash, for the rate limit. */
import { type D1Like, type FeedbackRow, type FeedbackStore, d1Store } from "./store";
import { validate } from "./validate";

export interface Env {
  DB: D1Like;
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

async function hashClient(address: string, salt: string): Promise<string> {
  const digest = await crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(`${salt}:${address}`),
  );
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

export interface Deps {
  store: FeedbackStore;
  now: () => number;
  id: () => string;
}

async function submit(request: Request, env: Env, deps: Deps): Promise<Response> {
  if (env.FEEDBACK_TOKEN !== undefined && env.FEEDBACK_TOKEN.length > 0) {
    const token = bearer(request);
    if (token === undefined || !sameToken(token, env.FEEDBACK_TOKEN))
      return json(401, { error: "unauthorized" });
  }
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

  const row: FeedbackRow = {
    id: deps.id(),
    created_at: now,
    ...checked.feedback,
    ip_hash: ipHash,
    country: request.headers.get("cf-ipcountry"),
  };
  await deps.store.insert(row);
  return json(201, { id: row.id });
}

async function list(request: Request, env: Env, deps: Deps): Promise<Response> {
  if (env.ADMIN_TOKEN === undefined || env.ADMIN_TOKEN.length === 0)
    return json(404, { error: "not_found" });
  const token = bearer(request);
  if (token === undefined || !sameToken(token, env.ADMIN_TOKEN))
    return json(401, { error: "unauthorized" });
  const url = new URL(request.url);
  const limit = Math.min(
    LIST_LIMIT_MAX,
    Math.max(1, Number.parseInt(url.searchParams.get("limit") ?? "50", 10) || 50),
  );
  const before = Number.parseInt(url.searchParams.get("before") ?? "", 10);
  const rows = await deps.store.list(limit, Number.isFinite(before) ? before : deps.now() + 1);
  // The address hash is for the rate limit, not for readers.
  return json(200, { items: rows.map(({ ip_hash: _hash, ...rest }) => rest) });
}

/** Routes one request; `deps` are the Worker's own unless a test passes its own. */
export async function handle(request: Request, env: Env, deps: Deps): Promise<Response> {
  const { pathname } = new URL(request.url);
  if (pathname === "/healthz") return new Response("ok");
  if (pathname !== "/v1/feedback") return json(404, { error: "not_found" });
  if (request.method === "POST") return submit(request, env, deps);
  if (request.method === "GET") return list(request, env, deps);
  return json(405, { error: "method_not_allowed" }, { allow: "GET, POST" });
}

export default {
  fetch(request: Request, env: Env): Promise<Response> {
    return handle(request, env, {
      store: d1Store(env.DB),
      now: () => Date.now(),
      id: () => crypto.randomUUID(),
    });
  },
};
