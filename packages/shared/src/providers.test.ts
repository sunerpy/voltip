import { describe, expect, it } from "vitest";
import { asrStreams, glossaryRoute, isDashscope, keyEntry, resolveEngineStatus } from "./providers";
import { defaultEngineSettings } from "./schema";

describe("Model Studio's recognition protocols (docs/dictation.md §3.4)", () => {
  it("a realtime model at a Model Studio address streams, like `AsrProtocol::streams`", () => {
    const studio = [
      "https://dashscope.aliyuncs.com/compatible-mode/v1",
      "https://ws-test.cn-beijing.maas.aliyuncs.com/compatible-mode/v1",
      "HTTPS://DashScope-Intl.AliyunCS.com/api/v1/",
    ];
    const streams = [
      "qwen-audio-3.1-asr-flash-streaming",
      "qwen-audio-3.0-asr-flash-streaming",
      "qwen-audio-3.1-asr-flash-message",
      " Qwen-Audio-3.1-ASR-Flash-Streaming-2026-12-01 ",
      "fun-asr-realtime",
      "fun-asr-flash-8k-realtime",
      "paraformer-realtime-v2",
    ];
    const whole = [
      "qwen-audio-3.1-asr-flash",
      "fun-asr-flash-2026-06-15",
      "qwen3-asr-flash",
      "qwen3-asr-flash-realtime",
      "qwen-audio-3.1-asr-flash-filetrans",
      "fun-asr",
      "paraformer-v2",
      "qwen3.8-omni-flash",
    ];
    for (const url of studio) {
      expect(isDashscope(url)).toBe(true);
      for (const model of streams)
        expect(asrStreams("aliyun", url, model), `${url} ${model}`).toBe(true);
      for (const model of whole)
        expect(asrStreams("aliyun", url, model), `${url} ${model}`).toBe(false);
    }
    for (const other of [
      "https://api.openai.com/v1",
      "http://127.0.0.1:8000/v1",
      "https://aliyuncs.com.example.test/v1",
      "not a url",
      "",
    ]) {
      expect(isDashscope(other)).toBe(false);
      expect(asrStreams("custom", other, "qwen-audio-3.1-asr-flash-streaming")).toBe(false);
    }
  });

  it("a vendor with a protocol of its own keeps it at any address (docs/dictation.md §3.15)", () => {
    const proxy = "https://proxy.example.test/asr";
    const realtime = ["volcengine_speech", "tencent", "iflytek", "iflytek_rtasr", "baidu"] as const;
    expect(realtime.filter((provider) => !asrStreams(provider, proxy, "any"))).toEqual([]);
    expect(asrStreams("stepfun", proxy, " StepAudio-2.5-ASR-Stream ")).toBe(true);
    expect(asrStreams("stepfun", proxy, "stepaudio-2.5-asr")).toBe(false);
    expect(asrStreams("glm", proxy, "glm-asr-2512")).toBe(false);
    expect(asrStreams("mimo", proxy, "mimo-v2.5-asr")).toBe(false);
    expect(glossaryRoute("volcengine_speech", proxy, "volc.seedasr.sauc.duration")).toBe(
      "hotwords",
    );
    expect(glossaryRoute("tencent", proxy, "16k_zh_en")).toBe("hotwords");
    expect(glossaryRoute("glm", proxy, "glm-asr-2512")).toBe("hotwords");
    expect(glossaryRoute("stepfun", proxy, "stepaudio-2.5-asr")).toBe("hotwords");
    expect(glossaryRoute("stepfun", proxy, "stepaudio-2.5-asr-stream")).toBe("prompt");
    const none = ["iflytek", "iflytek_rtasr", "baidu", "mimo"] as const;
    expect(none.map((provider) => glossaryRoute(provider, proxy, "m"))).toEqual(
      none.map(() => "not_supported"),
    );
    expect(glossaryRoute("openai", "https://api.openai.com/v1", "whisper-1")).toBe("prompt");
    const studio = "https://dashscope.aliyuncs.com/compatible-mode/v1";
    expect(
      [
        "qwen-audio-3.1-asr-flash",
        "fun-asr-realtime",
        "fun-asr-flash-2026-06-15",
        "fun-asr-flash-8k-realtime",
      ].map((m) => glossaryRoute("aliyun", studio, m)),
    ).toEqual(["hotwords", "prompt", "prompt", "not_supported"]);
    expect(keyEntry("volcengine_speech", "asr")).toBe("provider-key.volcengine-speech");
    expect(keyEntry("iflytek_rtasr", "asr")).toBe("provider-key.iflytek-rtasr");
  });

  it("the preview resolves a realtime model's stream as the live source and its text as final", () => {
    const input = (model: string, live_preview = true) => ({
      settings: {
        ...defaultEngineSettings(),
        asr_provider: "aliyun" as const,
        live_preview,
        providers: { aliyun: { asr_model: model } },
      },
      userKeys: new Set(["provider-key.aliyun"]),
      builtIn: {},
      local: { id: "qwen3-asr-0.6b", name: "均衡", installed: false },
      liveReady: true,
    });
    const streaming = resolveEngineStatus(input("qwen-audio-3.1-asr-flash-streaming"));
    expect(streaming.asr_ready).toBe(true);
    expect(streaming.asr_host).toBe("dashscope.aliyuncs.com");
    expect(streaming.live_source).toBe("stream");
    expect(streaming.effective_output_mode).toBe("streaming_final");
    const whole = resolveEngineStatus(input("qwen-audio-3.1-asr-flash"));
    expect(whole.live_source).toBe("local");
    expect(whole.effective_output_mode).toBe("whole_take");
    const off = resolveEngineStatus(input("qwen-audio-3.1-asr-flash-streaming", false));
    expect(off.live_source).toBeUndefined();
    expect(off.effective_output_mode).toBe("whole_take");
    const card = streaming.providers.find((p) => p.id === "aliyun");
    expect(card?.asr?.presets[0]).toBe("qwen-audio-3.1-asr-flash-streaming");
    expect(card?.llm?.default_base_url).toBe("https://dashscope.aliyuncs.com/compatible-mode/v1");
    expect(card?.console).toBe(true);
  });
});
