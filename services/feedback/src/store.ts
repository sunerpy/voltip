import {
  ATTACHMENT_TYPES,
  type AttachmentType,
  DIAGNOSTIC_KEYS,
  type Diagnostics,
  KINDS,
  type Kind,
} from "./validate";

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

// ---------------------------------------------------------------------------- attachments

/** One declared attachment of a report, kept in the files database (schema-files.sql) so the
 *  reports never share a database size limit with the bytes. `token_hash` is the SHA-256 of the
 *  upload token the report's answer carried; `uploaded_at` is set once every chunk is in. */
export interface AttachmentRow {
  report_id: string;
  idx: number;
  name: string;
  type: AttachmentType;
  size: number;
  sha256: string;
  token_hash: string;
  ip_hash: string;
  created_at: number;
  uploaded_at: number | null;
}

export interface AttachmentStore {
  insert(rows: AttachmentRow[]): Promise<void>;
  /** Declared bytes from `ipHash` at or after `since` (Unix ms). */
  bytesFrom(ipHash: string, since: number): Promise<number>;
  /** Declared bytes of every attachment kept. */
  totalBytes(): Promise<number>;
  get(reportId: string, idx: number): Promise<AttachmentRow | undefined>;
  /** Store chunk `seq` (a retried chunk replaces itself); answers how many chunks the attachment
   *  has now. */
  putChunk(reportId: string, idx: number, seq: number, data: Uint8Array): Promise<number>;
  markUploaded(reportId: string, idx: number, at: number): Promise<void>;
  /** Forget the attachments declared before `before` whose bytes never all arrived, and their
   *  chunks: past the upload window they cannot complete, and must not hold quota. */
  pruneUnfinished(before: number): Promise<void>;
  /** The attachments of `reportIds`, in report and index order. */
  listFor(reportIds: readonly string[]): Promise<AttachmentRow[]>;
  /** The chunks of one attachment, in order. */
  read(reportId: string, idx: number): Promise<Uint8Array[]>;
}

function isAttachmentType(value: string | null): value is AttachmentType {
  return value !== null && (ATTACHMENT_TYPES as readonly string[]).includes(value);
}

function number(row: unknown, key: string): number | null {
  const value = field(row, key);
  return typeof value === "number" ? value : null;
}

function readAttachment(raw: unknown): AttachmentRow | undefined {
  const reportId = text(raw, "report_id");
  const idx = number(raw, "idx");
  const name = text(raw, "name");
  const type = text(raw, "type");
  const size = number(raw, "size");
  const sha256 = text(raw, "sha256");
  const tokenHash = text(raw, "token_hash");
  const ipHash = text(raw, "ip_hash");
  const created = number(raw, "created_at");
  if (reportId === null || idx === null || name === null || !isAttachmentType(type))
    return undefined;
  if (size === null || sha256 === null || tokenHash === null || ipHash === null || created === null)
    return undefined;
  return {
    report_id: reportId,
    idx,
    name,
    type,
    size,
    sha256,
    token_hash: tokenHash,
    ip_hash: ipHash,
    created_at: created,
    uploaded_at: number(raw, "uploaded_at"),
  };
}

/** A BLOB as D1 hands it back (an ArrayBuffer, or an array of byte values from older runtimes). */
function bytes(value: unknown): Uint8Array | undefined {
  if (value instanceof Uint8Array) return value;
  if (value instanceof ArrayBuffer) return new Uint8Array(value);
  if (Array.isArray(value) && value.every((b) => typeof b === "number"))
    return Uint8Array.from(value);
  return undefined;
}

const ATTACHMENT_COLUMNS =
  "report_id, idx, name, type, size, sha256, token_hash, ip_hash, created_at, uploaded_at";
/** D1 allows 100 bound parameters a query; report ids are listed in batches below that. */
const IN_BATCH = 90;

/** Storage over the files database's `attachment` and `attachment_chunk` tables. */
export function d1Files(db: D1Like): AttachmentStore {
  return {
    async insert(rows) {
      await Promise.all(
        rows.map((row) =>
          db
            .prepare(
              `INSERT INTO attachment (${ATTACHMENT_COLUMNS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
            )
            .bind(
              row.report_id,
              row.idx,
              row.name,
              row.type,
              row.size,
              row.sha256,
              row.token_hash,
              row.ip_hash,
              row.created_at,
              row.uploaded_at,
            )
            .run(),
        ),
      );
    },
    async bytesFrom(ipHash, since) {
      const row = await db
        .prepare(
          "SELECT COALESCE(SUM(size), 0) AS n FROM attachment WHERE ip_hash = ? AND created_at >= ?",
        )
        .bind(ipHash, since)
        .first();
      return count(row);
    },
    async totalBytes() {
      return count(await db.prepare("SELECT COALESCE(SUM(size), 0) AS n FROM attachment").first());
    },
    async get(reportId, idx) {
      const row = await db
        .prepare(`SELECT ${ATTACHMENT_COLUMNS} FROM attachment WHERE report_id = ? AND idx = ?`)
        .bind(reportId, idx)
        .first();
      return readAttachment(row);
    },
    async putChunk(reportId, idx, seq, data) {
      await db
        .prepare(
          "INSERT OR REPLACE INTO attachment_chunk (report_id, idx, seq, data) VALUES (?, ?, ?, ?)",
        )
        .bind(reportId, idx, seq, data)
        .run();
      return count(
        await db
          .prepare("SELECT COUNT(*) AS n FROM attachment_chunk WHERE report_id = ? AND idx = ?")
          .bind(reportId, idx)
          .first(),
      );
    },
    async markUploaded(reportId, idx, at) {
      await db
        .prepare("UPDATE attachment SET uploaded_at = ? WHERE report_id = ? AND idx = ?")
        .bind(at, reportId, idx)
        .run();
    },
    async pruneUnfinished(before) {
      await db
        .prepare(
          "DELETE FROM attachment_chunk WHERE EXISTS (SELECT 1 FROM attachment a WHERE a.report_id = attachment_chunk.report_id AND a.idx = attachment_chunk.idx AND a.uploaded_at IS NULL AND a.created_at < ?)",
        )
        .bind(before)
        .run();
      await db
        .prepare("DELETE FROM attachment WHERE uploaded_at IS NULL AND created_at < ?")
        .bind(before)
        .run();
    },
    async listFor(reportIds) {
      const batches: string[][] = [];
      for (let i = 0; i < reportIds.length; i += IN_BATCH)
        batches.push(reportIds.slice(i, i + IN_BATCH));
      const answers = await Promise.all(
        batches.map((batch) =>
          db
            .prepare(
              `SELECT ${ATTACHMENT_COLUMNS} FROM attachment WHERE report_id IN (${batch.map(() => "?").join(", ")}) ORDER BY report_id, idx`,
            )
            .bind(...batch)
            .all(),
        ),
      );
      return answers.flatMap(({ results = [] }) =>
        results.map(readAttachment).filter((r) => r !== undefined),
      );
    },
    async read(reportId, idx) {
      const { results = [] } = await db
        .prepare("SELECT data FROM attachment_chunk WHERE report_id = ? AND idx = ? ORDER BY seq")
        .bind(reportId, idx)
        .all();
      return results.map((raw) => bytes(field(raw, "data"))).filter((b) => b !== undefined);
    },
  };
}

/** The in-memory store's key of one chunk. */
const chunkKey = (reportId: string, idx: number, seq: number) => `${reportId}/${idx}/${seq}`;

/** An in-memory files store with the same semantics (tests, local runs). */
export function memoryFiles(): AttachmentStore & {
  rows: AttachmentRow[];
  chunks: Map<string, Uint8Array>;
} {
  const rows: AttachmentRow[] = [];
  const chunks = new Map<string, Uint8Array>();
  const find = (reportId: string, idx: number) =>
    rows.find((r) => r.report_id === reportId && r.idx === idx);
  const own = (reportId: string, idx: number) =>
    [...chunks.entries()]
      .filter(([k]) => k.startsWith(`${reportId}/${idx}/`))
      .map(([k, data]) => ({ seq: Number(k.split("/")[2]), data }))
      .toSorted((a, b) => a.seq - b.seq);
  return {
    rows,
    chunks,
    insert(next) {
      rows.push(...next.map((r) => ({ ...r })));
      return Promise.resolve();
    },
    bytesFrom(ipHash, since) {
      return Promise.resolve(
        rows
          .filter((r) => r.ip_hash === ipHash && r.created_at >= since)
          .reduce((n, r) => n + r.size, 0),
      );
    },
    totalBytes() {
      return Promise.resolve(rows.reduce((n, r) => n + r.size, 0));
    },
    get(reportId, idx) {
      const row = find(reportId, idx);
      return Promise.resolve(row === undefined ? undefined : { ...row });
    },
    putChunk(reportId, idx, seq, data) {
      chunks.set(chunkKey(reportId, idx, seq), data.slice());
      return Promise.resolve(own(reportId, idx).length);
    },
    markUploaded(reportId, idx, at) {
      const row = find(reportId, idx);
      if (row !== undefined) row.uploaded_at = at;
      return Promise.resolve();
    },
    pruneUnfinished(before) {
      const stale = rows.filter((r) => r.uploaded_at === null && r.created_at < before);
      for (const row of stale) {
        for (const { seq } of own(row.report_id, row.idx))
          chunks.delete(chunkKey(row.report_id, row.idx, seq));
        rows.splice(rows.indexOf(row), 1);
      }
      return Promise.resolve();
    },
    listFor(reportIds) {
      return Promise.resolve(
        rows
          .filter((r) => reportIds.includes(r.report_id))
          .toSorted((a, b) => a.report_id.localeCompare(b.report_id) || a.idx - b.idx)
          .map((r) => ({ ...r })),
      );
    },
    read(reportId, idx) {
      return Promise.resolve(own(reportId, idx).map((c) => c.data));
    },
  };
}
