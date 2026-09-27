import { DIAGNOSTIC_KEYS, type Diagnostics, KINDS, type Kind } from "./validate";

/** One stored piece of feedback. The client address is kept only as a salted hash, for the rate
 *  limit. */
export interface FeedbackRow {
  id: string;
  created_at: number;
  kind: Kind;
  message: string;
  contact: string | null;
  diagnostics: Diagnostics;
  ip_hash: string;
  country: string | null;
}

/** What the handler needs from storage; `d1Store` is the Worker's, tests use `memoryStore`. */
export interface FeedbackStore {
  insert(row: FeedbackRow): Promise<void>;
  /** Rows from `ipHash` at or after `since` (Unix ms). */
  countFrom(ipHash: string, since: number): Promise<number>;
  /** All rows at or after `since`. */
  countSince(since: number): Promise<number>;
  /** Newest first, strictly older than `before`. */
  list(limit: number, before: number): Promise<FeedbackRow[]>;
}

/** The subset of Cloudflare's D1 binding this module uses. Rows come back as `unknown` and are
 *  read field by field: the database is a storage boundary. */
export interface D1Statement {
  bind(...values: unknown[]): D1Statement;
  run(): Promise<unknown>;
  first(): Promise<unknown>;
  all(): Promise<{ results?: unknown[] }>;
}
export interface D1Like {
  prepare(query: string): D1Statement;
}

function field(row: unknown, key: string): unknown {
  return typeof row === "object" && row !== null ? Reflect.get(row, key) : undefined;
}

function text(row: unknown, key: string): string | null {
  const value = field(row, key);
  return typeof value === "string" ? value : null;
}

function count(row: unknown): number {
  const n = field(row, "n");
  return typeof n === "number" ? n : 0;
}

function isKind(value: string | null): value is Kind {
  return value !== null && (KINDS as readonly string[]).includes(value);
}

/** The stored JSON back as diagnostics, keeping only the known string fields. */
function parseDiagnostics(raw: string): Diagnostics {
  let value: unknown;
  try {
    value = JSON.parse(raw);
  } catch (_error) {
    return {};
  }
  const out: Diagnostics = {};
  for (const key of DIAGNOSTIC_KEYS) {
    const entry = field(value, key);
    if (typeof entry === "string") out[key] = entry;
  }
  return out;
}

/** A stored row, or nothing when a column is not what the schema says. */
function readRow(raw: unknown): FeedbackRow | undefined {
  const id = text(raw, "id");
  const created = field(raw, "created_at");
  const kind = text(raw, "kind");
  const message = text(raw, "message");
  const ipHash = text(raw, "ip_hash");
  if (id === null || typeof created !== "number" || !isKind(kind)) return undefined;
  if (message === null || ipHash === null) return undefined;
  return {
    id,
    created_at: created,
    kind,
    message,
    contact: text(raw, "contact"),
    diagnostics: parseDiagnostics(text(raw, "diagnostics") ?? ""),
    ip_hash: ipHash,
    country: text(raw, "country"),
  };
}

/** Storage over the `feedback` table (schema.sql). */
export function d1Store(db: D1Like): FeedbackStore {
  return {
    async insert(row) {
      await db
        .prepare(
          "INSERT INTO feedback (id, created_at, kind, message, contact, diagnostics, ip_hash, country) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(
          row.id,
          row.created_at,
          row.kind,
          row.message,
          row.contact,
          JSON.stringify(row.diagnostics),
          row.ip_hash,
          row.country,
        )
        .run();
    },
    async countFrom(ipHash, since) {
      return count(
        await db
          .prepare("SELECT COUNT(*) AS n FROM feedback WHERE ip_hash = ? AND created_at >= ?")
          .bind(ipHash, since)
          .first(),
      );
    },
    async countSince(since) {
      return count(
        await db
          .prepare("SELECT COUNT(*) AS n FROM feedback WHERE created_at >= ?")
          .bind(since)
          .first(),
      );
    },
    async list(limit, before) {
      const { results = [] } = await db
        .prepare(
          "SELECT id, created_at, kind, message, contact, diagnostics, ip_hash, country FROM feedback WHERE created_at < ? ORDER BY created_at DESC LIMIT ?",
        )
        .bind(before, limit)
        .all();
      return results.map(readRow).filter((r) => r !== undefined);
    },
  };
}

/** An in-memory store with the same semantics (tests, local runs). */
export function memoryStore(rows: FeedbackRow[] = []): FeedbackStore & { rows: FeedbackRow[] } {
  return {
    rows,
    insert(row) {
      rows.push(row);
      return Promise.resolve();
    },
    countFrom(ipHash, since) {
      return Promise.resolve(
        rows.filter((r) => r.ip_hash === ipHash && r.created_at >= since).length,
      );
    },
    countSince(since) {
      return Promise.resolve(rows.filter((r) => r.created_at >= since).length);
    },
    list(limit, before) {
      return Promise.resolve(
        rows
          .filter((r) => r.created_at < before)
          .toSorted((a, b) => b.created_at - a.created_at)
          .slice(0, limit),
      );
    },
  };
}
