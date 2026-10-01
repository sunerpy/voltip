import {
  type AudioDevice,
  type EngineSettings,
  type ModelInstallState,
  type UiState,
  createTranslator,
  defaultEngineSettings,
} from "@voltip/shared";
import { MockBackend } from "@voltip/shared/mock";
import type { MenuSection } from "@voltip/ui";
import {
  choiceId,
  engineSettingsFor,
  microphoneMenuSections,
  parseChoice,
  polishMenuSections,
  speechMenuSections,
} from "./switchers";

const INSTALLED: ModelInstallState = { kind: "installed", path: "/models/x", installed_at: 1 };

/** The state the preview reports for `engines`: the built-in service in both roles (the mock's
 *  default), a Groq key when `groq` is set, and the 均衡 and 轻量 models on disk. */
async function state(engines: EngineSettings, { groq = false } = {}): Promise<UiState> {
  const backend = new MockBackend({
    settings: { engines },
    providerKeys: groq ? [{ provider: "groq", kind: "asr" }] : [],
    models: {
      "qwen3-asr-0.6b": INSTALLED,
      "sense-voice-small": INSTALLED,
      "zipformer-stream-zh-en": INSTALLED,
    },
  });
  return backend.getState();
}

const en = createTranslator("en");

const rows = (sections: readonly MenuSection[]) =>
  sections.map((s) => ({
    label: s.label,
    items: s.items.map((i) =>
      i.kind === "radio"
        ? `${i.checked ? "✓ " : ""}${i.label}${i.detail ? ` · ${i.detail}` : ""}${i.disabled ? " (off)" : ""}`
        : `> ${i.label}`,
    ),
  }));

describe("switcher choices", () => {
  it("round-trips every kind of row id, a model id with colons included, and rejects the rest", () => {
    for (const choice of [
      { kind: "remote", provider: "groq", model: "whisper-large-v3-turbo" },
      { kind: "remote", provider: "ollama", model: "qwen3:8b" },
      { kind: "local", id: "sense-voice-small" },
      { kind: "microphone", device: "alsa:hw:1,0" },
      { kind: "microphone", device: null },
      { kind: "source", source: "mixed" },
      { kind: "manage", target: "speech" },
    ] as const) {
      expect(parseChoice(choiceId(choice))).toEqual(choice);
    }
    const foreign = [
      "",
      "manage",
      "remote:",
      "remote:nobody:x",
      "remote:groq:",
      "local:",
      "manage:other",
      "unavailable:openai",
      "source:radio",
      "proofread",
    ];
    expect(foreign.map((id) => [id, parseChoice(id)])).toEqual(
      foreign.map((id) => [id, undefined]),
    );
  });
});

describe("the 语音模型 menu", () => {
  it("lists the built-in service, the installed local models with their product, the providers to set up, and the page", async () => {
    const ui = await state(defaultEngineSettings());
    const sections = speechMenuSections(ui.engines, ui.models);
    expect(rows(sections)).toEqual([
      { label: "内置服务", items: ["✓ Qwen3-ASR-1.7B"] },
      { label: "本地模型", items: ["均衡 · Qwen3-ASR 0.6B", "轻量 · SenseVoice Small"] },
      {
        label: "需要配置",
        items: [
          "OpenAI · 缺少密钥 (off)",
          "Groq · 缺少密钥 (off)",
          "硅基流动 · 缺少密钥 (off)",
          "自定义接口 · 缺少接口地址 (off)",
        ],
      },
      { label: undefined, items: ["> 管理语音模型…"] },
    ]);
  });

  it("checks the local model in use and a provider's model once its key is saved, in English too", async () => {
    const onDevice = await state({
      ...defaultEngineSettings(),
      asr_provider: "local",
      local_model: "qwen3-asr-0.6b",
    });
    const local = speechMenuSections(onDevice.engines, onDevice.models);
    expect(rows(local)[1]).toEqual({
      label: "本地模型",
      items: ["✓ 均衡 · Qwen3-ASR 0.6B", "轻量 · SenseVoice Small"],
    });
    const keyed = await state({ ...defaultEngineSettings(), asr_provider: "groq" }, { groq: true });
    const groq = speechMenuSections(keyed.engines, [], en.t, "en");
    expect(rows(groq).map((s) => s.label)).toEqual([
      "Built-in service",
      "Groq",
      "Needs setup",
      undefined,
    ]);
    expect(rows(groq)[1]).toEqual({
      label: "Groq",
      items: ["✓ whisper-large-v3-turbo", "whisper-large-v3"],
    });
    expect(rows(groq).at(-1)).toEqual({ label: undefined, items: ["> Manage speech models…"] });
  });
});

describe("the AI 润色模型 menu", () => {
  it("lists every model of the providers that can run, checks the one in use, and ends with the page", async () => {
    const ui = await state({ ...defaultEngineSettings(), llm_provider: "groq" }, { groq: true });
    const sections = polishMenuSections(ui.engines);
    expect(rows(sections)[0]).toEqual({ label: "内置服务", items: ["qwen3.8-27b"] });
    expect(rows(sections)[1]).toEqual({
      label: "Groq",
      items: ["✓ qwen3.8-27b", "gpt-oss-20b", "llama-3.3-70b-versatile"],
    });
    expect(rows(sections).at(-2)?.label).toBe("需要配置");
    expect(rows(sections).at(-1)).toEqual({ label: undefined, items: ["> 管理 AI 模型…"] });
  });
});

describe("the 麦克风 menu", () => {
  const DEVICES: AudioDevice[] = [
    { id: "a", name: "Fifine K669 USB Microphone", is_default: true },
    { id: "b", name: "MacBook Pro Microphone", is_default: false },
  ];

  it("offers the system default by name and every device by its short name, the chosen one checked", () => {
    expect(rows(microphoneMenuSections(DEVICES, null))).toEqual([
      { label: "输入设备", items: ["✓ 系统默认（Fifine K669）", "Fifine K669", "MacBook Pro"] },
      { label: undefined, items: ["> 录音来源设置…"] },
    ]);
    expect(rows(microphoneMenuSections(DEVICES, "b"))[0]?.items).toEqual([
      "系统默认（Fifine K669）",
      "Fifine K669",
      "✓ MacBook Pro",
    ]);
    expect(rows(microphoneMenuSections([], undefined))[0]?.items).toEqual(["✓ 系统默认"]);
  });

  it("adds what a take records where the computer's sound can be recorded, and nothing where it cannot", () => {
    const offered = rows(
      microphoneMenuSections(DEVICES, null, undefined, { current: "mixed", available: true }),
    );
    expect(offered.map((s) => s.label)).toEqual(["输入设备", "录音来源", undefined]);
    expect(offered[1]?.items).toEqual(["麦克风", "电脑声音", "✓ 混合"]);
    const without = rows(
      microphoneMenuSections(DEVICES, null, undefined, { current: "microphone", available: false }),
    );
    expect(without.map((s) => s.label)).toEqual(["输入设备", undefined]);
  });

  it("shows a chosen microphone that is not connected, checked and not choosable", () => {
    expect(rows(microphoneMenuSections(DEVICES, "Blue Yeti"))[0]?.items).toEqual([
      "系统默认（Fifine K669）",
      "Fifine K669",
      "MacBook Pro",
      "✓ Blue Yeti · 未连接 (off)",
    ]);
  });
});

describe("engineSettingsFor", () => {
  it("switches the provider, keeps the endpoint the user saved, and leaves the built-in service without settings", () => {
    const saved: EngineSettings = {
      ...defaultEngineSettings(),
      providers: { groq: { asr_url: "https://proxy.example.test/v1" } },
    };
    expect(
      engineSettingsFor(saved, "asr", {
        kind: "remote",
        provider: "groq",
        model: "whisper-large-v3",
      }),
    ).toEqual({
      ...saved,
      asr_provider: "groq",
      providers: {
        groq: { asr_url: "https://proxy.example.test/v1", asr_model: "whisper-large-v3" },
      },
    });
    expect(
      engineSettingsFor(saved, "llm", {
        kind: "remote",
        provider: "deepseek",
        model: "deepseek-flash",
      }),
    ).toEqual({
      ...saved,
      llm_provider: "deepseek",
      providers: { ...saved.providers, deepseek: { llm_model: "deepseek-flash" } },
    });
    expect(
      engineSettingsFor(saved, "llm", {
        kind: "remote",
        provider: "builtin",
        model: "qwen/qwen3.8-27b",
      }),
    ).toEqual({
      ...saved,
      llm_provider: "builtin",
    });
  });

  it("turns a local model into on-device recognition, and refuses what is not a model of the service", () => {
    const settings = defaultEngineSettings();
    expect(engineSettingsFor(settings, "asr", { kind: "local", id: "sense-voice-small" })).toEqual({
      ...settings,
      asr_provider: "local",
      local_model: "sense-voice-small",
    });
    expect(
      engineSettingsFor(settings, "llm", { kind: "local", id: "sense-voice-small" }),
    ).toBeUndefined();
    expect(
      engineSettingsFor(settings, "asr", { kind: "remote", provider: "local", model: "x" }),
    ).toBeUndefined();
    expect(
      engineSettingsFor(settings, "asr", { kind: "microphone", device: null }),
    ).toBeUndefined();
    expect(
      engineSettingsFor(settings, "asr", { kind: "manage", target: "speech" }),
    ).toBeUndefined();
  });
});
