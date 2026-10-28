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
    expect(bubbleStrings(createTranslator("en").t).close).toBe("Close");
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
      }),
    );
    expect(status).toEqual({
      enabled: true,
      running: false,
      permissions: { microphone: true, overlay: false, notifications: true },
      mode: "tap",
      showResult: false,
      showDiscard: true,
    });
    expect(grantedCount(status.permissions)).toBe(2);
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
