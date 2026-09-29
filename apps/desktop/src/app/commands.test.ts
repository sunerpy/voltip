import { THEME_IDS, createTranslator, zhT } from "@voltip/shared";
import { MockBackend, phoneIdentity } from "@voltip/shared/mock";
import { buildCommands } from "./commands";
import { AI_ROUTE, SPEECH_ROUTE } from "./router";

function deps(overrides: Partial<Parameters<typeof buildCommands>[0]> = {}) {
  const backend = new MockBackend();
  return {
    state: backend.peek(),
    resolvedTheme: "light" as const,
    systemDark: false,
    navigate: vi.fn(),
    setTheme: vi.fn(),
    toast: vi.fn(),
    dictate: vi.fn(),
    copyLast: vi.fn(),
    clearHistory: vi.fn(),
    i18n: zhT,
    ...overrides,
  };
}

describe("buildCommands", () => {
  it("lists themes first with the current one marked, then actions and navigation", () => {
    const toast = vi.fn<(message: string) => void>();
    const d = deps({ toast });
    const items = buildCommands(d);
    expect(items.slice(0, 4).map((i) => i.group)).toEqual(["主题", "主题", "主题", "主题"]);
    expect(items.find((i) => i.id === "theme-light")?.hint).toBe("当前");
    expect(items.find((i) => i.id === "theme-dark")?.hint).toBeUndefined();
    expect(items.find((i) => i.id === "theme-system")?.hint).toBe("系统：明亮");
    for (const item of items) item.run();
    expect(d.dictate).toHaveBeenCalledTimes(1);
    expect(d.copyLast).toHaveBeenCalledTimes(1);
    expect(d.clearHistory).toHaveBeenCalledTimes(1);
    expect(d.setTheme).toHaveBeenCalledWith("graphite", false);
    expect(d.setTheme).toHaveBeenCalledWith("light", true);
    expect(d.navigate).toHaveBeenCalledWith({ name: "rules", compose: true });
    expect(d.navigate).toHaveBeenCalledWith({ name: "history" });
    expect(d.navigate).toHaveBeenCalledWith({ name: "devices" });
    expect(d.navigate).toHaveBeenCalledWith({ name: "settings", section: "appearance" });
    // regression (2026-09-28): 语音模型, AI 模型 and 反馈 are pages, each with its own entry.
    expect(d.navigate).toHaveBeenCalledWith(SPEECH_ROUTE);
    expect(d.navigate).toHaveBeenCalledWith(AI_ROUTE);
    expect(d.navigate).toHaveBeenCalledWith({ name: "feedback" });
    expect(items.find((i) => i.id === "nav-engines")).toMatchObject({
      label: "打开语音模型",
      icon: "wave",
    });
    expect(items.find((i) => i.id === "nav-ai")).toMatchObject({
      label: "打开 AI 模型",
      icon: "wand",
    });
    expect(items.find((i) => i.id === "nav-feedback")).toMatchObject({
      label: "打开反馈",
      icon: "chat",
    });
    expect(toast).toHaveBeenCalledTimes(THEME_IDS.length + 1);
    for (const [message] of toast.mock.calls) expect(message).toMatch(/^主题已切换/);
    expect(items.map((i) => i.group)).not.toContain("BRIDGE");
  });

  it("regression: the palette has no Bridge & MCP entries", () => {
    const items = buildCommands(deps());
    // Windows test 2026-09-25: the feature is being removed, so no command may name or control it.
    expect(items.find((i) => i.id === "bridge-pause")).toBeUndefined();
    for (const item of items) {
      expect(
        `${item.group} ${item.label} ${item.hint ?? ""} ${item.disabledHint ?? ""}`,
      ).not.toMatch(/Bridge|MCP|Hook|Claude Code|OpenCode/i);
    }
  });

  it("regression: dictation, copy-last and clear-history are real actions that follow the core's state", () => {
    const backend = new MockBackend({
      identity: phoneIdentity(),
      settings: { follow_system_theme: true, hotkey: "Ctrl+Shift+D" },
    });
    const d = deps({ state: backend.peek(), systemDark: true });
    const items = buildCommands(d);
    const dictate = items.find((i) => i.id === "dictate");
    expect(dictate).toMatchObject({ label: "开始听写", icon: "mic", keys: "Ctrl Shift D" });
    expect(dictate?.disabled).toBeFalsy();
    const copy = items.find((i) => i.id === "copy-last");
    expect(copy?.disabled).toBe(false);
    expect(copy?.hint).toMatch(/^\d+ 字$/);
    const clear = items.find((i) => i.id === "clear-history");
    expect(clear?.hint).toBe(`${backend.peek().history.length} 条`);
    for (const item of items)
      expect(`${item.hint ?? ""} ${item.disabledHint ?? ""}`).not.toMatch(/第二阶段|尚未接入/);
    expect(d.toast).not.toHaveBeenCalled();
    expect(items.find((i) => i.id === "theme-system")?.hint).toBe("系统：暗黑 · 当前");
    expect(items.find((i) => i.id === "theme-light")?.hint).toBeUndefined();
    // While listening the entry becomes 停止听写; while processing it is disabled with a reason.
    const listening = buildCommands(
      deps({
        state: {
          ...backend.peek(),
          dictation: {
            session: 1,
            phase: { phase: "listening", started_at: 0, ready: true, locked: false },
            kind: "dictation",
          },
        },
      }),
    ).find((i) => i.id === "dictate");
    expect(listening).toMatchObject({ label: "停止听写", icon: "stop" });
    const processing = buildCommands(
      deps({
        state: {
          ...backend.peek(),
          dictation: {
            session: 1,
            phase: { phase: "processing", stage: "transcribing", started_at: 0 },
            kind: "dictation",
          },
        },
      }),
    ).find((i) => i.id === "dictate");
    expect(processing?.disabled).toBe(true);
    expect(processing?.disabledHint).toBe("上一段录音仍在识别或插入，请稍候");
    // With no history both history actions are disabled with a plain reason.
    const empty = buildCommands(deps({ state: { ...backend.peek(), history: [] } }));
    for (const id of ["copy-last", "clear-history"]) {
      const item = empty.find((i) => i.id === id);
      expect(item?.disabled).toBe(true);
      expect(item?.disabledHint).toBe("历史记录为空");
    }
    expect(empty.find((i) => i.id === "copy-last")?.hint).toBe("历史为空");
  });
});

describe("buildCommands in English", () => {
  it("regression: every palette entry, hint and toast follows the translator", () => {
    const toast = vi.fn<(message: string) => void>();
    const d = deps({ toast, i18n: createTranslator("en"), systemDark: true });
    const items = buildCommands(d);
    expect(items.slice(0, 4).map((i) => i.group)).toEqual(["Theme", "Theme", "Theme", "Theme"]);
    expect(items.find((i) => i.id === "theme-light")?.hint).toBe("current");
    expect(items.find((i) => i.id === "theme-system")?.hint).toBe("System: Dark");
    expect(items.find((i) => i.id === "dictate")?.label).toBe("Start dictating");
    expect(items.find((i) => i.id === "clear-history")?.hint).toMatch(/^\d+ entries$/);
    for (const item of items) item.run();
    for (const [message] of toast.mock.calls) expect(message).toMatch(/^Theme switched/);
    const text = items.flatMap((i) => [i.group, i.label, i.hint ?? "", i.disabledHint ?? ""]);
    expect(text.filter((s) => /[一-鿿]/.test(s))).toEqual([]);
  });
});
