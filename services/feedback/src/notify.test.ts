import {
  NOTICE_MESSAGE_CHARS,
  NOTIFY_TIMEOUT_MS,
  type Notice,
  TELEGRAM_API,
  noticeText,
  telegramNotifier,
} from "./notify";

/** What a stubbed fetch was asked for. */
const urlOf = (input: RequestInfo | URL) =>
  typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
const bodyOf = (init: RequestInit = {}) => (typeof init.body === "string" ? init.body : "");

const NOTICE: Notice = {
  id: "0d1c7a52-1111-4222-8333-944455556666",
  kind: "bug",
  message: "松开热键后没有粘贴",
  contact: true,
  diagnostics: { app_version: "0.1.7", os: "windows", arch: "x86_64", locale: "zh-CN" },
  attachments: 2,
  country: "CN",
};

describe("noticeText", () => {
  it("names the kind, quotes the message and says what the report holds, without the contact", () => {
    const text = noticeText(NOTICE, "https://admin.example.test/admin");
    expect(text).toBe(
      [
        "Voltip 新反馈 · 问题",
        "松开热键后没有粘贴",
        "",
        "0.1.7 · windows x86_64 · zh-CN · CN",
        "附件 2 个 · 留了联系方式",
        "https://admin.example.test/admin",
        `id ${NOTICE.id}`,
      ].join("\n"),
    );
  });

  it("cuts a long message and leaves out the lines it has nothing for", () => {
    const long = `${"长".repeat(NOTICE_MESSAGE_CHARS)}尾巴`;
    const text = noticeText({
      ...NOTICE,
      kind: "idea",
      message: `  ${long}  `,
      contact: false,
      diagnostics: {},
      attachments: 0,
      country: null,
    });
    expect(text).toBe(
      ["Voltip 新反馈 · 建议", `${"长".repeat(NOTICE_MESSAGE_CHARS)}…`, "", `id ${NOTICE.id}`].join(
        "\n",
      ),
    );
    expect(noticeText({ ...NOTICE, kind: "other" })).toMatch(/^Voltip 新反馈 · 其他\n/u);
  });
});

describe("telegramNotifier", () => {
  it("is off until both the bot and the chat are set", () => {
    expect(telegramNotifier({})).toBeUndefined();
    expect(telegramNotifier({ TELEGRAM_BOT_TOKEN: "123:abc" })).toBeUndefined();
    expect(telegramNotifier({ TELEGRAM_CHAT_ID: "42" })).toBeUndefined();
    expect(telegramNotifier({ TELEGRAM_BOT_TOKEN: "", TELEGRAM_CHAT_ID: "42" })).toBeUndefined();
  });

  it("posts the notice as plain text to the chat, with a deadline", async () => {
    const calls: { url: string; init: RequestInit }[] = [];
    const send = ((input: RequestInfo | URL, init: RequestInit = {}) => {
      calls.push({ url: urlOf(input), init });
      return Promise.resolve(Response.json({ ok: true }));
    }) as typeof fetch;
    const timeout = vi.spyOn(AbortSignal, "timeout");
    const notify = telegramNotifier(
      { TELEGRAM_BOT_TOKEN: "123:abc", TELEGRAM_CHAT_ID: "42", ADMIN_URL: "https://a.test/admin" },
      send,
    );
    await notify?.(NOTICE);
    expect(calls).toHaveLength(1);
    expect(calls[0]?.url).toBe(`${TELEGRAM_API}/bot123:abc/sendMessage`);
    expect(calls[0]?.init.method).toBe("POST");
    const body = JSON.parse(bodyOf(calls[0]?.init)) as Record<string, unknown>;
    expect(body).toEqual({
      chat_id: "42",
      text: noticeText(NOTICE, "https://a.test/admin"),
      disable_web_page_preview: true,
    });
    expect(body).not.toHaveProperty("parse_mode");
    expect(timeout).toHaveBeenCalledWith(NOTIFY_TIMEOUT_MS);
  });

  it("leaves the link out when no admin page is set", async () => {
    let text = "";
    const send = ((_input: RequestInfo | URL, init: RequestInit = {}) => {
      text = (JSON.parse(bodyOf(init)) as { text: string }).text;
      return Promise.resolve(Response.json({ ok: true }));
    }) as typeof fetch;
    await telegramNotifier(
      { TELEGRAM_BOT_TOKEN: "t", TELEGRAM_CHAT_ID: "1", ADMIN_URL: "" },
      send,
    )?.(NOTICE);
    expect(text).toBe(noticeText(NOTICE));
    expect(text).not.toContain("http");
  });

  it("fails with the status only, never the URL that holds the token", async () => {
    const send = (() => Promise.resolve(new Response("nope", { status: 403 }))) as typeof fetch;
    const notify = telegramNotifier(
      { TELEGRAM_BOT_TOKEN: "123:secret", TELEGRAM_CHAT_ID: "42" },
      send,
    );
    const failure = await notify?.(NOTICE).then(
      () => undefined,
      (error: unknown) => error,
    );
    expect(failure).toBeInstanceOf(Error);
    expect((failure as Error).message).toBe("telegram answered 403");
    expect((failure as Error).message).not.toContain("secret");
  });
});
