// The floating bubble's bridge (docs/dictation.md §20.12): the texts the app hands its native
// side, and the state that side answers with.
import { createTranslator } from "@voltip/shared";

import { BUBBLE_STRING_KEYS, bubbleStrings, grantedCount, parseBubbleStatus } from "./bubble";

describe("the bubble's bridge", () => {
  it("hands over every text the bubble shows, in the app's language, {message} left to fill in", () => {
    const zh = bubbleStrings(createTranslator("zh-CN").t);
    expect(new Set(Object.keys(zh))).toEqual(new Set(BUBBLE_STRING_KEYS));
    for (const key of BUBBLE_STRING_KEYS) expect(zh[key]).not.toBe("");
    expect(zh.failed).toBe("未完成：{message}");
    expect(zh.copied).toBe("已复制 · 长按输入框粘贴");
    // The handle of a bubble tucked into the edge, and the screen reader's way to tuck it there.
    expect([zh.expand, zh.stow]).toEqual(["展开悬浮球", "收起到屏幕边缘"]);
    // 手势粘贴: the card while the finger can swipe to paste, in the paste position, and after.
    expect(zh.recordingHoldPaste).toBe("松开复制 · 内滑粘贴 · 上下滑开取消");
    expect(zh.releasePaste).toBe("松开后粘贴到输入框");
    expect([zh.pasted, zh.pasteFallback]).toEqual(["已发出粘贴请求", "未能粘贴 · 已复制到剪贴板"]);
    const en = bubbleStrings(createTranslator("en").t);
    expect(en.close).toBe("Close");
    expect(en.pasted).toBe("Paste requested");
  });

  it("reads the native side's state, and refuses one it cannot read", () => {
    const status = parseBubbleStatus(
      JSON.stringify({
        enabled: true,
        running: false,
        permissions: { microphone: true, overlay: false, notifications: true },
        mode: "tap",
        showResult: false,
        showDiscard: true,
        pasteEnabled: true,
        accessibilityEnabled: false,
        autoStowSeconds: 5,
      }),
    );
    expect(status).toEqual({
      enabled: true,
      running: false,
      permissions: { microphone: true, overlay: false, notifications: true },
      mode: "tap",
      showResult: false,
      showDiscard: true,
      pasteEnabled: true,
      accessibilityEnabled: false,
      autoStowSeconds: 5,
    });
    expect(grantedCount(status.permissions)).toBe(2);
    // 空闲时自动收起: a value the settings page does not offer reads as off, as the native side
    // reads it.
    expect(
      parseBubbleStatus(JSON.stringify({ ...status, autoStowSeconds: 7 })).autoStowSeconds,
    ).toBe(0);
    expect(
      parseBubbleStatus(JSON.stringify({ ...status, autoStowSeconds: 30 })).autoStowSeconds,
    ).toBe(30);
    expect(() => parseBubbleStatus("[]")).toThrow("not an object");
    expect(() =>
      parseBubbleStatus(
        JSON.stringify({ enabled: "yes", running: false, permissions: {}, showResult: true }),
      ),
    ).toThrow("enabled is not a boolean");
    expect(() =>
      parseBubbleStatus(
        JSON.stringify({
          enabled: true,
          running: true,
          permissions: { microphone: true, overlay: true, notifications: true },
          mode: "hold",
          showResult: true,
        }),
      ),
    ).toThrow("showDiscard is not a boolean");
  });
});
