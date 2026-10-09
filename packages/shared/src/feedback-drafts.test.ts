import {
  attachmentError,
  attachmentSize,
  attachmentType,
  diagnosticValue,
  feedbackError,
  FEEDBACK_LIMITS,
  precheckAttachment,
} from "./feedback-drafts";
import { zhT } from "./i18n";
import { FEEDBACK_MAX_VIDEO_BYTES } from "./schema";

// The 反馈 page's helpers, the desktop's dialog and the phone's page (docs/feedback.md).
describe("the feedback helpers", () => {
  it("types a file without one by its extension and checks the limits before reading it", () => {
    expect(attachmentType({ name: "a.PNG", type: "" })).toBe("image/png");
    expect(attachmentType({ name: "clip.mov", type: "" })).toBe("video/quicktime");
    expect(attachmentType({ name: "noext", type: "" })).toBe("");
    expect(attachmentType({ name: "a.png", type: "image/x-icon" })).toBe("image/x-icon");
    const staged = [{ id: "1", name: "a.mp4", type: "video/mp4", size: FEEDBACK_MAX_VIDEO_BYTES }];
    expect(precheckAttachment("image/png", 1, [])).toBeUndefined();
    expect(precheckAttachment("image/heic", 1, [])).toBe("attachment_type");
    expect(precheckAttachment("video/webm", FEEDBACK_MAX_VIDEO_BYTES, staged)).toBe(
      "attachment_total",
    );
    expect(precheckAttachment("image/png", 1, [...staged, ...staged, ...staged])).toBe(
      "attachment_too_many",
    );
    expect(attachmentError(new Error("attachment_total"))).toBe("attachment_total");
    expect(attachmentError(new Error("feedback: 请在电脑上反馈"))).toBe("attachment_type");
    expect(feedbackError(new Error("storage_full"))).toBe("storage_full");
  });

  it("words the diagnostics and maps the shell's refusals", () => {
    const { t } = zhT;
    expect(diagnosticValue("os", "linux", t, "zh-CN")).toBe("Linux");
    expect(diagnosticValue("os", "android", t, "zh-CN")).toBe("Android");
    expect(diagnosticValue("os", "haiku", t, "zh-CN")).toBe("haiku");
    expect(diagnosticValue("asr_provider", "local", t, "zh-CN")).toBe("本机");
    expect(diagnosticValue("llm_provider", "someone", t, "zh-CN")).toBe("someone");
    expect(diagnosticValue("compute", "gpu", t, "zh-CN")).toBe("GPU");
    expect(diagnosticValue("compute", "npu", t, "zh-CN")).toBe("npu");
    expect(diagnosticValue("output_mode", "live_inject", t, "zh-CN")).toBe("边说边输入");
    expect(diagnosticValue("output_mode", "odd", t, "zh-CN")).toBe("odd");
    expect(diagnosticValue("arch", "aarch64", t, "zh-CN")).toBe("aarch64");
    expect(feedbackError(new Error("timeout"))).toBe("timeout");
    expect(feedbackError("rate_limited")).toBe("rate_limited");
    expect(feedbackError(new Error("feedback: 请在电脑上反馈"))).toBe("server");
  });

  it("words the limits and an attachment's size", () => {
    expect(FEEDBACK_LIMITS).toEqual({ count: 3, image: "5 MB", video: "20 MB", total: "25 MB" });
    expect(attachmentSize(512)).toBe("1 KB");
    expect(attachmentSize(3 * 1_048_576 + 100_000)).toBe("3.1 MB");
  });
});
