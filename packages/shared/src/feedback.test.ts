// In-app feedback (docs/feedback.md): the wire schemas mirror the shell's src/feedback.rs, both
// backends answer the two queries, and the mock plays the endpoint.
import {
  FEEDBACK_UNAVAILABLE,
  MOCK_FEEDBACK_MS,
  MockBackend,
  desktopIdentity,
} from "./mock-backend";
import {
  FEEDBACK_CONTACT_MAX,
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

  it("both feedback commands are queries, not UiCommands", () => {
    expect(QUERY_COMMANDS).toContain("feedback_diagnostics");
    expect(QUERY_COMMANDS).toContain("feedback_submit");
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

  it("the phone sends no feedback", async () => {
    const phone = new MockBackend({ role: "phone" });
    await expect(phone.feedbackDiagnostics("zh-CN")).rejects.toThrow(FEEDBACK_UNAVAILABLE);
    await expect(
      phone.feedbackSubmit({ kind: "bug", message: "x", contact: null, locale: "zh-CN" }),
    ).rejects.toThrow(FEEDBACK_UNAVAILABLE);
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
