// Sample rows for 02 历史记录: converted into real `HistoryEntry` records by `MockBackend` so the
// browser preview has something to show; the real store is `history.json` in the core.
export type DeliveryOutcome = "inserted" | "staged" | "returned" | "unfocused" | "lost";

export interface SampleHistoryRow {
  id: number;
  day: string;
  time: string;
  timestamp: string;
  text: string;
  rawText: string;
  app: string;
  process: string;
  handle: string;
  source: "local" | "cloud";
  engineId: string;
  latencyMs: number | null;
  outcome: DeliveryOutcome;
  starred: boolean;
  suppressed: boolean;
  timing: { asr: number; rules: number; polish: number | null; deliver: number };
  audioSecs: number;
  chars: number;
  segments: number;
  polishSkippedReason?: string;
}

export const historyEntries: SampleHistoryRow[] = [
  {
    id: 128,
    day: "今天 · 9月24日 星期四",
    time: "14:32",
    timestamp: "2026-09-24 14:32:07",
    text: "把这段逻辑抽成一个 helper，然后在 session_assembly 里复用。",
    rawText: "把这段逻辑抽成一个 helper 然后在 session assembly 里复用",
    app: "Visual Studio Code",
    process: "Code.exe",
    handle: "windows-hwnd:0x000A0C42",
    source: "local",
    engineId: "sensevoice-small-int8",
    latencyMs: 774,
    outcome: "inserted",
    starred: true,
    suppressed: false,
    timing: { asr: 412, rules: 1, polish: 210, deliver: 151 },
    audioSecs: 6.8,
    chars: 31,
    segments: 2,
  },
  {
    id: 127,
    day: "今天 · 9月24日 星期四",
    time: "11:08",
    timestamp: "2026-09-24 11:08:12",
    text: "会议改到周四下午三点，地点不变。",
    rawText: "会议改到周四下午三点 地点不变",
    app: "飞书 Lark",
    process: "Feishu.exe",
    handle: "windows-hwnd:0x0007B120",
    source: "cloud",
    engineId: "openai",
    latencyMs: 1912,
    outcome: "inserted",
    starred: false,
    suppressed: false,
    timing: { asr: 1840, rules: 1, polish: null, deliver: 72 },
    audioSecs: 4.2,
    chars: 16,
    segments: 1,
    polishSkippedReason: "未启用",
  },
  {
    id: 126,
    day: "今天 · 9月24日 星期四",
    time: "09:47",
    timestamp: "2026-09-24 09:47:55",
    text: "",
    rawText: "",
    app: "1Password",
    process: "1Password.exe",
    handle: "windows-hwnd:0x0002F004",
    source: "local",
    engineId: "sensevoice-small-int8",
    latencyMs: null,
    outcome: "inserted",
    starred: false,
    suppressed: true,
    timing: { asr: 0, rules: 0, polish: null, deliver: 0 },
    audioSecs: 2.3,
    chars: 0,
    segments: 0,
  },
  {
    id: 125,
    day: "昨天 · 9月23日 星期三",
    time: "18:21",
    timestamp: "2026-09-23 18:21:40",
    text: "TODO: 给配对超时补一条测试。",
    rawText: "todo 给配对超时补一条测试",
    app: "Windows Terminal",
    process: "WindowsTerminal.exe",
    handle: "windows-hwnd:0x000C1100",
    source: "local",
    engineId: "streaming-zipformer-bilingual-zh-en-int8",
    latencyMs: 812,
    outcome: "staged",
    starred: false,
    suppressed: false,
    timing: { asr: 640, rules: 1, polish: null, deliver: 171 },
    audioSecs: 5.1,
    chars: 26,
    segments: 1,
    polishSkippedReason: "LatencyExceeded",
  },
  {
    id: 124,
    day: "昨天 · 9月23日 星期三",
    time: "16:05",
    timestamp: "2026-09-23 16:05:03",
    text: "Please attach the latency report from run 3 to the ticket.",
    rawText: "please attach the latency report from run three to the ticket",
    app: "Outlook",
    process: "OUTLOOK.EXE",
    handle: "windows-hwnd:0x0009A2B0",
    source: "cloud",
    engineId: "groq",
    latencyMs: 1488,
    outcome: "inserted",
    starred: false,
    suppressed: false,
    timing: { asr: 1402, rules: 1, polish: null, deliver: 85 },
    audioSecs: 4.9,
    chars: 58,
    segments: 1,
    polishSkippedReason: "未启用",
  },
  {
    id: 123,
    day: "昨天 · 9月23日 星期三",
    time: "10:12",
    timestamp: "2026-09-23 10:12:41",
    text: "这个函数的返回值类型改成 Option<String>，空字符串不要再出现。",
    rawText: "这个函数的返回值类型改成 option string 空字符串不要再出现",
    app: "Visual Studio Code",
    process: "Code.exe",
    handle: "windows-hwnd:0x000A0C42",
    source: "local",
    engineId: "sensevoice-small-int8",
    latencyMs: 1203,
    outcome: "inserted",
    starred: true,
    suppressed: false,
    timing: { asr: 631, rules: 1, polish: 412, deliver: 160 },
    audioSecs: 10.0,
    chars: 26,
    segments: 3,
  },
  {
    id: 122,
    day: "9月22日 星期二",
    time: "15:40",
    timestamp: "2026-09-22 15:40:19",
    text: "麻烦把发票抬头改成公司全称。",
    rawText: "麻烦把发票抬头改成公司全称",
    app: "微信 WeChat",
    process: "WeChat.exe",
    handle: "windows-hwnd:0x0005D3C8",
    source: "local",
    engineId: "sensevoice-small-int8",
    latencyMs: 690,
    outcome: "lost",
    starred: false,
    suppressed: false,
    timing: { asr: 512, rules: 1, polish: null, deliver: 177 },
    audioSecs: 3.4,
    chars: 13,
    segments: 1,
    polishSkippedReason: "未启用",
  },
];
