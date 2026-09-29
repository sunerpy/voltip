import {
  activationChip,
  activationDescription,
  activationHint,
  activationLabel,
  activationShortcut,
  connectionKindLabel,
  connectionLabel,
  dictationFailureText,
  dictationPhaseLabel,
  failureLabel,
  formatCode,
  formatCount,
  durationParts,
  formatDate,
  formatDuration,
  formatElapsed,
  formatMs,
  formatRemaining,
  formatSeconds,
  coreMessageText,
  hostOf,
  hotkeyMethodText,
  joinLiveText,
  liveCaptionParts,
  livePreviewText,
  liveTextGap,
  modelDescription,
  modelDictionaryKey,
  takeFailureText,
  takePhaseLabel,
  modelDisplayName,
  modelTierLabel,
  outcomeLabel,
  outputModeDescription,
  outputModeLabel,
  pairingStateLabel,
  platformLabel,
  processingStageLabel,
  recordingSourceLabel,
  relativeTime,
  relayLabel,
  secretStateLabel,
  segmentsDoneLabel,
  shortFingerprint,
  shortKey,
  themeName,
  themeSubtitle,
  viaLabel,
} from "./labels";
import type { DictationFailureCode, DictationPhase } from "./schema";
import { MESSAGES } from "./i18n";
import { type MessageTree, leafPaths, lookup } from "./i18n/runtime";

/** Every text of a dictionary (plural forms both). */
function leafTexts(tree: MessageTree): string[] {
  return leafPaths(tree).flatMap((path) => {
    const leaf = lookup(tree, path);
    if (leaf === undefined) return [];
    return typeof leaf === "string" ? [leaf] : [leaf.one, leaf.other];
  });
}

describe("coreMessageText", () => {
  it("regression: a core refusal shows without its machine prefix (docs/frontend.md §8)", () => {
    expect(coreMessageText("scenes: 已有名为「聊天」的场景")).toBe("已有名为「聊天」的场景");
    expect(coreMessageText("phone text: 没有要发送的文字")).toBe("没有要发送的文字");
    expect(coreMessageText("history.keep: 500–20000")).toBe("500–20000");
    expect(coreMessageText("openai.asr_url: 地址无效")).toBe("地址无效");
    // A message without one, and text that only looks like it, stay as they are.
    expect(coreMessageText("Ctrl+Alt+Space 没能生效：纯 Wayland")).toBe(
      "Ctrl+Alt+Space 没能生效：纯 Wayland",
    );
    expect(coreMessageText("relay refused the ticket")).toBe("relay refused the ticket");
    expect(coreMessageText("Error: boom")).toBe("Error: boom");
  });
});

describe("hotkeyMethodText", () => {
  it("regression: the shortcut method names the system and the Linux session, not the library or the system call", () => {
    // The shell's table (apps/desktop/src-tauri/src/hotkey.rs backend_name_for).
    expect(hotkeyMethodText("global-shortcut · Windows · RegisterHotKey")).toBe("Windows");
    expect(hotkeyMethodText("global-shortcut · macOS · Carbon")).toBe("macOS");
    expect(hotkeyMethodText("global-shortcut · Linux · X11")).toBe("Linux · X11");
    expect(hotkeyMethodText("global-shortcut · Linux · XWayland")).toBe("Linux · XWayland");
    expect(hotkeyMethodText("global-shortcut · Linux · Wayland")).toBe("Linux · Wayland");
    // Anything else is shown as it is.
    expect(hotkeyMethodText("mock · browser preview")).toBe("mock · browser preview");
    expect(hotkeyMethodText("global-shortcut")).toBe("global-shortcut");
  });
});

describe("labels", () => {
  it("labels platforms and connections", () => {
    expect(platformLabel("macos")).toBe("macOS");
    expect(connectionLabel({ state: "online", via: "direct" })).toEqual({
      text: "在线 · 直连",
      tone: "ok",
    });
    expect(connectionLabel({ state: "online", via: "relay" }).text).toBe("在线 · 经中继");
    expect(connectionLabel({ state: "connecting" }).tone).toBe("accent");
    expect(connectionLabel({ state: "offline" }).text).toBe("离线");
    expect(connectionLabel({ state: "identity_changed", presented_fingerprint: "x" }).tone).toBe(
      "danger",
    );
    expect(connectionKindLabel("direct")).toBe("直连");
    expect(connectionKindLabel("relay")).toBe("中继");
    expect(connectionKindLabel(undefined)).toBe("—");
  });

  it("labels relay states including reconnect attempts", () => {
    expect(relayLabel({ state: "connected", attempts: 0, source: "builtin" })).toEqual({
      text: "已连接",
      tone: "ok",
    });
    expect(relayLabel({ state: "connecting", attempts: 0, source: "builtin" }).tone).toBe("accent");
    expect(relayLabel({ state: "authenticating", attempts: 0, source: "builtin" }).text).toBe(
      "认证中",
    );
    expect(relayLabel({ state: "reconnecting", attempts: 3, source: "builtin" }).text).toBe(
      "重连中 · 第 3 次",
    );
    expect(relayLabel({ state: "reconnecting", attempts: 0, source: "builtin" }).text).toBe(
      "重连中",
    );
    expect(relayLabel({ state: "disconnected", attempts: 0, source: "none" }).text).toBe("未配置");
    expect(relayLabel({ state: "disconnected", attempts: 0, source: "builtin" }).text).toBe(
      "未连接",
    );
    expect(relayLabel({ state: "closed", attempts: 0, source: "builtin" }).tone).toBe("danger");
  });

  it("labels pairing states and failure reasons", () => {
    expect(pairingStateLabel({ state: "idle" }).text).toBe("未开始");
    expect(pairingStateLabel({ state: "creating_session" }).text).toBe("正在准备配对");
    expect(pairingStateLabel({ state: "waiting_for_peer" }).text).toBe("等待对方设备");
    expect(pairingStateLabel({ state: "key_exchange" }).text).toBe("正在建立加密连接");
    expect(pairingStateLabel({ state: "awaiting_verification" }).text).toBe("请核对安全码");
    expect(pairingStateLabel({ state: "trusted" }).tone).toBe("ok");
    expect(pairingStateLabel({ state: "expired" }).text).toBe("已过期");
    expect(pairingStateLabel({ state: "rejected" }).text).toBe("已拒绝");
    expect(pairingStateLabel({ state: "failed", reason: { kind: "timeout" } }).text).toBe(
      "失败 · 超时",
    );
    expect(failureLabel({ kind: "relay", code: "invalid_code" })).toBe("验证码不正确");
    expect(failureLabel({ kind: "relay", code: "weird" })).toBe("中继拒绝连接 · weird");
    expect(failureLabel({ kind: "identity_changed" })).toBe("对方设备的身份已变化");
  });

  it("formats time, codes, fingerprints and counts", () => {
    expect(formatRemaining(107)).toBe("01:47");
    expect(formatRemaining(-5)).toBe("00:00");
    expect(formatCode("483921")).toBe("483 921");
    expect(formatCode("48")).toBe("48");
    expect(formatCode("4839219")).toBe("483 921");
    expect(shortFingerprint("A7:C4:19:8E · 3D:F2:61:09")).toBe("A7C4 … 6109");
    expect(shortFingerprint("AB")).toBe("AB");
    expect(shortKey("9f0c2b1e6a4d3c5f7e8a9b0c1d2e3f405162738495a6b7c8d9e0f1a2b3c4d5e6")).toBe(
      "9f0c…d5e6",
    );
    expect(shortKey("short")).toBe("short");
    expect(formatCount(1842)).toBe("1,842");
    expect(formatDate(1_757_635_200)).toBe("2025-09-12");
  });

  it("renders relative time in Chinese", () => {
    const now = 1_700_000_000;
    expect(relativeTime(undefined, now)).toBe("从未");
    expect(relativeTime(now - 10, now)).toBe("刚刚");
    expect(relativeTime(now - 180, now)).toBe("3 分钟前");
    expect(relativeTime(now - 7200, now)).toBe("2 小时前");
    expect(relativeTime(now - 86_400 * 2, now)).toBe("2 天前");
    expect(relativeTime(now - 86_400 * 40, now)).toBe(formatDate(now - 86_400 * 40));
  });

  it("regression: no string in either dictionary says a feature is not wired yet", () => {
    // Every feature the UI shows is real (2026-09-27); the not-wired wording is gone with them.
    const words = /尚未接入|not wired yet|第二阶段|示例数据/i;
    const offenders = Object.entries(MESSAGES).flatMap(([locale, tree]) =>
      leafTexts(tree)
        .filter((text) => words.test(text))
        .map((text) => `${locale}: ${text}`),
    );
    expect(offenders).toEqual([]);
  });

  it("labels dictation phases with elapsed time, insertion route and failure text", () => {
    const now = 1_000_000;
    expect(dictationPhaseLabel({ phase: "idle" }, now)).toEqual({ text: "就绪", tone: "idle" });
    expect(
      dictationPhaseLabel(
        { phase: "listening", started_at: now - 3200, ready: true, locked: false },
        now,
      ),
    ).toEqual({
      text: "正在录音… 00:03",
      tone: "accent",
    });
    expect(
      dictationPhaseLabel({ phase: "processing", stage: "transcribing", started_at: now }, now)
        .text,
    ).toBe("识别中…");
    expect(
      dictationPhaseLabel({ phase: "processing", stage: "refining", started_at: now }, now).text,
    ).toBe("润色中…");
    expect(
      dictationPhaseLabel({ phase: "processing", stage: "inserting", started_at: now }, now).text,
    ).toBe("插入中…");
    expect(
      dictationPhaseLabel({ phase: "processing", stage: "finalizing", started_at: now }, now).text,
    ).toBe("补齐最后一句…");
    const done = {
      phase: "done" as const,
      text: "x".repeat(42),
      raw_text: "x",
      chars: 42,
      via: "paste" as const,
      refined: true,
      duration_ms: 6800,
      asr_ms: 400,
      refine_ms: 300,
      mode: "whole_take" as const,
    };
    expect(dictationPhaseLabel(done, now)).toEqual({
      text: "已插入 42 字 · 粘贴 · 已润色",
      tone: "ok",
    });
    expect(dictationPhaseLabel({ ...done, via: "clipboard", refined: false }, now).text).toBe(
      "已复制 42 字 · 剪贴板",
    );
    expect(dictationPhaseLabel({ phase: "failed", message: "没有听到声音" }, now)).toEqual({
      text: "失败 · 没有听到声音",
      tone: "danger",
    });
    expect(
      dictationPhaseLabel({ phase: "failed", message: "粘贴超时", text: "abc" }, now).text,
    ).toBe("未插入 · 粘贴超时");
    expect(dictationPhaseLabel({ phase: "cancelled", injected_chars: 0 }, now)).toEqual({
      text: "已取消",
      tone: "idle",
    });
  });

  it("labels history outcomes, secrets, routes and formats durations", () => {
    expect(outcomeLabel({ kind: "inserted", via: "paste" })).toEqual({
      text: "已插入 · 粘贴",
      tone: "ok",
    });
    // The reason is explained under the history entry, never in the label (docs/dictation.md §4.2).
    expect(outcomeLabel({ kind: "clipboard", reason: "目标窗口没有焦点" })).toEqual({
      text: "已复制到剪贴板",
      tone: "warn",
    });
    expect(
      outcomeLabel(
        { kind: "clipboard", reason: "enigo: no permission", code: "no_permission" },
        "en",
      ),
    ).toEqual({ text: "Copied to clipboard", tone: "warn" });
    expect(outcomeLabel({ kind: "failed", reason: "ASR 401" })).toEqual({
      text: "失败 · ASR 401",
      tone: "danger",
    });
    expect(secretStateLabel({ set: true, source: "builtin" })).toEqual({
      text: "已内置",
      tone: "ok",
    });
    expect(secretStateLabel({ set: true, source: "user" })).toEqual({ text: "已设置", tone: "ok" });
    expect(secretStateLabel({ set: false, source: "none" })).toEqual({
      text: "未设置",
      tone: "danger",
    });
    expect(viaLabel("paste")).toBe("粘贴");
    expect(viaLabel("clipboard")).toBe("剪贴板");
    expect(formatElapsed(65_400)).toBe("01:05");
    expect(formatElapsed(-5)).toBe("00:00");
    expect(formatMs(1384)).toBe("1,384 ms");
    expect(formatMs(undefined)).toBe("—");
    expect(formatSeconds(6800)).toBe("6.8 s");
    // The statistics' spans read in words at every size: a 20 000-entry history saves hours, which
    // `m:ss 分` wrote as 「3780:00 分」.
    expect(formatDuration(0)).toBe("0 秒");
    expect(formatDuration(-5)).toBe("0 秒");
    expect(formatDuration(21_400)).toBe("21 秒");
    expect(formatDuration(59_600)).toBe("1 分");
    expect(formatDuration(227_000)).toBe("3 分 47 秒");
    expect(formatDuration(3_600_000)).toBe("1 小时");
    expect(formatDuration(63 * 3_600_000 + 12 * 60_000 + 30_000)).toBe("63 小时 12 分");
    expect(formatDuration(1_234 * 3_600_000)).toBe("1,234 小时");
    expect(formatDuration(227_000, "en")).toBe("3 min 47 s");
    expect(formatDuration(63 * 3_600_000 + 12 * 60_000, "en")).toBe("63 h 12 min");
    // The home page draws each number with its unit smaller, like the character counts.
    expect(durationParts(227_000)).toEqual([
      { value: "3", unit: "分" },
      { value: "47", unit: "秒" },
    ]);
    expect(durationParts(3_600_000, "en")).toEqual([{ value: "1", unit: "h" }]);
    expect(hostOf("https://api.example.com/openai/v1")).toBe("api.example.com");
    expect(hostOf("voltip.example")).toBe("voltip.example");
    expect(hostOf("ftp://x.y/z")).toBe("x.y");
    expect(hostOf("")).toBe("");
  });
});

describe("labels in English", () => {
  const CJK = /[一-鿿]/;

  it("regression: every label helper renders English for the en locale", () => {
    const now = 1_700_000_000;
    const samples: string[] = [
      themeName("warm", "en"),
      themeSubtitle("graphite", "en"),
      platformLabel("other", "en"),
      platformLabel("macos", "en"),
      connectionLabel({ state: "online", via: "direct" }, "en").text,
      connectionLabel({ state: "online", via: "relay" }, "en").text,
      connectionLabel({ state: "connecting" }, "en").text,
      connectionLabel({ state: "offline" }, "en").text,
      connectionLabel({ state: "identity_changed", presented_fingerprint: "x" }, "en").text,
      connectionKindLabel("direct", "en"),
      connectionKindLabel("relay", "en"),
      relayLabel({ state: "reconnecting", attempts: 3, source: "builtin" }, "en").text,
      relayLabel({ state: "disconnected", attempts: 0, source: "none" }, "en").text,
      relayLabel({ state: "connected", attempts: 0, endpoint: "wss://r", source: "user" }, "en")
        .text,
      failureLabel({ kind: "relay", code: "session_full" }, "en"),
      failureLabel({ kind: "relay", code: "weird" }, "en"),
      failureLabel({ kind: "peer_left" }, "en"),
      pairingStateLabel({ state: "awaiting_verification" }, "en").text,
      pairingStateLabel({ state: "failed", reason: { kind: "timeout" } }, "en").text,
      relativeTime(undefined, now, "en"),
      relativeTime(now - 10, now, "en"),
      relativeTime(now - 60, now, "en"),
      relativeTime(now - 7200, now, "en"),
      relativeTime(now - 86_400 * 3, now, "en"),
      viaLabel("clipboard", "en"),
      formatDuration(21_000, "en"),
      formatDuration(227_000, "en"),
      formatDuration(3_600_000, "en"),
      formatDuration(3_720_000, "en"),
      formatDuration(120_000, "en"),
      modelDisplayName("qwen3-asr-0.6b", "均衡", "en"),
      modelDisplayName("paraformer-zh", "轻量 · 中文", "en"),
      modelDisplayName("zipformer-stream-zh-en", "实时预览", "en"),
      modelTierLabel("balanced", "en"),
      modelTierLabel("streaming", "en"),
      dictationPhaseLabel({ phase: "idle" }, now, "en").text,
      dictationPhaseLabel(
        { phase: "listening", started_at: 0, ready: true, locked: false },
        3000,
        "en",
      ).text,
      dictationPhaseLabel({ phase: "processing", stage: "refining", started_at: 0 }, 0, "en").text,
      dictationPhaseLabel(
        {
          phase: "done",
          text: "x",
          raw_text: "x",
          chars: 1,
          via: "clipboard",
          refined: true,
          duration_ms: 1,
          asr_ms: 1,
          mode: "whole_take",
        },
        0,
        "en",
      ).text,
      dictationPhaseLabel({ phase: "cancelled", injected_chars: 0 }, 0, "en").text,
      outcomeLabel({ kind: "inserted", via: "paste" }, "en").text,
      outcomeLabel({ kind: "clipboard", reason: "lost focus" }, "en").text,
      outcomeLabel({ kind: "failed", reason: "uipi" }, "en").text,
      secretStateLabel({ set: true, source: "builtin" }, "en").text,
      secretStateLabel({ set: true, source: "user" }, "en").text,
      secretStateLabel({ set: false, source: "none" }, "en").text,
    ];
    expect(samples.filter((s) => CJK.test(s))).toEqual([]);
    expect(relativeTime(now - 60, now, "en")).toBe("1 minute ago");
    expect(relativeTime(now - 7200, now, "en")).toBe("2 hours ago");
    expect(relayLabel({ state: "reconnecting", attempts: 3, source: "builtin" }, "en").text).toBe(
      "Reconnecting · attempt 3",
    );
    expect(themeName("light")).toBe("明亮");
    expect(themeSubtitle("light")).toBe("白瓷");
    expect(processingStageLabel("inserting", "en")).toBe("Inserting…");
  });
});

describe("live preview text (docs/dictation.md §11)", () => {
  it("regression: joinLiveText mirrors LiveText::preview — no space at CJK boundaries, one space between Latin words", () => {
    expect(joinLiveText(["你好。", "hello world.", "今天"])).toBe("你好。hello world.今天");
    expect(joinLiveText(["Hello.", "How are"])).toBe("Hello. How are");
    expect(joinLiveText(["你好，", "Rust 很好"])).toBe("你好，Rust 很好");
    expect(joinLiveText(["  a ", "", "   "])).toBe("a");
    expect(joinLiveText([])).toBe("");
    // A trailing space on the left already separates: no double space.
    expect(joinLiveText(["Hello ", "world"])).toBe("Hello world");
    expect(liveTextGap("helper", "然后")).toBe("");
    expect(liveTextGap("helper", "then")).toBe(" ");
    expect(liveTextGap("", "then")).toBe("");
    expect(liveTextGap("a ", "b")).toBe("");
    const live = {
      committed: [
        { text: "把 fetchUser 改成 async，", start_ms: 0, end_ms: 1480 },
        { text: "然后加上错误处理。", start_ms: 1480, end_ms: 2900 },
      ],
      current: "再加一次 retry",
    };
    expect(livePreviewText(live)).toBe("把 fetchUser 改成 async，然后加上错误处理。再加一次 retry");
    expect(liveCaptionParts(live)).toEqual({
      committed: "把 fetchUser 改成 async，然后加上错误处理。",
      current: "再加一次 retry",
    });
    expect(liveCaptionParts({ committed: [], current: " 把这段 " })).toEqual({
      committed: "",
      current: "把这段",
    });
  });
});

describe("local model names (docs/dictation.md §10)", () => {
  it("regression: the core's Chinese tier names stay under zh-CN, the dictionary names them by id under en, unknown ids keep the core's name", () => {
    expect(modelDisplayName("qwen3-asr-0.6b", "均衡")).toBe("均衡");
    expect(modelDisplayName("qwen3-asr-0.6b", "均衡", "zh-CN")).toBe("均衡");
    expect(modelDisplayName("qwen3-asr-0.6b", "均衡", "en")).toBe("Balanced");
    expect(modelDisplayName("qwen3-asr-1.7b", "高精度", "en")).toBe("Accurate");
    expect(modelDisplayName("sense-voice-small", "轻量", "en")).toBe("Light");
    expect(modelDisplayName("paraformer-zh", "轻量 · 中文", "en")).toBe("Light · Chinese");
    expect(modelDisplayName("zipformer-stream-zh-en", "实时预览", "en")).toBe("Live preview");
    expect(modelDisplayName("future-model", "未来", "en")).toBe("未来");
    expect(modelDictionaryKey("qwen3-asr-0.6b")).toBe("qwen3-asr-0_6b");
    expect(modelDictionaryKey("qwen3-asr-1.7b")).toBe("qwen3-asr-1_7b");
    expect(modelDictionaryKey("sense-voice-small")).toBe("sense-voice-small");
    expect(modelDictionaryKey("whisper")).toBeUndefined();
    expect(modelDictionaryKey("toString")).toBeUndefined();
    expect(modelDescription("qwen3-asr-0.6b", "core text")).toBe(
      "推荐；Qwen3-ASR 0.6B，30 语种自动识别，自带标点；690 MB",
    );
    expect(modelDescription("qwen3-asr-1.7b", "core text", "en")).toMatch(/^Qwen3-ASR 1\.7B/);
    expect(modelDescription("zipformer-stream-zh-en", "core text", "en")).toMatch(/final text/);
    expect(modelDescription("future-model", "core text", "en")).toBe("core text");
    expect(modelTierLabel("balanced")).toBe("均衡");
    expect(modelTierLabel("accurate")).toBe("高精度");
    expect(modelTierLabel("light", "en")).toBe("Light");
    expect(modelTierLabel("streaming", "zh-CN")).toBe("实时预览");
  });
});

describe("output modes and activation labels (docs/dictation.md §12–§13)", () => {
  it("regression: the three output modes and three activation modes have Chinese names, one-line descriptions, a chip, a footer caption and a hotkey hint; English under en; nothing ASCII-shouty under zh-CN", () => {
    expect(outputModeLabel("whole_take")).toBe("整段输出");
    expect(outputModeLabel("streaming_final")).toBe("边说边识别");
    expect(outputModeLabel("live_inject")).toBe("边说边输入");
    expect(outputModeDescription("whole_take")).toBe("松开快捷键后一次性完成识别、润色和插入。");
    expect(outputModeDescription("live_inject")).toMatch(/不进行润色/);
    expect(outputModeDescription("streaming_final")).toMatch(/只补最后一句/);
    expect(outputModeLabel("live_inject", "en")).toBe("Type as you speak");
    expect(outputModeDescription("streaming_final", "en")).toMatch(/^Sentences settle/);
    expect(activationLabel("hold")).toBe("按住说话");
    expect(activationLabel("toggle")).toBe("按一下开始，再按一下结束");
    expect(activationLabel("hold_or_toggle")).toBe("按住或按一下");
    expect(activationDescription("hold_or_toggle")).toMatch(/短按则锁定/);
    expect(activationChip("hold")).toBe("按住说话");
    expect(activationChip("toggle")).toBe("按一下开始 · 再按结束");
    expect(activationChip("hold_or_toggle")).toBe("按住说话 · 短按锁定");
    expect(activationShortcut("hold")).toBe("按住听写");
    expect(activationShortcut("toggle")).toBe("按一下听写");
    expect(activationShortcut("hold_or_toggle")).toBe("按住或按一下听写");
    // The hint spells the chord with spaces, whatever the mode.
    expect(activationHint("hold", "Ctrl+Alt+Space")).toBe("按住 Ctrl Alt Space 说一句，松开即插入");
    expect(activationHint("toggle", "Ctrl+Alt+Space")).toBe(
      "按一下 Ctrl Alt Space 开始，再按一下结束",
    );
    expect(activationHint("hold_or_toggle", "Ctrl+Shift+D")).toBe(
      "按住 Ctrl Shift D 说话，短按锁定",
    );
    expect(activationHint("hold", "Ctrl+Alt+Space", "en")).toBe(
      "Hold Ctrl Alt Space, say a sentence, release to insert",
    );
    expect(activationLabel("toggle", "en")).toBe("Press to start, again to stop");
    expect(activationShortcut("hold_or_toggle", "en")).toBe("Hold or press to dictate");
    expect(processingStageLabel("finalizing")).toBe("补齐最后一句…");
    expect(processingStageLabel("finalizing", "en")).toBe("Finishing the last sentence…");
    expect(modelTierLabel("auxiliary")).toBe("辅助");
    expect(modelTierLabel("auxiliary", "en")).toBe("Auxiliary");
    // Single-language UI (user 2026-09-25): no Latin words in the Chinese wording.
    const zh = [
      ...(["whole_take", "streaming_final", "live_inject"] as const).flatMap((m) => [
        outputModeLabel(m),
        outputModeDescription(m),
      ]),
      ...(["hold", "toggle", "hold_or_toggle"] as const).flatMap((a) => [
        activationLabel(a),
        activationDescription(a),
        activationChip(a),
        activationShortcut(a),
      ]),
    ];
    for (const text of zh) expect(text).not.toMatch(/[A-Za-z]/);
  });
});

describe("long take labels (section 22)", () => {
  it("the timer shows hours past an hour, the phase line counts the segments and the source has a name", () => {
    expect(formatElapsed(3_599_999)).toBe("59:59");
    expect(formatElapsed(3_600_000)).toBe("1:00:00");
    expect(formatElapsed(7_323_000)).toBe("2:02:03");
    const now = 10_000_000;
    const listening = {
      phase: "listening",
      started_at: now - 3_723_000,
      ready: true,
      locked: false,
    } as const;
    const take = (phase: DictationPhase, done?: number, total?: number) => ({
      phase,
      kind: "dictation" as const,
      segments: done === undefined ? undefined : { done, total: total ?? done },
    });
    expect(takePhaseLabel(take(listening, 12, 13), now).text).toBe(
      "正在录音… 1:02:03 · 已识别 12 段",
    );
    expect(takePhaseLabel(take(listening, 1), now, "en").text).toBe(
      "Recording… 1:02:03 · 1 segment recognised",
    );
    expect(segmentsDoneLabel({ done: 2, total: 3 }, "en")).toBe("2 segments recognised");
    const transcribing = { phase: "processing", stage: "transcribing", started_at: now } as const;
    expect(takePhaseLabel(take(transcribing, 12, 20), now).text).toBe("已识别 12/20 段");
    expect(takePhaseLabel(take(transcribing, 12, 20), now, "en").text).toBe(
      "12 of 20 segments recognised",
    );
    // A short take has no count; after the recognition the stage reads as before.
    expect(takePhaseLabel(take(transcribing), now).text).toBe("识别中…");
    expect(takePhaseLabel(take({ ...transcribing, stage: "refining" }, 20), now).text).toBe(
      "润色中…",
    );
    expect(recordingSourceLabel("microphone")).toBe("麦克风");
    expect(recordingSourceLabel("mixed")).toBe("混合");
    expect(recordingSourceLabel("system", "en")).toBe("Computer audio");
  });
});

describe("voice edit labels (section 19)", () => {
  it("regression: an edit take listens for the instruction and rewrites instead of polishing and reports the replaced text and localises the four refusals; a dictation reads as before", () => {
    const now = 10_000;
    const edit = <P extends DictationPhase>(phase: P) => ({ phase, kind: "edit" as const });
    expect(
      takePhaseLabel(
        edit({ phase: "listening", started_at: now - 3000, ready: true, locked: false }),
        now,
      ).text,
    ).toBe("正在听编辑指令… 00:03");
    expect(
      takePhaseLabel(edit({ phase: "processing", stage: "refining", started_at: now }), now).text,
    ).toBe("改写中…");
    expect(
      takePhaseLabel(edit({ phase: "processing", stage: "transcribing", started_at: now }), now)
        .text,
    ).toBe("识别中…");
    const done: DictationPhase = {
      phase: "done",
      text: "各位同事：会议改至周四上午十点。",
      raw_text: "改得更正式",
      chars: 16,
      via: "paste",
      refined: true,
      duration_ms: 1400,
      asr_ms: 380,
      refine_ms: 900,
      mode: "whole_take",
    };
    expect(takePhaseLabel(edit(done), now)).toEqual({ text: "已替换 16 字 · 粘贴", tone: "ok" });
    expect(takePhaseLabel(edit({ ...done, via: "clipboard" }), now).text).toBe(
      "改写结果已复制 16 字 · 剪贴板",
    );
    expect(takePhaseLabel(edit(done), now, "en").text).toBe("Replaced with 16 chars · Paste");
    // The same phases under `dictation` keep their wording (no rewrite, polished suffix).
    expect(takePhaseLabel({ phase: done, kind: "dictation" }, now)).toEqual(
      dictationPhaseLabel(done, now),
    );
    // Failures: the four refusals, a rewrite that failed, and the selection reason without the
    // `selection: ` prefix of the core's error text.
    const failed = (code: DictationFailureCode, message = "x") =>
      ({ phase: "failed", code, message }) as const;
    expect(takeFailureText(edit(failed("no_selection")))).toBe("没有选中文本");
    expect(takeFailureText(edit(failed("selection_too_long")))).toBe(
      "选中文本过长（上限 2000 字）",
    );
    expect(takeFailureText(edit(failed("edit_unavailable")))).toBe(
      "编辑需要 AI 润色服务，请先配置润色密钥",
    );
    expect(
      takeFailureText(edit(failed("selection", "selection: keystroke: no copy tool on Wayland"))),
    ).toBe("读取选中文本失败：keystroke: no copy tool on Wayland");
    expect(takeFailureText(edit(failed("refine", "refine: 429")))).toBe("改写失败，选中文本未改动");
    expect(dictationFailureText(failed("refine", "refine: 429"))).toBe("润色失败");
    expect(takePhaseLabel(edit(failed("no_selection")), now)).toEqual({
      text: "失败 · 没有选中文本",
      tone: "danger",
    });
    expect(takeFailureText(edit(failed("no_selection")), "en")).toBe("Nothing is selected");
    // Section 19.2: a terminal in front refuses the edit before the copy chord.
    expect(takeFailureText(edit(failed("edit_in_terminal")))).toBe(
      "终端里不支持语音编辑：终端里的选区不能被替换",
    );
    expect(takeFailureText(edit(failed("edit_in_terminal")), "en")).toBe(
      "Voice edit is off in terminals: a terminal's selection cannot be replaced",
    );
    expect(takeFailureText(edit(failed("refine")), "en")).toBe(
      "Rewrite failed; the selection is unchanged",
    );
    expect(takeFailureText(edit(failed("selection", "selection: clipboard: busy")), "en")).toBe(
      "Could not read the selection: clipboard: busy",
    );
  });
});
