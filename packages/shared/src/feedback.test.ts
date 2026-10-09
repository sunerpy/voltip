// In-app feedback (docs/feedback.md): the wire schemas mirror the shell's src/feedback.rs, both
// backends answer the two queries, and the mock plays the endpoint.
import {
  MOCK_FEEDBACK_MS,
  MockBackend,
  cleanAttachmentName,
  desktopIdentity,
} from "./mock-backend";
import {
  FEEDBACK_CONTACT_MAX,
  FEEDBACK_MAX_IMAGE_BYTES,
  FEEDBACK_MAX_VIDEO_BYTES,
  FEEDBACK_MESSAGE_MAX,
  QUERY_COMMANDS,
  defaultEngineSettings,
  feedbackInfoSchema,
} from "./schema";
import { TauriBackend } from "./tauri-backend";

describe("feedback", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("the feedback commands are queries, not UiCommands", () => {
    for (const command of [
      "feedback_diagnostics",
      "feedback_submit",
      "feedback_attachment_add",
      "feedback_attachment_remove",
      "feedback_attachments_clear",
    ] as const)
      expect(QUERY_COMMANDS).toContain(command);
  });

  it("the mock stages files with the shell's checks in the shell's order, and a report takes them along", async () => {
    vi.useFakeTimers();
    const mock = new MockBackend();
    const file = (name: string, type: string, size: number) => ({
      name,
      type,
      bytes: new Uint8Array(size),
    });
    const refused = (name: string, type: string, size: number) =>
      mock.feedbackAttachmentAdd(file(name, type, size)).then(
        () => "staged",
        (e: unknown) => (e instanceof Error ? e.message : String(e)),
      );
    const shot = await mock.feedbackAttachmentAdd(file("C:\\shots\\截图.png", "image/png", 3));
    expect(shot).toEqual({ id: "attachment-1", name: "截图.png", type: "image/png", size: 3 });
    expect(await refused("  ", "text/plain", 1)).toBe("attachment_name");
    expect(await refused("a.txt", "text/plain", 1)).toBe("attachment_type");
    expect(await refused("a.png", "image/png", FEEDBACK_MAX_IMAGE_BYTES + 1)).toBe(
      "attachment_too_large",
    );
    expect(await refused("a.mp4", "video/mp4", 0)).toBe("attachment_too_large");
    const clip = await mock.feedbackAttachmentAdd(
      file("clip.mp4", "video/mp4", FEEDBACK_MAX_VIDEO_BYTES),
    );
    expect(await refused("b.mp4", "video/mp4", FEEDBACK_MAX_VIDEO_BYTES)).toBe("attachment_total");
    await mock.feedbackAttachmentAdd(file("c.png", "image/png", 1));
    expect(await refused("d.png", "image/png", 1)).toBe("attachment_too_many");
    await mock.feedbackAttachmentRemove("attachment-3");
    await mock.feedbackAttachmentRemove("never-staged");
    expect(mock.feedbackStaged.map((a) => a.id)).toEqual([shot.id, clip.id]);
    const draft = { kind: "bug" as const, message: "看图", contact: null, locale: "zh-CN" };
    await expect(mock.feedbackSubmit({ ...draft, attachments: ["nope"] })).rejects.toThrow(
      "invalid",
    );
    const sent = mock.feedbackSubmit({ ...draft, attachments: [shot.id] });
    vi.advanceTimersByTime(MOCK_FEEDBACK_MS);
    expect(await sent).toEqual({ id: "feedback-1" });
    expect(mock.feedbackSent).toEqual([{ ...draft, attachments: [shot.id] }]);
    // The report took its file; the other stays staged until it is cleared.
    expect(mock.feedbackStaged.map((a) => a.id)).toEqual([clip.id]);
    await mock.feedbackAttachmentsClear();
    expect(mock.feedbackStaged).toEqual([]);
  });

  it("the mock's attachment failures come only with files, and an unfinished upload still sends the report", async () => {
    vi.useFakeTimers();
    const draft = { kind: "bug" as const, message: "m", contact: null, locale: "en" };
    const png = { name: "a.png", type: "image/png", bytes: new Uint8Array(1) };
    const partial = new MockBackend({ feedback: "attachments" });
    const plain = partial.feedbackSubmit(draft);
    vi.advanceTimersByTime(MOCK_FEEDBACK_MS);
    expect(await plain).toEqual({ id: "feedback-1" });
    const staged = await partial.feedbackAttachmentAdd(png);
    const withFile = partial.feedbackSubmit({ ...draft, attachments: [staged.id] });
    vi.advanceTimersByTime(MOCK_FEEDBACK_MS);
    await expect(withFile).rejects.toThrow("attachments");
    expect(partial.feedbackSent).toHaveLength(2);
    expect(partial.feedbackStaged).toEqual([]);
    const full = new MockBackend({ feedback: "storage_full" });
    const refusedFile = await full.feedbackAttachmentAdd(png);
    const refused = full.feedbackSubmit({ ...draft, attachments: [refusedFile.id] });
    vi.advanceTimersByTime(MOCK_FEEDBACK_MS);
    await expect(refused).rejects.toThrow("storage_full");
    expect(full.feedbackSent).toEqual([]);
    expect(full.feedbackStaged).toHaveLength(1);
    // The phone stages files like the desktop (user decision 2026-10-01; it refused before).
    expect(await new MockBackend({ role: "phone" }).feedbackAttachmentAdd(png)).toMatchObject({
      name: png.name,
    });
  });

  it("cleanAttachmentName is the shell's clean_name", () => {
    expect(cleanAttachmentName("/home/u/Pictures/shot.png")).toBe("shot.png");
    expect(cleanAttachmentName(' a\u0007b"c.png ')).toBe("a_b_c.png");
    expect(cleanAttachmentName("dir/")).toBeUndefined();
    const long = `${"名".repeat(200)}.webm`;
    const cleaned = cleanAttachmentName(long) ?? "";
    expect(Array.from(cleaned)).toHaveLength(120);
    expect(cleaned.endsWith(".webm")).toBe(true);
    expect(cleanAttachmentName(`${"x".repeat(200)}.averyverylongext`)).toBe("x".repeat(120));
  });

  it("TauriBackend sends a file as the raw IPC body with its name and type in headers", async () => {
    const raw: { command: string; bytes: Uint8Array; headers: Record<string, string> }[] = [];
    const calls: { command: string; args: unknown }[] = [];
    const backend = new TauriBackend({
      invoke: (command, args) => {
        calls.push({ command, args });
        return Promise.resolve(null);
      },
      invokeRaw: (command, bytes, headers) => {
        raw.push({ command, bytes, headers });
        return Promise.resolve({ id: "u-1", name: "截图 1.png", type: "image/png", size: 2 });
      },
      listen: () => Promise.resolve(() => undefined),
    });
    const bytes = new Uint8Array([7, 9]);
    expect(
      await backend.feedbackAttachmentAdd({ name: "截图 1.png", type: "image/png", bytes }),
    ).toEqual({ id: "u-1", name: "截图 1.png", type: "image/png", size: 2 });
    expect(raw).toEqual([
      {
        command: "feedback_attachment_add",
        bytes,
        headers: { "x-voltip-name": "%E6%88%AA%E5%9B%BE%201.png", "x-voltip-type": "image/png" },
      },
    ]);
    await backend.feedbackAttachmentRemove("u-1");
    await backend.feedbackAttachmentsClear();
    expect(calls).toEqual([
      { command: "feedback_attachment_remove", args: { id: "u-1" } },
      { command: "feedback_attachments_clear", args: undefined },
    ]);
    const broken = new TauriBackend({
      invokeRaw: () => Promise.resolve({ id: 1 }),
      listen: () => Promise.resolve(() => undefined),
    });
    await expect(
      broken.feedbackAttachmentAdd({ name: "a.png", type: "image/png", bytes }),
    ).rejects.toThrow(/invalid|expected/i);
  });

  it("the mock's diagnostics follow the engines: on-device names the model and the device, the clean-up only when it is on", async () => {
    const cloud = await new MockBackend().feedbackDiagnostics("zh-CN");
    expect(feedbackInfoSchema.parse(cloud)).toEqual(cloud);
    expect(cloud).toMatchObject({
      configured: true,
      diagnostics: {
        app_version: "0.0.1",
        os: "windows",
        locale: "zh-CN",
        asr_provider: "builtin",
        llm_provider: "builtin",
        output_mode: "whole_take",
      },
    });
    expect(cloud.diagnostics.local_model).toBeUndefined();
    const local = await new MockBackend({
      settings: {
        engines: {
          ...defaultEngineSettings(),
          asr_provider: "local",
          local_model: "sense-voice-small",
          refine_enabled: false,
        },
      },
    }).feedbackDiagnostics("en");
    expect(local.diagnostics).toMatchObject({
      asr_provider: "local",
      local_model: "sense-voice-small",
      compute: "auto",
    });
    expect(local.diagnostics.llm_provider).toBeUndefined();
    const linux = await new MockBackend({
      identity: { ...desktopIdentity(), platform: "linux" },
    }).feedbackDiagnostics("en");
    expect(linux.diagnostics.os).toBe("linux");
    expect(
      (await new MockBackend({ feedback: "not_configured" }).feedbackDiagnostics("en")).configured,
    ).toBe(false);
  });

  it("the mock endpoint takes a report after a moment, trims it, and refuses what the real one refuses", async () => {
    vi.useFakeTimers();
    const mock = new MockBackend();
    const sent = mock.feedbackSubmit({
      kind: "bug",
      message: "  hi ",
      contact: "  ",
      locale: "en",
    });
    vi.advanceTimersByTime(MOCK_FEEDBACK_MS);
    expect(await sent).toEqual({ id: "feedback-1" });
    expect(mock.feedbackSent).toEqual([
      { kind: "bug", message: "hi", contact: null, locale: "en" },
    ]);
    const draft = { kind: "idea" as const, contact: null, locale: "en" };
    await expect(mock.feedbackSubmit({ ...draft, message: "   " })).rejects.toThrow("invalid");
    await expect(
      mock.feedbackSubmit({ ...draft, message: "x".repeat(FEEDBACK_MESSAGE_MAX + 1) }),
    ).rejects.toThrow("invalid");
    await expect(
      mock.feedbackSubmit({
        ...draft,
        message: "x",
        contact: "c".repeat(FEEDBACK_CONTACT_MAX + 1),
      }),
    ).rejects.toThrow("invalid");
    const failing = new MockBackend({ feedback: "timeout" });
    const late = failing.feedbackSubmit({ ...draft, message: "x" });
    vi.advanceTimersByTime(MOCK_FEEDBACK_MS);
    await expect(late).rejects.toThrow("timeout");
    expect(failing.feedbackSent).toEqual([]);
  });

  it("the phone sends feedback of its own, naming the phone (user decision 2026-10-01; it refused before)", async () => {
    vi.useFakeTimers();
    const phone = new MockBackend({ role: "phone" });
    const info = await phone.feedbackDiagnostics("zh-CN");
    expect(info.configured).toBe(true);
    expect(info.diagnostics).toMatchObject({ os: "android", arch: "aarch64", locale: "zh-CN" });
    expect(info.diagnostics.session).toBeUndefined();
    const staged = await phone.feedbackAttachmentAdd({
      name: "截图.png",
      type: "image/png",
      bytes: new Uint8Array([1, 2, 3]),
    });
    const sent = phone.feedbackSubmit({
      kind: "bug",
      message: "x",
      contact: null,
      locale: "zh-CN",
      attachments: [staged.id],
    });
    vi.advanceTimersByTime(MOCK_FEEDBACK_MS);
    await expect(sent).resolves.toEqual({ id: "feedback-1" });
    expect(phone.feedbackSent).toHaveLength(1);
    vi.useRealTimers();
  });

  it("TauriBackend invokes both commands and validates what comes back", async () => {
    const calls: { command: string; args: unknown }[] = [];
    const info = {
      configured: true,
      diagnostics: {
        app_version: "0.0.1",
        os: "linux",
        arch: "x86_64",
        session: "wayland",
        locale: "en",
        asr_provider: "local",
        local_model: "qwen3-asr-0.6b",
        compute: "gpu",
        output_mode: "whole_take",
      },
    };
    const backend = new TauriBackend({
      invoke: (command, args) => {
        calls.push({ command, args });
        return Promise.resolve(command === "feedback_diagnostics" ? info : { id: "f-9" });
      },
      listen: () => Promise.resolve(() => undefined),
    });
    expect(await backend.feedbackDiagnostics("en")).toEqual(info);
    const draft = { kind: "other" as const, message: "m", contact: "c", locale: "en" };
    expect(await backend.feedbackSubmit(draft)).toEqual({ id: "f-9" });
    expect(calls).toEqual([
      { command: "feedback_diagnostics", args: { locale: "en" } },
      { command: "feedback_submit", args: draft },
    ]);
    const broken = new TauriBackend({
      invoke: () => Promise.resolve({ configured: "yes" }),
      listen: () => Promise.resolve(() => undefined),
    });
    await expect(broken.feedbackDiagnostics("en")).rejects.toThrow(/invalid|expected/i);
    await expect(broken.feedbackSubmit(draft)).rejects.toThrow(/invalid|expected/i);
  });
});
