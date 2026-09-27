import { type UiState, createTranslator, emptyEngineStatus } from "@voltip/shared";
import { MOCK_HOTKEY_BACKEND, MockBackend, sampleDevices } from "@voltip/shared/mock";
import { type Route, SETTINGS_SECTIONS } from "../app/router";
import {
  type PageMetaExtras,
  engineReadout,
  hotkeyBackendReadout,
  hotkeyShortcut,
  pageMeta,
  settingsReadouts,
  shortModel,
} from "./page-meta";

const extras: PageMetaExtras = { microphone: "Fifine K669" };
const EN = createTranslator("en");

function stateWith(overrides: Partial<UiState> = {}): UiState {
  return { ...new MockBackend({ devices: sampleDevices(1_758_700_000) }).peek(), ...overrides };
}

describe("pageMeta", () => {
  it("regression: a settings route describes the page beneath the dialog, not the dialog", () => {
    const state = stateWith();
    for (const section of SETTINGS_SECTIONS) {
      const route: Route = { name: "settings", section };
      // Without a background the router's default (home) applies.
      expect(pageMeta(route, state, extras)).toEqual(pageMeta({ name: "home" }, state, extras));
      for (const background of [
        { name: "history", filter: "today" },
        { name: "dictionary" },
        { name: "rules" },
        { name: "devices" },
      ] as const) {
        const meta = pageMeta(route, state, extras, background);
        expect(meta).toEqual(pageMeta(background, state, extras));
        expect(meta.title).not.toMatch(/设置|模型/);
      }
    }
    expect(
      pageMeta({ name: "settings", section: "speech" }, state, extras, { name: "devices" }).title,
    ).toBe("手机");
  });

  it("regression: the hotkey line and the engine readouts come from state, never from a fixture", () => {
    const state = stateWith({ settings: { ...stateWith().settings, hotkey: "Ctrl+Shift+D" } });
    const home = pageMeta({ name: "home" }, state, extras);
    expect(home.shortcuts[0]).toEqual(["Ctrl Shift D", "按住听写"]);
    expect(home.shortcuts.flat()).not.toContain("Ctrl Alt Space");
    expect(home.readouts[0]).toEqual({
      label: "语音模型",
      value: "Qwen3-ASR-1.7B",
      lamp: "ok",
      title: "内置服务 · Qwen/Qwen3-ASR-1.7B · 就绪",
    });
    expect(home.readouts[1]).toEqual({ label: "麦克风", value: "Fifine K669" });
    expect(pageMeta({ name: "overlay" }, state, extras).shortcuts[0]).toEqual([
      "Ctrl Shift D",
      "听写",
    ]);
    // Before the core reported its engines the readout says so instead of inventing one.
    expect(engineReadout(emptyEngineStatus())).toEqual({
      label: "语音模型",
      value: "等待核心…",
      lamp: "idle",
    });
    expect(
      engineReadout({ ...state.engines, asr_ready: false, asr_issue: "key_missing" }),
    ).toMatchObject({ lamp: "danger", title: "内置服务 · Qwen/Qwen3-ASR-1.7B · 缺少密钥" });
    // Regression (public release, 2026-09-27): no readout names the built-in service's host.
    expect(JSON.stringify(engineReadout(state.engines))).not.toMatch(/https?:|\.example/);
    expect(hotkeyShortcut("Ctrl+Alt+Space")).toEqual(["Ctrl Alt Space", "按住听写"]);
    // docs/dictation.md §13: the footer caption follows the activation mode.
    expect(hotkeyShortcut("Ctrl+Alt+Space", "toggle")).toEqual(["Ctrl Alt Space", "按一下听写"]);
    expect(hotkeyShortcut("Ctrl+Alt+Space", "hold_or_toggle")).toEqual([
      "Ctrl Alt Space",
      "按住或按一下听写",
    ]);
    expect(shortModel("Qwen/Qwen3-ASR-1.7B")).toBe("Qwen3-ASR-1.7B");
    expect(shortModel("whisper")).toBe("whisper");
    expect(shortModel("vendor/")).toBe("vendor/");
    // History reads the core's list.
    const history = pageMeta({ name: "history" }, state, extras);
    expect(history.readouts[0]).toEqual({
      label: "历史记录",
      value: `${state.history.length} / 500 条`,
      lamp: "ok",
    });
    expect(history.readouts[1]?.value).toBe("history.json");
    expect(
      pageMeta({ name: "history" }, stateWith({ history: [] }), extras).readouts[0]?.lamp,
    ).toBe("idle");
    for (const route of [{ name: "dictionary" }, { name: "rules" }] as const)
      expect(pageMeta(route, state, extras).readouts.map((r) => r.value)).not.toContain(
        "精确 · SenseVoice",
      );
    for (const route of [
      { name: "home" },
      { name: "history" },
      { name: "dictionary" },
      { name: "rules" },
    ] as const)
      expect(JSON.stringify(pageMeta(route, state, extras))).not.toMatch(/第二阶段|示例|sqlite/);
  });

  it("regression: on-device the engine readout names the model with a 本机 tag and a lamp that follows asr_ready", () => {
    const state = stateWith();
    // The core reports the Chinese tier name; the English readout takes the dictionary's name by id.
    const local = {
      ...state.engines,
      asr_provider: "local" as const,
      local_model: "sense-voice-small",
      asr_host: "",
      asr_model: "轻量",
    };
    const ready = { ...local, local_ready: true, asr_ready: true };
    const missing = {
      ...local,
      local_ready: false,
      asr_ready: false,
      asr_issue: "model_not_installed" as const,
    };
    expect(engineReadout(ready)).toEqual({
      label: "语音模型",
      value: "轻量",
      lamp: "ok",
      badge: "本机",
      title: "本机 · 轻量 · 就绪",
    });
    expect(engineReadout(missing)).toMatchObject({
      lamp: "danger",
      badge: "本机",
      title: "本机 · 轻量 · 模型未下载",
    });
    expect(engineReadout(ready, EN)).toEqual({
      label: "Speech model",
      value: "Light",
      lamp: "ok",
      badge: "This device",
      title: "This device · Light · Ready",
    });
    // An id the dictionary does not know keeps the core's name in both locales.
    expect(
      engineReadout({ ...ready, local_model: "future-model", asr_model: "未来" }, EN).value,
    ).toBe("未来");
    // Cloud readouts carry no badge.
    expect(engineReadout(state.engines).badge).toBeUndefined();
  });

  it("titles every page and reads live counts from state and extras", () => {
    const state = stateWith();
    expect(pageMeta({ name: "home" }, state, extras).title).toBe("首页");
    expect(pageMeta({ name: "history" }, state, extras).title).toBe("历史记录");
    const dictionary = pageMeta({ name: "dictionary" }, state, extras);
    expect(dictionary.title).toBe("词典");
    expect(dictionary.readouts.map((r) => r.value)).toEqual([
      "Qwen3-ASR-1.7B",
      "0 / 0 启用",
      "dictionary.json",
    ]);
    expect(dictionary.readouts[1]?.lamp).toBe("idle");
    expect(pageMeta({ name: "rules" }, state, extras).title).toBe("规则");
    const devices = pageMeta({ name: "devices" }, state, extras);
    expect(devices.title).toBe("手机");
    const online = state.devices.filter((d) => d.connection.state === "online").length;
    expect(online).toBeGreaterThan(0);
    expect(devices.readouts[0]?.value).toBe(`${state.devices.length} 已配对 · ${online} 在线`);
    expect(devices.readouts[0]?.lamp).toBe("ok");
    const offline = stateWith({
      devices: state.devices.map((d) => ({
        ...d,
        connection: { ...d.connection, state: "offline" as const },
      })),
    });
    const offlineMeta = pageMeta({ name: "devices" }, offline, extras);
    expect(offlineMeta.readouts[0]?.value).toBe(`${state.devices.length} 已配对 · 0 在线`);
    expect(offlineMeta.readouts[0]?.lamp).toBe("idle");
    const onboarding = pageMeta({ name: "onboarding", step: 2 }, state, extras);
    expect(onboarding.title).toBe("首次设置 / 第 2 步");
    expect(onboarding.readouts[0]?.value).toBe("2 / 4 · 热键");
    expect(onboarding.readouts[1]?.value).toBe("Windows");
    expect(
      pageMeta({ name: "onboarding", step: 1 }, stateWith({ identity: null }), extras).readouts[1]
        ?.value,
    ).toBe("未知");
    expect(pageMeta({ name: "overlay" }, state, extras).title).toBe("悬浮胶囊");
    expect(pageMeta({ name: "notfound", path: "/x" }, state, extras)).toEqual({
      title: "未找到",
      readouts: [],
      shortcuts: [],
    });
  });
});

describe("pageMeta in English", () => {
  it("regression: titles, readouts and shortcuts follow the translator without CJK", () => {
    const state = stateWith();
    const routes: Route[] = [
      { name: "home" },
      { name: "history" },
      { name: "dictionary" },
      { name: "rules" },
      { name: "devices" },
      { name: "onboarding", step: 3 },
      { name: "overlay" },
      { name: "notfound", path: "/x" },
      { name: "settings", section: "about" },
      { name: "settings", section: "speech" },
      { name: "settings", section: "ai" },
    ];
    for (const route of routes) {
      const meta = pageMeta(route, state, extras, { name: "devices" }, EN);
      expect(JSON.stringify(meta)).not.toMatch(/[一-鿿]/);
    }
    expect(
      JSON.stringify(
        settingsReadouts(
          "speech",
          state,
          { resolvedTheme: "warm", density: "compact", fontSizePx: 15 },
          EN,
        ),
      ),
    ).not.toMatch(/[一-鿿]/);
    const home = pageMeta({ name: "home" }, state, extras, undefined, EN);
    expect(home.title).toBe("Home");
    expect(home.shortcuts[0]).toEqual(["Ctrl Alt Space", "Hold to dictate"]);
    expect(home.readouts[0]).toMatchObject({
      label: "Speech model",
      title: "Built-in service · Qwen/Qwen3-ASR-1.7B · Ready",
    });
    expect(engineReadout(emptyEngineStatus(), EN).value).toBe("Waiting for core…");
    expect(
      pageMeta({ name: "onboarding", step: 2 }, state, extras, undefined, EN).readouts[0]?.value,
    ).toBe("2 / 4 · Hotkey");
    expect(
      pageMeta(
        { name: "onboarding", step: 9 },
        stateWith({ identity: null }),
        extras,
        undefined,
        EN,
      ).readouts.map((r) => r.value),
    ).toEqual(["9 / 4 · ", "Unknown"]);
    expect(pageMeta({ name: "history" }, state, extras, undefined, EN).readouts[0]?.value).toBe(
      `${state.history.length} / 500 entries`,
    );
    expect(
      settingsReadouts(
        "appearance",
        state,
        { resolvedTheme: "warm", density: "compact", fontSizePx: 15 },
        EN,
      ),
    ).toEqual([
      { label: "Theme", value: "Warm · Not following system" },
      { label: "Density", value: "Compact · 15 px" },
    ]);
    expect(hotkeyBackendReadout({ pressed: false, capturing: false, backend: "" }, EN)).toBe(
      "Not reported yet",
    );
  });
});

describe("hotkeyBackendReadout", () => {
  it("keeps the platform tail of the shell's report and names failures", () => {
    expect(
      hotkeyBackendReadout({
        pressed: false,
        capturing: false,
        backend: "global-shortcut · Windows · RegisterHotKey",
        registered: "Ctrl+Alt+Space",
      }),
    ).toBe("Windows · RegisterHotKey");
    expect(hotkeyBackendReadout({ pressed: false, capturing: false, backend: "mock" })).toBe(
      "mock",
    );
    expect(hotkeyBackendReadout({ pressed: false, capturing: false, backend: "" })).toBe(
      "尚未报告",
    );
    expect(
      hotkeyBackendReadout({
        pressed: false,
        capturing: false,
        backend: "global-shortcut · Linux · X11",
        error: "已被占用",
      }),
    ).toBe("注册失败");
  });
});

describe("settingsReadouts", () => {
  const appearance = { resolvedTheme: "warm" as const, density: "compact", fontSizePx: 15 };

  it("gives the dialog header the hotkey, appearance and engine readouts and nothing for the brief groups", () => {
    const state = stateWith();
    // 语音模型: the ASR readout (with its token lamp) and the injection; AI 模型: the polish state.
    expect(settingsReadouts("speech", state, appearance)).toEqual([
      {
        label: "语音模型",
        value: "Qwen3-ASR-1.7B",
        lamp: "ok",
        title: "内置服务 · Qwen/Qwen3-ASR-1.7B · 就绪",
      },
      { label: "注入", value: "粘贴" },
    ]);
    expect(settingsReadouts("ai", state, appearance)).toEqual([
      { label: "润色", value: "开 · qwen3.8-27b", lamp: "ok" },
    ]);
    const off = stateWith({
      engines: { ...state.engines, refine_enabled: false, inject: "clipboard_only" },
    });
    expect(settingsReadouts("speech", off, appearance).map((r) => r.value)).toEqual([
      "Qwen3-ASR-1.7B",
      "仅剪贴板",
    ]);
    expect(settingsReadouts("ai", off, appearance).map((r) => r.value)).toEqual(["关"]);
    expect(settingsReadouts("hotkey", state, appearance)).toEqual([
      { label: "热键", value: "Ctrl Alt Space" },
      { label: "后端", value: MOCK_HOTKEY_BACKEND.split(" · ").slice(1).join(" · ") },
    ]);
    expect(settingsReadouts("appearance", state, appearance)).toEqual([
      { label: "主题", value: "暖纸 · 不跟随系统" },
      { label: "密度", value: "紧凑 · 15 px" },
    ]);
    const following = stateWith({
      settings: { ...state.settings, follow_system_theme: true },
    });
    expect(
      settingsReadouts("appearance", following, { ...appearance, density: "default" })[0]?.value,
    ).toBe("暖纸 · 跟随系统");
    expect(
      settingsReadouts("appearance", following, { ...appearance, density: "default" })[1]?.value,
    ).toBe("默认 · 15 px");
    for (const section of ["general", "privacy", "about"] as const)
      expect(settingsReadouts(section, state, appearance)).toEqual([]);
  });
});

describe("pageMeta for the dictionary and rules pages", () => {
  it("regression: the dictionary and rules readouts count the core lists and every listed shortcut is one the page handles", () => {
    const base = stateWith();
    const entry = {
      id: "e1",
      term: "Voltip",
      heard_as: [],
      source: { kind: "manual" as const },
      created_at_ms: 1,
      updated_at_ms: 1,
    };
    const rule = {
      id: "r1",
      name: "r",
      kind: "literal" as const,
      pattern: "a",
      replacement: "b",
      case_sensitive: true,
      created_at_ms: 1,
      updated_at_ms: 1,
    };
    const state = stateWith({
      dictionary: [
        { ...entry, enabled: true },
        { ...entry, id: "e2", enabled: false },
      ],
      rules: [
        { ...rule, enabled: true },
        { ...rule, id: "r2", enabled: true },
        { ...rule, id: "r3", enabled: false },
      ],
    });
    const dictionary = pageMeta({ name: "dictionary" }, state, extras);
    expect(dictionary.readouts[1]).toEqual({ label: "词典", value: "1 / 2 启用", lamp: "ok" });
    expect(dictionary.shortcuts.map(([keys]) => keys)).toEqual(["Ctrl N", "Enter", "Esc"]);
    const rules = pageMeta({ name: "rules" }, state, extras);
    expect(rules.readouts.slice(0, 2)).toEqual([
      { label: "规则", value: "2 / 3 启用", lamp: "ok" },
      { label: "存储", value: "rules.json", mono: true },
    ]);
    expect(rules.shortcuts.map(([keys]) => keys)).toEqual(["Ctrl N", "Ctrl ↵", "Ctrl S", "Esc"]);
    expect(pageMeta({ name: "rules" }, base, extras).readouts[0]?.lamp).toBe("idle");
    // Every key a footer lists has a handler behind it (History / Devices / Onboarding tests); the
    // manual-address hint had none and is gone.
    expect(pageMeta({ name: "devices" }, base, extras).shortcuts.map(([keys]) => keys)).toEqual([
      "Ctrl R",
      "Ctrl ,",
    ]);
    expect(pageMeta({ name: "history" }, base, extras).shortcuts.map(([keys]) => keys)).toEqual([
      "Ctrl F",
      "Ctrl C",
      "Del",
    ]);
    expect(
      pageMeta(
        { name: "history" },
        { ...base, settings: { ...base.settings, history: { enabled: true, keep: 50 } } },
        extras,
      ).readouts[0]?.value,
    ).toBe(`${base.history.length} / 50 条`);
    expect(pageMeta({ name: "rules" }, state, extras, undefined, EN).readouts[0]).toEqual({
      label: "Rules",
      value: "2 / 3 enabled",
      lamp: "ok",
    });
  });
});
