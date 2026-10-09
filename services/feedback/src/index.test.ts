import worker, {
  ATTACHMENT_BYTES_PER_CLIENT_PER_DAY,
  ATTACHMENT_STORE_MAX_BYTES,
  type Deps,
  type Env,
  LIST_LIMIT_MAX,
  MAX_BODY_BYTES,
  PER_CLIENT_PER_HOUR,
  TOTAL_PER_DAY,
  UPLOAD_WINDOW_MS,
  handle,
} from "./index";
import {
  type D1Like,
  type D1Statement,
  type FeedbackRow,
  d1Files,
  d1Store,
  memoryFiles,
  memoryStore,
} from "./store";
import {
  ATTACHMENT_CHUNK_BYTES,
  MAX_ATTACHMENTS,
  MAX_ATTACHMENT_TOTAL_BYTES,
  MAX_CONTACT_CHARS,
  MAX_IMAGE_BYTES,
  MAX_MESSAGE_CHARS,
  MAX_VIDEO_BYTES,
  chunkCount,
  validate,
} from "./validate";

const NOW = 1_790_000_000_000;
const URL_BASE = "https://feedback.example.test";

function deps(rows: FeedbackRow[] = []): Deps & { store: ReturnType<typeof memoryStore> } {
  let n = 0;
  return { store: memoryStore(rows), now: () => NOW, id: () => `id-${++n}` };
}

const ENV: Env = {
  DB: { prepare: () => fakeStatement() },
  FEEDBACK_TOKEN: "app-token",
  IP_SALT: "salt",
};

function post(body: unknown, headers: Record<string, string> = {}): Request {
  return new Request(`${URL_BASE}/v1/feedback`, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      authorization: "Bearer app-token",
      "cf-connecting-ip": "203.0.113.7",
      "cf-ipcountry": "CN",
      ...headers,
    },
    body: typeof body === "string" ? body : JSON.stringify(body),
  });
}

const GOOD = {
  kind: "bug",
  message: "  松开热键后没有粘贴  ",
  contact: " me@example.test ",
  diagnostics: {
    app_version: "0.0.1",
    os: "windows",
    arch: "x86_64",
    locale: "zh-CN",
    asr_provider: "builtin",
    output_mode: "whole_take",
    something_new: "dropped",
  },
};

function fakeStatement(
  log: { sql: string; binds: unknown[] }[] = [],
  answer: unknown = null,
): D1Statement {
  const stmt: D1Statement = {
    bind: (...values) => {
      const last = log.at(-1);
      if (last) last.binds = values;
      return stmt;
    },
    run: () => Promise.resolve({}),
    first: () => Promise.resolve(answer),
    all: () => Promise.resolve({ results: Array.isArray(answer) ? answer : undefined }),
  };
  return stmt;
}

describe("validate", () => {
  it("accepts a report, trims it and keeps only the known diagnostics", () => {
    const checked = validate(GOOD);
    expect(checked).toEqual({
      ok: true,
      feedback: {
        kind: "bug",
        message: "松开热键后没有粘贴",
        contact: "me@example.test",
        diagnostics: {
          app_version: "0.0.1",
          os: "windows",
          arch: "x86_64",
          locale: "zh-CN",
          asr_provider: "builtin",
          output_mode: "whole_take",
        },
        attachments: [],
      },
    });
    expect(validate({ kind: "idea", message: "x", contact: "  " })).toEqual({
      ok: true,
      feedback: { kind: "idea", message: "x", contact: null, diagnostics: {}, attachments: [] },
    });
    expect(
      validate({ kind: "other", message: "x", contact: null, diagnostics: { os: null } }),
    ).toMatchObject({ ok: true });
  });

  it("names the first bad field", () => {
    const cases: [unknown, string][] = [
      ["nope", "body"],
      [[], "body"],
      [{ ...GOOD, kind: "praise" }, "kind"],
      [{ ...GOOD, message: "   " }, "message"],
      [{ ...GOOD, message: 7 }, "message"],
      [{ ...GOOD, message: "长".repeat(MAX_MESSAGE_CHARS + 1) }, "message"],
      [{ ...GOOD, contact: 3 }, "contact"],
      [{ ...GOOD, contact: "a".repeat(MAX_CONTACT_CHARS + 1) }, "contact"],
      [{ ...GOOD, diagnostics: "windows" }, "diagnostics"],
      [{ ...GOOD, diagnostics: { os: 1 } }, "diagnostics.os"],
      [{ ...GOOD, diagnostics: { os: "<script>" } }, "diagnostics.os"],
      [{ ...GOOD, diagnostics: { os: "w".repeat(65) } }, "diagnostics.os"],
    ];
    for (const [body, field] of cases) expect(validate(body)).toEqual({ ok: false, field });
    // The limit counts characters as the dialog does, not bytes: 5000 CJK characters are fine.
    expect(validate({ ...GOOD, message: "长".repeat(MAX_MESSAGE_CHARS) })).toMatchObject({
      ok: true,
    });
  });
});

describe("handle", () => {
  it("stores a report with a hashed client address and answers its id", async () => {
    const d = deps();
    const res = await handle(post(GOOD), ENV, d);
    expect(res.status).toBe(201);
    expect(await res.json()).toEqual({ id: "id-1" });
    const [row] = d.store.rows;
    expect(row).toMatchObject({
      id: "id-1",
      created_at: NOW,
      kind: "bug",
      message: "松开热键后没有粘贴",
      country: "CN",
    });
    expect(row?.ip_hash).toMatch(/^[0-9a-f]{64}$/);
    expect(JSON.stringify(row)).not.toContain("203.0.113.7");
    expect(row?.diagnostics).not.toHaveProperty("something_new");
  });

  it("refuses a missing or wrong app token, and takes any client when the endpoint has none", async () => {
    const d = deps();
    expect((await handle(post(GOOD, { authorization: "" }), ENV, d)).status).toBe(401);
    expect((await handle(post(GOOD, { authorization: "Bearer app-tokeN" }), ENV, d)).status).toBe(
      401,
    );
    expect((await handle(post(GOOD, { authorization: "Bearer app" }), ENV, d)).status).toBe(401);
    expect(d.store.rows).toHaveLength(0);
    const open: Env = { DB: ENV.DB };
    expect((await handle(post(GOOD, { authorization: "" }), open, d)).status).toBe(201);
  });

  it("refuses other media types, oversized bodies, bad JSON and bad fields without storing", async () => {
    const d = deps();
    expect((await handle(post(GOOD, { "content-type": "text/plain" }), ENV, d)).status).toBe(415);
    expect(
      (await handle(post(GOOD, { "content-length": String(MAX_BODY_BYTES + 1) }), ENV, d)).status,
    ).toBe(413);
    const huge = JSON.stringify({ ...GOOD, message: "x".repeat(MAX_BODY_BYTES) });
    expect((await handle(post(huge), ENV, d)).status).toBe(413);
    const bad = await handle(post("{not json"), ENV, d);
    expect(bad.status).toBe(400);
    expect(await bad.json()).toEqual({ error: "invalid", field: "body" });
    const field = await handle(post({ ...GOOD, kind: "x" }), ENV, d);
    expect(await field.json()).toEqual({ error: "invalid", field: "kind" });
    expect(d.store.rows).toHaveLength(0);
  });

  it(`limits one client to ${PER_CLIENT_PER_HOUR} an hour and everyone to ${TOTAL_PER_DAY} a day`, async () => {
    const d = deps();
    for (let i = 0; i < PER_CLIENT_PER_HOUR; i++)
      expect((await handle(post(GOOD), ENV, d)).status).toBe(201);
    const limited = await handle(post(GOOD), ENV, d);
    expect(limited.status).toBe(429);
    expect(limited.headers.get("retry-after")).toBe("3600");
    // Another address is still welcome; one without a known address shares one bucket.
    expect((await handle(post(GOOD, { "cf-connecting-ip": "198.51.100.1" }), ENV, d)).status).toBe(
      201,
    );
    const full = deps(
      Array.from({ length: TOTAL_PER_DAY }, (_, i) => ({
        id: `old-${i}`,
        created_at: NOW - 1000,
        kind: "other" as const,
        message: "m",
        contact: null,
        diagnostics: {},
        ip_hash: `h${i}`,
        country: null,
      })),
    );
    expect((await handle(post(GOOD, { "cf-connecting-ip": "" }), ENV, full)).status).toBe(429);
  });

  it("lets the owner read feedback back, newest first, without the address hash", async () => {
    const d = deps();
    await handle(post(GOOD), ENV, d);
    await handle(post({ ...GOOD, kind: "idea", message: "second" }), ENV, {
      ...d,
      now: () => NOW + 5,
    });
    const get = (query: string, token?: string) =>
      new Request(`${URL_BASE}/v1/feedback${query}`, {
        headers: token === undefined ? {} : { authorization: `Bearer ${token}` },
      });
    expect((await handle(get(""), ENV, d)).status).toBe(404);
    const admin: Env = { ...ENV, ADMIN_TOKEN: "admin" };
    expect((await handle(get(""), admin, d)).status).toBe(401);
    expect((await handle(get("", "nope"), admin, d)).status).toBe(401);
    const res = await handle(get("?limit=1", "admin"), admin, { ...d, now: () => NOW + 10 });
    const body = (await res.json()) as { items: Record<string, unknown>[] };
    expect(body.items).toHaveLength(1);
    expect(body.items[0]).toMatchObject({ kind: "idea", message: "second" });
    expect(body.items[0]).not.toHaveProperty("ip_hash");
    const older = (await (
      await handle(get(`?before=${NOW + 5}&limit=999`, "admin"), admin, d)
    ).json()) as {
      items: unknown[];
    };
    expect(older.items).toHaveLength(1);
    expect(LIST_LIMIT_MAX).toBeLessThan(999);
    const junk = (await (
      await handle(get("?limit=abc", "admin"), admin, { ...d, now: () => NOW + 10 })
    ).json()) as {
      items: unknown[];
    };
    expect(junk.items).toHaveLength(2);
  });

  it("answers the health check, and 404 / 405 for everything else", async () => {
    const d = deps();
    const health = await handle(new Request(`${URL_BASE}/healthz`), ENV, d);
    expect(await health.text()).toBe("ok");
    expect((await handle(new Request(`${URL_BASE}/`), ENV, d)).status).toBe(404);
    const replaced = await handle(
      new Request(`${URL_BASE}/v1/feedback`, { method: "PUT" }),
      ENV,
      d,
    );
    expect(replaced.status).toBe(405);
    expect(replaced.headers.get("allow")).toBe("GET, POST");
  });
});

describe("d1Store", () => {
  it("speaks the schema's SQL with bound values and reads the rows back", async () => {
    const log: { sql: string; binds: unknown[] }[] = [];
    let answer: unknown = { n: 3 };
    const db: D1Like = {
      prepare: (sql) => {
        log.push({ sql, binds: [] });
        return fakeStatement(log, answer);
      },
    };
    const store = d1Store(db);
    const row: FeedbackRow = {
      id: "a",
      created_at: NOW,
      kind: "bug",
      message: "m",
      contact: null,
      diagnostics: { os: "linux" },
      ip_hash: "h",
      country: "DE",
    };
    await store.insert(row);
    expect(log[0]?.sql).toMatch(
      /^INSERT INTO feedback \(id, created_at, kind, message, contact, diagnostics, ip_hash, country\)/,
    );
    expect(log[0]?.binds).toEqual(["a", NOW, "bug", "m", null, '{"os":"linux"}', "h", "DE"]);
    expect(await store.countFrom("h", NOW - 1)).toBe(3);
    expect(log[1]?.binds).toEqual(["h", NOW - 1]);
    expect(await store.countSince(NOW - 2)).toBe(3);
    answer = null;
    expect(await store.countFrom("h", 0)).toBe(0);
    expect(await store.countSince(0)).toBe(0);
    answer = [
      { ...row, diagnostics: '{"os":"linux","junk":1,"arch":7}' },
      { ...row, id: "b", diagnostics: "not json" },
      { ...row, id: "c", diagnostics: "42", contact: "x@example.test" },
      { ...row, id: "d", kind: "praise" },
      { ...row, id: "e", message: 7 },
      "not a row",
    ];
    const listed = await store.list(5, NOW);
    expect(listed.map((r) => r.id)).toEqual(["a", "b", "c"]);
    expect(listed.map((r) => r.diagnostics)).toEqual([{ os: "linux" }, {}, {}]);
    expect(listed[2]?.contact).toBe("x@example.test");
    expect(log.at(-1)?.binds).toEqual([NOW, 5]);
    answer = undefined;
    expect(await store.list(5, NOW)).toEqual([]);
  });

  it("is the Worker's storage", async () => {
    const log: { sql: string; binds: unknown[] }[] = [];
    const env: Env = {
      DB: {
        prepare: (sql) => {
          log.push({ sql, binds: [] });
          return fakeStatement(log, { n: 0 });
        },
      },
    };
    const res = await worker.fetch(post(GOOD, { authorization: "" }), env);
    expect(res.status).toBe(201);
    const { id } = (await res.json()) as { id: string };
    expect(id).toMatch(/^[0-9a-f-]{36}$/);
    expect(log.map((l) => l.sql.split(" ")[0])).toEqual(["SELECT", "SELECT", "INSERT"]);
  });
});

const SHA = "a".repeat(64);
const SHOT = { name: "屏幕截图 2026-09-28.png", type: "image/png", size: 1500, sha256: SHA };
const CLIP = {
  name: "录屏.mp4",
  type: "video/mp4",
  size: ATTACHMENT_CHUNK_BYTES * 2 + 10,
  sha256: "b".repeat(64),
};

function filesDeps(rows: FeedbackRow[] = []) {
  const files = memoryFiles();
  let n = 0;
  return {
    store: memoryStore(rows),
    files,
    now: () => NOW,
    id: () => `id-${++n}`,
    token: () => "upload-token",
  } satisfies Deps;
}

function put(
  reportId: string,
  idx: number,
  seq: number,
  body: Uint8Array<ArrayBuffer>,
  headers: Record<string, string> = {},
): Request {
  return new Request(`${URL_BASE}/v1/feedback/${reportId}/attachments/${idx}/${seq}`, {
    method: "PUT",
    headers: {
      "content-type": "application/octet-stream",
      authorization: "Bearer app-token",
      "x-upload-token": "upload-token",
      ...headers,
    },
    body,
  });
}

describe("attachments", () => {
  it("validates what a report declares: types, sizes, names, digests, count and total", () => {
    const ok = validate({ ...GOOD, attachments: [SHOT, { ...CLIP, name: "  clip.mp4 " }] });
    expect(ok).toMatchObject({
      ok: true,
      feedback: { attachments: [SHOT, { ...CLIP, name: "clip.mp4" }] },
    });
    const cases: [unknown, string][] = [
      ["nope", "attachments"],
      [Array.from({ length: MAX_ATTACHMENTS + 1 }, () => SHOT), "attachments"],
      [[7], "attachments.0"],
      [[{ ...SHOT, name: " " }], "attachments.0.name"],
      [[{ ...SHOT, name: "a/b.png" }], "attachments.0.name"],
      [[{ ...SHOT, name: "x".repeat(121) }], "attachments.0.name"],
      [[{ ...SHOT, type: "application/pdf" }], "attachments.0.type"],
      [[{ ...SHOT, size: 0 }], "attachments.0.size"],
      [[{ ...SHOT, size: 1.5 }], "attachments.0.size"],
      [[{ ...SHOT, size: MAX_IMAGE_BYTES + 1 }], "attachments.0.size"],
      [[{ ...CLIP, size: MAX_VIDEO_BYTES + 1 }], "attachments.0.size"],
      [[SHOT, { ...SHOT, sha256: "xyz" }], "attachments.1.sha256"],
      [
        [
          { ...CLIP, size: MAX_VIDEO_BYTES },
          { ...SHOT, size: MAX_ATTACHMENT_TOTAL_BYTES - MAX_VIDEO_BYTES },
          { ...SHOT, size: 1 },
        ],
        "attachments",
      ],
    ];
    for (const [attachments, field] of cases)
      expect(validate({ ...GOOD, attachments })).toEqual({ ok: false, field });
    expect(chunkCount(1)).toBe(1);
    expect(chunkCount(ATTACHMENT_CHUNK_BYTES)).toBe(1);
    expect(chunkCount(ATTACHMENT_CHUNK_BYTES + 1)).toBe(2);
  });

  it("regression: a report with a screenshot and a recording gets an upload token, takes the chunks and hands them to the owner (user feedback 2026-09-28)", async () => {
    const d = filesDeps();
    const admin: Env = { ...ENV, ADMIN_TOKEN: "admin" };
    const res = await handle(post({ ...GOOD, attachments: [SHOT, CLIP] }), admin, d);
    expect(res.status).toBe(201);
    expect(await res.json()).toEqual({
      id: "id-1",
      upload: { token: "upload-token", chunk_bytes: ATTACHMENT_CHUNK_BYTES },
    });
    expect(d.files.rows.map((r) => [r.idx, r.name, r.size, r.uploaded_at])).toEqual([
      [0, SHOT.name, SHOT.size, null],
      [1, CLIP.name, CLIP.size, null],
    ]);
    // The token is kept only as a hash.
    expect(JSON.stringify(d.files.rows)).not.toContain("upload-token");
    const shot = new Uint8Array(SHOT.size).fill(7);
    const first = await handle(put("id-1", 0, 0, shot), admin, d);
    expect(await first.json()).toEqual({ received: 1, complete: true });
    // The recording, in three chunks; the middle one retried.
    const clip = new Uint8Array(CLIP.size);
    for (let i = 0; i < clip.length; i++) clip[i] = i % 251;
    const part = (seq: number) =>
      clip.slice(seq * ATTACHMENT_CHUNK_BYTES, (seq + 1) * ATTACHMENT_CHUNK_BYTES);
    expect((await handle(put("id-1", 1, 0, part(0)), admin, d)).status).toBe(200);
    expect(await (await handle(put("id-1", 1, 1, part(1)), admin, d)).json()).toEqual({
      received: 2,
      complete: false,
    });
    expect(await (await handle(put("id-1", 1, 1, part(1)), admin, d)).json()).toEqual({
      received: 2,
      complete: false,
    });
    expect(await (await handle(put("id-1", 1, 2, part(2)), admin, d)).json()).toEqual({
      received: 3,
      complete: true,
    });
    expect((await handle(put("id-1", 1, 2, part(2)), admin, d)).status).toBe(409);
    // The owner sees both, complete, and downloads the bytes back.
    const listed = (await (
      await handle(
        new Request(`${URL_BASE}/v1/feedback`, { headers: { authorization: "Bearer admin" } }),
        admin,
        { ...d, now: () => NOW + 1 },
      )
    ).json()) as { items: { attachments: Record<string, unknown>[] }[] };
    expect(listed.items[0]?.attachments).toEqual([
      {
        index: 0,
        name: SHOT.name,
        type: "image/png",
        size: SHOT.size,
        sha256: SHA,
        complete: true,
      },
      {
        index: 1,
        name: CLIP.name,
        type: "video/mp4",
        size: CLIP.size,
        sha256: CLIP.sha256,
        complete: true,
      },
    ]);
    const get = (idx: number, token = "admin") =>
      handle(
        new Request(`${URL_BASE}/v1/feedback/id-1/attachments/${idx}`, {
          headers: { authorization: `Bearer ${token}` },
        }),
        admin,
        d,
      );
    const download = await get(1);
    expect(download.headers.get("content-type")).toBe("video/mp4");
    expect(download.headers.get("content-disposition")).toBe(
      `attachment; filename*=UTF-8''${encodeURIComponent(CLIP.name)}`,
    );
    // Byte-for-byte, compared natively (a deep equal over 2 MiB takes seconds).
    expect(Buffer.from(await download.arrayBuffer()).equals(Buffer.from(clip))).toBe(true);
    expect((await get(1, "nope")).status).toBe(401);
    expect((await get(2)).status).toBe(404);
  });

  it("refuses chunks without the upload token, out of range, of the wrong length or too late", async () => {
    const d = filesDeps();
    await handle(post({ ...GOOD, attachments: [CLIP] }), ENV, d);
    const chunk = new Uint8Array(ATTACHMENT_CHUNK_BYTES);
    expect(
      (await handle(put("id-1", 0, 0, chunk, { "x-upload-token": "guess" }), ENV, d)).status,
    ).toBe(404);
    expect((await handle(put("id-1", 0, 0, chunk, { authorization: "" }), ENV, d)).status).toBe(
      401,
    );
    expect((await handle(put("id-9", 0, 0, chunk), ENV, d)).status).toBe(404);
    expect((await handle(put("id-1", 1, 0, chunk), ENV, d)).status).toBe(404);
    expect((await handle(put("id-1", 0, 3, chunk), ENV, d)).status).toBe(400);
    expect((await handle(put("id-1", 0, 0, chunk.slice(1)), ENV, d)).status).toBe(400);
    expect((await handle(put("id-1", 0, 2, new Uint8Array(11)), ENV, d)).status).toBe(400);
    const late = { ...d, now: () => NOW + UPLOAD_WINDOW_MS + 1 };
    expect((await handle(put("id-1", 0, 0, chunk), ENV, late)).status).toBe(410);
    expect(d.files.chunks.size).toBe(0);
    const wrongMethod = await handle(
      new Request(`${URL_BASE}/v1/feedback/id-1/attachments/0/0`, { method: "POST" }),
      ENV,
      d,
    );
    expect(wrongMethod.status).toBe(405);
    expect(wrongMethod.headers.get("allow")).toBe("PUT");
    const incomplete = await handle(
      new Request(`${URL_BASE}/v1/feedback/id-1/attachments/0`, {
        headers: { authorization: "Bearer admin" },
      }),
      { ...ENV, ADMIN_TOKEN: "admin" },
      d,
    );
    expect(incomplete.status).toBe(409);
  });

  it("bounds what attachments can cost: per client a day, the whole store, and no files database", async () => {
    const d = filesDeps();
    const big = { ...CLIP, size: MAX_VIDEO_BYTES };
    const perDay = Math.floor(ATTACHMENT_BYTES_PER_CLIENT_PER_DAY / MAX_VIDEO_BYTES);
    for (let i = 0; i < perDay; i++)
      expect((await handle(post({ ...GOOD, attachments: [big] }), ENV, d)).status).toBe(201);
    expect((await handle(post({ ...GOOD, attachments: [big] }), ENV, d)).status).toBe(429);
    // A report without attachments still goes through.
    expect((await handle(post(GOOD), ENV, d)).status).toBe(201);
    const full = filesDeps();
    await full.files.insert([
      {
        report_id: "old",
        idx: 0,
        name: "x.mp4",
        type: "video/mp4",
        size: ATTACHMENT_STORE_MAX_BYTES,
        sha256: SHA,
        token_hash: SHA,
        ip_hash: "other",
        created_at: 0,
        uploaded_at: 1,
      },
    ]);
    const storage = await handle(post({ ...GOOD, attachments: [SHOT] }), ENV, full);
    expect(storage.status).toBe(507);
    expect(await storage.json()).toEqual({ error: "storage_full" });
    // A declaration whose bytes never came holds no quota once its upload window closed.
    const stalled = filesDeps();
    await stalled.files.insert([
      {
        report_id: "stalled",
        idx: 0,
        name: "x.mp4",
        type: "video/mp4",
        size: ATTACHMENT_STORE_MAX_BYTES,
        sha256: SHA,
        token_hash: SHA,
        ip_hash: "other",
        created_at: NOW - UPLOAD_WINDOW_MS - 1,
        uploaded_at: null,
      },
    ]);
    await stalled.files.putChunk("stalled", 0, 0, new Uint8Array(1));
    expect((await handle(post({ ...GOOD, attachments: [SHOT] }), ENV, stalled)).status).toBe(201);
    expect(stalled.files.rows.map((r) => r.report_id)).not.toContain("stalled");
    expect([...stalled.files.chunks.keys()].some((k) => k.startsWith("stalled/"))).toBe(false);
    const none = deps();
    const unavailable = await handle(post({ ...GOOD, attachments: [SHOT] }), ENV, none);
    expect(unavailable.status).toBe(503);
    expect(none.store.rows).toHaveLength(0);
    expect((await handle(put("id-1", 0, 0, new Uint8Array(1)), ENV, none)).status).toBe(503);
  });

  it("d1Files speaks the files schema's SQL with bound values", async () => {
    const log: { sql: string; binds: unknown[] }[] = [];
    let answer: unknown = { n: 2 };
    const db: D1Like = {
      prepare: (sql) => {
        log.push({ sql, binds: [] });
        return fakeStatement(log, answer);
      },
    };
    const files = d1Files(db);
    const row = {
      report_id: "r",
      idx: 0,
      name: "a.png",
      type: "image/png" as const,
      size: 3,
      sha256: SHA,
      token_hash: "t",
      ip_hash: "h",
      created_at: NOW,
      uploaded_at: null,
    };
    await files.insert([row]);
    expect(log[0]?.sql).toMatch(/^INSERT INTO attachment \(report_id, idx, name, type, size/);
    expect(log[0]?.binds).toEqual(["r", 0, "a.png", "image/png", 3, SHA, "t", "h", NOW, null]);
    expect(await files.bytesFrom("h", 5)).toBe(2);
    expect(await files.totalBytes()).toBe(2);
    expect(await files.putChunk("r", 0, 0, new Uint8Array([1, 2, 3]))).toBe(2);
    expect(log.at(-2)?.sql).toMatch(/^INSERT OR REPLACE INTO attachment_chunk/);
    await files.markUploaded("r", 0, NOW + 1);
    expect(log.at(-1)?.binds).toEqual([NOW + 1, "r", 0]);
    await files.pruneUnfinished(NOW);
    expect(log.at(-2)?.sql).toMatch(/^DELETE FROM attachment_chunk WHERE EXISTS/);
    expect(log.at(-1)).toEqual({
      sql: "DELETE FROM attachment WHERE uploaded_at IS NULL AND created_at < ?",
      binds: [NOW],
    });
    answer = row;
    expect(await files.get("r", 0)).toEqual(row);
    answer = { ...row, type: "text/html" };
    expect(await files.get("r", 0)).toBeUndefined();
    answer = [row, { ...row, idx: "x" }];
    expect(await files.listFor(["r"])).toEqual([row]);
    expect(log.at(-1)?.sql).toMatch(/WHERE report_id IN \(\?\) ORDER BY report_id, idx$/);
    expect(await files.listFor([])).toEqual([]);
    answer = [{ data: new Uint8Array([1]).buffer }, { data: [2, 3] }, { data: "no" }];
    expect(await files.read("r", 0)).toEqual([new Uint8Array([1]), new Uint8Array([2, 3])]);
  });
});
