import { describe, expect, it } from "vitest";
import {
  asrStreams,
  glossaryRoute,
  isDashscope,
  keyEntry,
  livePlan,
  resolveEngineStatus,
} from "./providers";
import { type EngineStatus, type OutputMode, defaultEngineSettings } from "./schema";

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
    // The streaming model is installed: 本地模型预览 is ready while its switch is on.
    const input = (
      model: string,
      switches: { provider_preview?: boolean; local_preview?: boolean } = {},
      localPreviewReady = switches.local_preview ?? true,
    ) => ({
      settings: {
        ...defaultEngineSettings(),
        asr_provider: "aliyun" as const,
        ...switches,
        providers: { aliyun: { asr_model: model } },
      },
      userKeys: new Set(["provider-key.aliyun"]),
      builtIn: {},
      local: { id: "qwen3-asr-0.6b", name: "均衡", installed: false },
      localPreviewReady,
    });
    const streaming = resolveEngineStatus(input("qwen-audio-3.1-asr-flash-streaming"));
    expect(streaming.asr_ready).toBe(true);
    expect(streaming.asr_host).toBe("dashscope.aliyuncs.com");
    expect(streaming.live_source).toBe("stream");
    expect(streaming.effective_output_mode).toBe("streaming_final");
    const whole = resolveEngineStatus(input("qwen-audio-3.1-asr-flash"));
    expect(whole.live_source).toBe("local");
    expect(whole.effective_output_mode).toBe("whole_take");
    // docs/dictation.md §11.10: the provider's words off, the realtime model still streams the
    // take; only nothing of it is shown.
    const hidden = resolveEngineStatus(
      input("qwen-audio-3.1-asr-flash-streaming", {
        provider_preview: false,
        local_preview: false,
      }),
    );
    expect(hidden).toMatchObject({ live_ready: true, live_source: "stream" });
    expect(hidden.preview_source).toBeUndefined();
    expect(hidden.effective_output_mode).toBe("streaming_final");
    const card = streaming.providers.find((p) => p.id === "aliyun");
    expect(card?.asr?.presets[0]).toBe("qwen-audio-3.1-asr-flash-streaming");
    expect(card?.llm?.default_base_url).toBe("https://dashscope.aliyuncs.com/compatible-mode/v1");
    expect(card?.console).toBe(true);
  });

  it("the two live preview switches pick their own sources and the provider's words come first (docs/dictation.md §11.10)", () => {
    const REALTIME = "qwen-audio-3.1-asr-flash-streaming";
    const WHOLE = "qwen-audio-3.1-asr-flash";
    const resolve = (
      provider: "aliyun" | "builtin" | "custom",
      model: string,
      provider_preview: boolean,
      local_preview: boolean,
      installed: boolean,
    ) =>
      resolveEngineStatus({
        settings: {
          ...defaultEngineSettings(),
          asr_provider: provider,
          provider_preview,
          local_preview,
          providers: {
            aliyun: { asr_model: model },
            custom: { asr_url: "http://127.0.0.1:9000/v1", asr_model: model },
          },
        },
        userKeys: new Set(["provider-key.aliyun"]),
        builtIn: { asr: { model: "Qwen/Qwen3-ASR-1.7B", key: true, preview: true } },
        local: { id: "qwen3-asr-0.6b", name: "均衡", installed: false },
        localPreviewReady: local_preview && installed,
      });
    const cases = [
      // [provider, model, 服务商的实时文字, 本地模型预览, model installed] → [live_source, preview_source, preview_provider, effective, live_ready]
      [
        ["aliyun", REALTIME, true, true, true],
        ["stream", "stream", "aliyun", "streaming_final", true],
      ],
      [
        ["aliyun", REALTIME, false, true, true],
        ["stream", "local", undefined, "streaming_final", true],
      ],
      [
        ["aliyun", REALTIME, false, false, true],
        ["stream", undefined, undefined, "streaming_final", true],
      ],
      [
        ["aliyun", REALTIME, false, true, false],
        ["stream", undefined, undefined, "streaming_final", true],
      ],
      [
        ["aliyun", REALTIME, true, false, false],
        ["stream", "stream", "aliyun", "streaming_final", true],
      ],
      [
        ["aliyun", WHOLE, true, true, true],
        ["local", "local", undefined, "whole_take", true],
      ],
      [
        ["aliyun", WHOLE, true, false, true],
        [undefined, undefined, undefined, "whole_take", false],
      ],
      [
        ["builtin", "", true, true, true],
        ["cloud", "cloud", "builtin", "whole_take", true],
      ],
      [
        ["builtin", "", false, true, true],
        ["local", "local", undefined, "whole_take", true],
      ],
      // Changed on purpose 2026-11-23: nothing recognises a whole take while it is spoken, but the
      // streaming modes would still take effect, the built-in service writing their text.
      [
        ["builtin", "", false, false, true],
        [undefined, undefined, undefined, "whole_take", true],
      ],
      [
        ["custom", "whisper", true, true, true],
        ["local", "local", undefined, "whole_take", true],
      ],
    ] as const;
    for (const [[provider, model, pp, lp, installed], expected] of cases) {
      const st = resolve(provider, model, pp, lp, installed);
      expect(
        [
          st.live_source,
          st.preview_source,
          st.preview_provider,
          st.effective_output_mode,
          st.live_ready,
        ],
        `${provider} ${model} provider_preview=${pp} local_preview=${lp} installed=${installed}`,
      ).toEqual(expected);
    }
  });

  it("regression: in 边说边识别 and 边说边输入 the built-in service writes the text whatever 服务商的实时文字 says; the switch only decides whether its words show (docs/dictation.md §11.10, §12)", () => {
    // User report 2026-11-23 (Windows): 边说边识别 on the built-in service with 服务商的实时文字
    // off took its text from the small on-device streaming model (实时预览, Zipformer): poor
    // accuracy, and the last words of a take were lost. As the core now resolves it
    // (engines.rs `the_built_in_service_writes_the_streaming_text_whatever_its_words_switch_says`):
    // a streaming take's text is the built-in service's, sentence by sentence; with its words off
    // the pill shows the local model's words when it can, else none; 整段输出 is unchanged.
    const resolve = (output_mode: OutputMode, provider_preview: boolean, local: boolean) =>
      resolveEngineStatus({
        settings: {
          ...defaultEngineSettings(),
          output_mode,
          provider_preview,
          local_preview: local,
        },
        userKeys: new Set(),
        builtIn: { asr: { model: "Qwen/Qwen3-ASR-1.7B", key: true, preview: true } },
        local: { id: "qwen3-asr-0.6b", name: "均衡", installed: false },
        localPreviewReady: local,
      });
    const row = (st: EngineStatus) => [
      st.live_source,
      st.preview_source,
      st.preview_provider,
      st.effective_output_mode,
      st.live_ready,
    ];
    const cases = [
      // [output mode, 服务商的实时文字, 本地模型预览 ready] → [live_source, preview_source, preview_provider, effective, live_ready]
      [
        ["streaming_final", false, true],
        ["cloud", "local", undefined, "streaming_final", true],
      ],
      [
        ["streaming_final", false, false],
        ["cloud", undefined, undefined, "streaming_final", true],
      ],
      [
        ["live_inject", false, true],
        ["cloud", "local", undefined, "live_inject", true],
      ],
      [
        ["live_inject", false, false],
        ["cloud", undefined, undefined, "live_inject", true],
      ],
      // Its words on: they show, in every mode, as before.
      [
        ["streaming_final", true, true],
        ["cloud", "cloud", "builtin", "streaming_final", true],
      ],
      [
        ["live_inject", true, false],
        ["cloud", "cloud", "builtin", "live_inject", true],
      ],
      [
        ["whole_take", true, false],
        ["cloud", "cloud", "builtin", "whole_take", true],
      ],
      // 整段输出 with its words off sends nothing while the take is spoken; the local model shows
      // its own words when it can.
      [
        ["whole_take", false, true],
        ["local", "local", undefined, "whole_take", true],
      ],
      [
        ["whole_take", false, false],
        [undefined, undefined, undefined, "whole_take", true],
      ],
    ] as const;
    for (const [[mode, pp, local], expected] of cases)
      expect(
        row(resolve(mode, pp, local)),
        `${mode} provider_preview=${pp} local=${local}`,
      ).toEqual(expected);
    // `LiveSources::plan`: the built-in service recognises a streaming take, shown or not; a whole
    // take only with its words on. Other sources do not depend on the mode.
    const cloud = { cloud: true, stream: false, providerShown: false, local: true };
    expect(livePlan(cloud, true, true, true)).toEqual({ text: "cloud", shown: "local" });
    expect(livePlan({ ...cloud, local: false }, true, true, true)).toEqual({ text: "cloud" });
    expect(livePlan(cloud, true, true, false)).toEqual({ text: "local", shown: "local" });
    expect(livePlan({ ...cloud, local: false }, true, true, false)).toEqual({});
    expect(livePlan({ ...cloud, providerShown: true }, true, true, false)).toEqual({
      text: "cloud",
      shown: "cloud",
    });
    const stream = { cloud: false, stream: true, providerShown: false, local: true };
    for (const streaming of [true, false])
      expect(livePlan(stream, true, true, streaming)).toEqual({ text: "stream", shown: "local" });
    // A model taking whole recordings stands in for the realtime one (`stream_now` false): a whole
    // take previews with the local model; a streaming take has no live source since 2026-11-23 (it
    // took the local model's words before, the rule changed on purpose, see the next test).
    expect(livePlan(stream, false, true, false)).toEqual({ text: "local", shown: "local" });
    expect(livePlan(stream, false, true, true)).toEqual({});
    // A fallback model standing in for the built-in service (2026-11-24): a streaming take has no
    // source (it runs whole); a whole take previews on the local model.
    expect(livePlan(cloud, true, false, true)).toEqual({});
    expect(livePlan({ ...cloud, providerShown: true }, true, false, false)).toEqual({
      text: "local",
      shown: "local",
    });
  });

  it("regression: while a fallback model stands in for the built-in service, 边说边识别 and 边说边输入 run as a whole take and the local model only previews (docs/dictation.md §3.5, §11.10, §12)", () => {
    // The user's decision of 2026-11-24 (「候补模型顶替时可以修改为整段识别」): with the built-in
    // service's day used up, its sentence-by-sentence recognition went on through the chain, a
    // request a sentence to the fallback service. As the core resolves it now (engines.rs
    // `regression_a_fallback_standing_in_for_the_built_in_service_makes_the_streaming_modes_whole`).
    const resolve = (output_mode: OutputMode, out: boolean, localReady = true) =>
      resolveEngineStatus({
        settings: {
          ...defaultEngineSettings(),
          output_mode,
          asr_fallback: {
            enabled: true,
            models: [{ provider: "groq", model: "whisper-large-v3" }],
          },
        },
        userKeys: new Set(["provider-key.groq"]),
        builtIn: { asr: { model: "Qwen/Qwen3-ASR-1.7B", key: true, preview: true } },
        local: { id: "qwen3-asr-0.6b", name: "均衡", installed: false },
        localPreviewReady: localReady,
        ...(out ? { quotaOut: { asr: 99 } } : {}),
      });
    const row = (st: EngineStatus) => [
      st.effective_output_mode,
      st.live_ready,
      st.live_source,
      st.preview_source,
      st.preview_provider,
    ];
    for (const mode of ["streaming_final", "live_inject"] as const) {
      expect(row(resolve(mode, false)), `${mode}: the built-in service`).toEqual([
        mode,
        true,
        "cloud",
        "cloud",
        "builtin",
      ]);
      expect(row(resolve(mode, true)), `${mode}: a whole take through the chain`).toEqual([
        "whole_take",
        false,
        "local",
        "local",
        undefined,
      ]);
      // Without the local model nothing previews and the live source stays the built-in service,
      // so the settings page names the fallback model, not a switch or a download, as the reason.
      expect(row(resolve(mode, true, false)), `${mode}: no local model`).toEqual([
        "whole_take",
        false,
        "cloud",
        undefined,
        undefined,
      ]);
    }
  });

  it("regression: while a fallback model that takes whole recordings stands in for the realtime model, 边说边识别 and 边说边输入 run as a whole take and the local model only previews (docs/dictation.md §3.5, §11.10, §12)", () => {
    // The user's decision after the review of 边说边输入 on realtime models, 2026-11-23: with the
    // realtime model's day used up and Model Studio's whole-file model standing in, the local
    // streaming model wrote the streaming modes' text (poor words, the ends lost on windows-local
    // that day). As the core resolves it now (engines.rs
    // `regression_a_whole_take_fallback_standing_in_makes_the_streaming_modes_whole`): the
    // streaming modes run as a whole take through the fallback chain (`live_ready` false, the
    // settings page's 候补模型 notice), and the sources are a whole take's: the local model shows
    // its words when it can.
    const resolve = (output_mode: OutputMode, localReady: boolean, out: boolean) =>
      resolveEngineStatus({
        settings: {
          ...defaultEngineSettings(),
          asr_provider: "aliyun",
          output_mode,
          local_preview: localReady,
          providers: { aliyun: { asr_model: "qwen-audio-3.1-asr-flash-streaming" } },
          asr_fallback: {
            enabled: true,
            models: [{ provider: "aliyun", model: "qwen-audio-3.1-asr-flash" }],
          },
        },
        userKeys: new Set(["provider-key.aliyun"]),
        builtIn: { asr: { model: "Qwen/Qwen3-ASR-1.7B", key: true, preview: true } },
        local: { id: "qwen3-asr-0.6b", name: "均衡", installed: false },
        localPreviewReady: localReady,
        ...(out ? { quotaOut: { asr: 99 } } : {}),
      });
    const row = (st: EngineStatus) => [
      st.effective_output_mode,
      st.live_ready,
      st.live_source,
      st.preview_source,
      st.preview_provider,
    ];
    for (const mode of ["streaming_final", "live_inject"] as const) {
      // The realtime model has quota left: it streams the take.
      expect(row(resolve(mode, true, false)), `${mode}: the realtime model`).toEqual([
        mode,
        true,
        "stream",
        "stream",
        "aliyun",
      ]);
      expect(row(resolve(mode, true, true)), `${mode}: a whole take through the chain`).toEqual([
        "whole_take",
        false,
        "local",
        "local",
        undefined,
      ]);
      // Without the local model nothing previews; the live source stays the realtime model.
      expect(row(resolve(mode, false, true)), `${mode}: no local model`).toEqual([
        "whole_take",
        false,
        "stream",
        undefined,
        undefined,
      ]);
    }
    // 整段输出 previews the same way; its `live_ready` reads the streaming modes', paused too.
    expect(row(resolve("whole_take", true, true))).toEqual([
      "whole_take",
      false,
      "local",
      "local",
      undefined,
    ]);
  });
});
