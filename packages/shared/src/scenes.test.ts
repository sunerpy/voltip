import {
  LINUX_TERMINALS,
  SceneError,
  WINDOWS_TERMINALS,
  checkScenes,
  cleanContextLine,
  isAutoLanguage,
  isTerminalApp,
  terminalIds,
  matchScene,
  normalizeAppId,
  recentApps,
  sanitizeForegroundApp,
  validateSceneDraft,
} from "./scenes";
import type { HistoryEntry, Scene, SceneDraft } from "./schema";

function draft(name: string, apps: string[], keywords: string[] = []): SceneDraft {
  return { name, enabled: true, match: { apps, title_contains: keywords }, overrides: {} };
}

function scene(name: string, apps: string[], keywords: string[] = [], enabled = true): Scene {
  const d = validateSceneDraft(draft(name, apps, keywords));
  return { id: `id-${name}`, ...d, enabled, created_at_ms: 1, updated_at_ms: 1 };
}

function refusal(d: SceneDraft): string {
  let error: unknown;
  try {
    validateSceneDraft(d);
  } catch (e) {
    error = e;
  }
  expect(error).toBeInstanceOf(SceneError);
  return error instanceof Error ? error.message : String(error);
}

describe("scenes as the core runs them (docs/dictation.md section 18)", () => {
  it("normalises app ids like voltip_core::scenes::normalize_app_id", () => {
    for (const [raw, want] of [
      ["  Slack.EXE ", "slack"],
      ["notepad.exe.exe", "notepad"],
      ["com.Apple.Safari", "com.apple.safari"],
      [".exe", ""],
      ["微信.exe", "微信"],
    ] as const) {
      expect(normalizeAppId(raw)).toBe(want);
      expect(normalizeAppId(normalizeAppId(raw))).toBe(want);
    }
  });

  it("normalises a draft: trimmed name, normalised and de-duplicated apps, keywords without case duplicates, lower-case language, clean prompt", () => {
    const d = validateSceneDraft({
      name: " 聊天 ",
      enabled: false,
      match: {
        apps: [" Slack.EXE", "slack", "  ", "WeChat.exe"],
        title_contains: [" GitHub ", "github", ""],
      },
      overrides: {
        refine_enabled: null,
        refine_style: "punctuation",
        language: " ZH-Hans ",
        prompt: "  第一行\r\n第二行\r第三行\t。 \n",
      },
    });
    expect(d).toEqual({
      name: "聊天",
      enabled: false,
      match: { apps: ["slack", "wechat"], title_contains: ["GitHub"] },
      overrides: {
        refine_style: "punctuation",
        language: "zh-hans",
        prompt: "第一行\n第二行\n第三行\t。",
      },
    });
    expect(validateSceneDraft(d)).toEqual(d);
    const blank = validateSceneDraft({
      ...draft("a", ["x"]),
      overrides: { language: " ", prompt: " \r\n" },
    });
    expect(blank.overrides).toEqual({});
    const auto = validateSceneDraft({ ...draft("a", ["x"]), overrides: { language: "AUTO" } });
    expect(isAutoLanguage(auto.overrides.language)).toBe(true);
    expect(isAutoLanguage("zh")).toBe(false);
  });

  it("refuses drafts past the limits with the core's words", () => {
    expect(refusal(draft("  ", ["x"]))).toBe("scenes: 场景名称不能为空");
    expect(refusal(draft("名".repeat(33), ["x"]))).toContain("最多 32 个字符（当前 33）");
    expect(refusal(draft("a\nb", ["x"]))).toContain("不能包含换行");
    expect(refusal(draft("聊天", []))).toBe("scenes: 场景「聊天」至少要有一个应用");
    expect(refusal(draft("a", [".EXE"]))).toContain("应用 id「.EXE」无效");
    expect(refusal(draft("a", ["x".repeat(129)]))).toContain("应用 id最多 128 个字符");
    const many = Array.from({ length: 21 }, (_, i) => `app${i}`);
    expect(refusal(draft("a", many))).toContain("最多 20 个应用（当前 21）");
    const keywords = Array.from({ length: 11 }, (_, i) => `k${i}`);
    expect(refusal(draft("a", ["x"], keywords))).toContain("最多 10 个窗口标题关键词（当前 11）");
    for (const language of ["中文", "zh_CN", "-x", "a".repeat(17)])
      expect(refusal({ ...draft("a", ["x"]), overrides: { language } })).toContain("语言代码");
    expect(refusal({ ...draft("a", ["x"]), overrides: { prompt: "长".repeat(501) } })).toContain(
      "补充要求最多 500 个字符（当前 501）",
    );
    expect(refusal({ ...draft("a", ["x"]), overrides: { prompt: "a\u001b[31m" } })).toBe(
      "scenes: 补充要求不能包含控制字符",
    );
  });

  it("keeps the list at 50 scenes with unique names", () => {
    const list = Array.from({ length: 50 }, (_, i) => scene(`s${i}`, ["x"]));
    expect(() => {
      checkScenes(list);
    }).not.toThrow();
    expect(() => {
      checkScenes([...list, scene("one more", ["y"])]);
    }).toThrow("场景最多 50 个");
    expect(() => {
      checkScenes([scene("Chat", ["a"]), scene("chat", ["b"])]);
    }).toThrow("scenes: 已有名为「Chat」的场景");
  });

  it("matches the first enabled scene by app and title, like section 18.3", () => {
    const scenes = [
      scene("GitHub", ["chrome"], ["GitHub"]),
      scene("浏览器", ["Chrome.exe"]),
      scene("停用", ["slack"], [], false),
      scene("聊天", ["slack"]),
    ];
    const hit = (id: string, title?: string) =>
      matchScene(scenes, { id, name: id, ...(title === undefined ? {} : { title }) })?.name;
    expect(hit("chrome", "PR · GITHUB")).toBe("GitHub");
    expect(hit("CHROME.EXE", "Inbox")).toBe("浏览器");
    expect(hit("chrome")).toBe("浏览器");
    expect(hit("slack")).toBe("聊天");
    expect(hit("code", "GitHub")).toBeUndefined();
    expect(hit("")).toBeUndefined();
  });

  it("keeps the probe's answer as the core does: a normalised id, one-line names cut with …, no id no answer", () => {
    expect(cleanContextLine("  a\tb\n\u0007c  ", 64)).toBe("a b c");
    expect(cleanContextLine(" \n ", 64)).toBeUndefined();
    expect(cleanContextLine("长".repeat(10), 5)).toBe("长长长长…");
    expect(cleanContextLine("ab cd", 4)).toBe("ab…");
    expect(cleanContextLine("abcd", 4)).toBe("abcd");
    expect(
      sanitizeForegroundApp({ id: " Slack.EXE", name: " Slack\n 4 ", title: " #dev\r\n " }),
    ).toEqual({
      id: "slack",
      name: "Slack 4",
      title: "#dev",
    });
    expect(sanitizeForegroundApp({ id: "code", name: "  " })).toEqual({ id: "code", name: "code" });
    expect(sanitizeForegroundApp({ id: "code", name: "Code", title: " " })).toEqual({
      id: "code",
      name: "Code",
    });
    expect(sanitizeForegroundApp({ id: "x", name: "n".repeat(70) })?.name).toHaveLength(64);
    expect(sanitizeForegroundApp({ id: ".exe", name: "Nothing" })).toBeUndefined();
  });

  it("lists the recent apps newest first, one per id, capped", () => {
    const row = (app?: { id: string; name: string }): HistoryEntry => ({
      id: "h",
      at_ms: 1,
      raw_text: "a",
      text: "a",
      refined: false,
      asr_model: "m",
      duration_ms: 1,
      asr_ms: 1,
      outcome: { kind: "inserted", via: "paste" },
      starred: false,
      mode: "whole_take",
      kind: "dictation",
      ...(app === undefined ? {} : { app }),
    });
    const history = [
      row({ id: "slack", name: "Slack 4" }),
      row(),
      row({ id: "code", name: "Code" }),
      row({ id: "slack", name: "Slack 3" }),
    ];
    expect(recentApps(history)).toEqual([
      { id: "slack", name: "Slack 4" },
      { id: "code", name: "Code" },
    ]);
    expect(recentApps(history, 1)).toHaveLength(1);
  });
});

describe("the voice edit terminal guard (section 19)", () => {
  it("regression: the terminal table is per OS and macOS is exempt like voltip_platform::foreground::is_terminal", () => {
    for (const id of [
      "WindowsTerminal.exe",
      "cmd.exe",
      "conhost",
      "powershell",
      "pwsh",
      "wezterm-gui",
    ])
      expect(isTerminalApp("windows", id)).toBe(true);
    for (const id of [
      " Gnome-Terminal-Server ",
      "konsole",
      "xterm",
      "org.wezfurlong.wezterm",
      "kgx",
    ])
      expect(isTerminalApp("linux", id)).toBe(true);
    for (const id of WINDOWS_TERMINALS) expect(isTerminalApp("windows", id)).toBe(true);
    for (const id of LINUX_TERMINALS) expect(isTerminalApp("linux", id)).toBe(true);
    // Editors and chat apps are not terminals; the tables are per OS.
    for (const id of ["code", "slack", "chrome", "winword", ""]) {
      expect(isTerminalApp("windows", id)).toBe(false);
      expect(isTerminalApp("linux", id)).toBe(false);
    }
    expect(isTerminalApp("windows", "gnome-terminal-server")).toBe(false);
    expect(isTerminalApp("linux", "windowsterminal")).toBe(false);
    // macOS copies with Cmd+C: no guard; other hosts send no copy chord.
    expect(terminalIds("macos")).toEqual([]);
    expect(terminalIds("other")).toEqual([]);
    expect(isTerminalApp("macos", "com.apple.terminal")).toBe(false);
    expect(isTerminalApp("macos", "windowsterminal")).toBe(false);
    expect(
      [...WINDOWS_TERMINALS, ...LINUX_TERMINALS].every((id) => normalizeAppId(id) === id),
    ).toBe(true);
    expect(WINDOWS_TERMINALS).toHaveLength(11);
    expect(LINUX_TERMINALS).toHaveLength(33);
  });
});
