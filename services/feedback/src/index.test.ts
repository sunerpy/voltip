import worker, {
  type Deps,
  type Env,
  LIST_LIMIT_MAX,
  MAX_BODY_BYTES,
  PER_CLIENT_PER_HOUR,
  TOTAL_PER_DAY,
  handle,
} from "./index";
import { type D1Like, type D1Statement, type FeedbackRow, d1Store, memoryStore } from "./store";
import { MAX_CONTACT_CHARS, MAX_MESSAGE_CHARS, validate } from "./validate";

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
      },
    });
    expect(validate({ kind: "idea", message: "x", contact: "  " })).toEqual({
      ok: true,
      feedback: { kind: "idea", message: "x", contact: null, diagnostics: {} },
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
    const put = await handle(new Request(`${URL_BASE}/v1/feedback`, { method: "PUT" }), ENV, d);
    expect(put.status).toBe(405);
    expect(put.headers.get("allow")).toBe("GET, POST");
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
