// The menus of a 重新识别 (docs/dictation.md §26.4, user request 2026-10-26): which models they
// offer, which one is checked, and what `audio_retake` is sent.
import {
  type EngineSettings,
  type ModelInstallState,
  type UiState,
  createTranslator,
  defaultEngineSettings,
} from "@voltip/shared";
import { MockBackend } from "@voltip/shared/mock";
import type { MenuSection } from "@voltip/ui";
import { POLISH_OFF_ID, retakeMenus } from "./RetakeModels";

const INSTALLED: ModelInstallState = { kind: "installed", path: "/models/x", installed_at: 1 };
const zh = createTranslator("zh-CN");

/** The preview's state for `engines`: the built-in service in both roles and the 均衡 model on
 *  disk. */
async function state(engines: EngineSettings): Promise<UiState> {
  return new MockBackend({
    settings: { engines },
    models: { "qwen3-asr-0.6b": INSTALLED },
  }).getState();
}

const custom: EngineSettings = {
  ...defaultEngineSettings(),
  providers: {
    custom: {
      asr_url: "https://asr.example.test/v1",
      asr_model: "whisper-1",
      llm_url: "https://llm.example.test/v1",
      llm_model: "gpt-4o-mini",
    },
  },
};

/** The radio rows, the checked ones marked. */
const radios = (sections: readonly MenuSection[]) =>
  sections.flatMap((s) =>
    s.items.flatMap((i) => (i.kind === "radio" ? [`${i.checked ? "✓ " : ""}${i.label}`] : [])),
  );

describe("retakeMenus", () => {
  it("with nothing chosen checks the settings' models and sends no choice", async () => {
    const s = await state(custom);
    const menus = retakeMenus(s.engines, s.models, {}, zh.t, "zh-CN");
    expect(radios(menus.speech)).toEqual(["✓ Qwen3-ASR-1.7B", "whisper-1", "均衡"]);
    expect(radios(menus.polish)).toEqual([
      "不润色",
      "✓ qwen3.8-27b",
      "gpt-oss-120b",
      "gpt-oss-20b",
      "gpt-4o-mini",
    ]);
    expect(menus.models).toEqual({});
  });

  it("with the clean-up off in the settings checks 不润色", async () => {
    const s = await state({ ...custom, refine_enabled: false });
    const menus = retakeMenus(s.engines, s.models, {}, zh.t, "zh-CN");
    expect(radios(menus.polish).filter((r) => r.startsWith("✓"))).toEqual(["✓ 不润色"]);
    expect(menus.polish[0]?.items[0]).toMatchObject({ id: POLISH_OFF_ID, checked: true });
  });

  it("a choice checks itself alone and is what it sends", async () => {
    const s = await state(custom);
    const menus = retakeMenus(
      s.engines,
      s.models,
      {
        asr: { kind: "local", id: "qwen3-asr-0.6b" },
        refine: { kind: "remote", provider: "custom", model: "gpt-4o-mini" },
      },
      zh.t,
      "zh-CN",
    );
    expect(radios(menus.speech)).toEqual(["Qwen3-ASR-1.7B", "whisper-1", "✓ 均衡"]);
    expect(radios(menus.polish).filter((r) => r.startsWith("✓"))).toEqual(["✓ gpt-4o-mini"]);
    expect(menus.models).toEqual({
      asr: { kind: "local", id: "qwen3-asr-0.6b" },
      refine: { kind: "remote", provider: "custom", model: "gpt-4o-mini" },
    });
    const off = retakeMenus(s.engines, s.models, { refine: { kind: "off" } }, zh.t, "zh-CN");
    expect(radios(off.polish).filter((r) => r.startsWith("✓"))).toEqual(["✓ 不润色"]);
    expect(off.models).toEqual({ refine: { kind: "off" } });
  });

  it("a choice its menu no longer offers is the settings' again", async () => {
    // Chosen while the custom endpoint was set up; it is gone now.
    const s = await state(defaultEngineSettings());
    const menus = retakeMenus(
      s.engines,
      s.models,
      {
        asr: { kind: "remote", provider: "custom", model: "whisper-1" },
        refine: { kind: "remote", provider: "custom", model: "gpt-4o-mini" },
      },
      zh.t,
      "zh-CN",
    );
    expect(menus.models).toEqual({});
    expect(radios(menus.speech)).toEqual(["✓ Qwen3-ASR-1.7B", "均衡"]);
    expect(radios(menus.polish).filter((r) => r.startsWith("✓"))).toEqual(["✓ qwen3.8-27b"]);
  });
});
