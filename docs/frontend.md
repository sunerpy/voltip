# 前端实现规格（React 19 + TypeScript 5.9 + Tailwind 4 + Vite 7 + Vitest 3）

> 读者：实现 `packages/ui`、`packages/shared`、`apps/desktop/src`、`apps/mobile/src` 的工程师/Agent。Rust 侧（`crates/`、`apps/*/src-tauri`）不在本规格范围内。

## 0. 视觉基线

- 令牌与组件的唯一真相是代码：`packages/ui/src/tokens.css`（四主题色板、字体、圆角、阴影）与 `packages/ui/src/components/*`。页面文案以 `packages/shared/src/labels.ts` 与各页面源码为准。
- 当前 UI 的截图在 `docs/acceptance/screens/desktop/`（浏览器 MockBackend 模式，1440 宽，明亮 / 暗黑，中 / 英）与 `docs/acceptance/screens/tauri/`（Linux Xvfb 下的真实 Tauri 窗口）。

## 1. 令牌（CSS 变量）

四套主题 `light | dark | warm | graphite`，角色：`canvas surface inset inset2 border borderStrong fg fgMuted fgSubtle primary primaryFg accent accentFg accentSoft accentText accentTextHover thumb ok okSoft okText danger dangerSoft warning warningSoft info infoSoft nav navActive ledOff ledOn ledPeak pillBg pillBorder pillFg pillMuted wave desktop desktopField keycapBg keycapBorder scrim diffAdd diffDel mark` + 三组阴影 `shadowPop shadowPill shadowWin`。

- 实现为 `packages/ui/src/tokens.css`：`:root[data-theme="light"] { --canvas: #F2F3F6; … }` 四段；Tailwind 4 用 `@theme inline { --color-canvas: var(--canvas); … }` 暴露为 `bg-canvas text-fg border-border` 等工具类。
- 蓝色系与强调色（2026-09-29，用户选定）：明亮、暗黑、石墨三套主题的默认强调色是 Codex 桌面应用的 `#339cff`，暖纸保留赭色；`info` 一律是 Codex 的蓝。设置 › 外观的「强调色」是本机选项（`LocalAppearance.accent`，写进 `<html data-accent>`，其他窗口经 `storage` 事件跟随）：默认、蓝、绿、黄、粉、橙、紫、墨色，取 ChatGPT / Codex 的色板，浅色主题用浅档、深色主题用深档。每个主题 × 强调色的 `accent / accent-fg / accent-soft / accent-text / accent-text-hover / led-peak` 事先算好写进 `tokens.css`：`accent-fg` 取对比度更高的黑或白，`accent-text` 与悬停色在表面、画布、嵌入色和 `accent-soft` 上都 ≥ 4.5:1，焦点框用 `accent-text`（`theme.test.ts` 逐一检查）。开关照 Codex：32 × 19，开为强调色、关为淡墨色，白色滑块（`--thumb`、`shadow-thumb`）；文字链接用 `accent-text`，无下划线，悬停换 `accent-text-hover`；设置项标签 14 px 中等字重，说明 12 px。浅色主题里 `#339cff`、黄、绿、粉等填充色与白底的对比度低于 3:1，这是 Codex / ChatGPT 的原样设计：开关的状态同时由滑块位置表示，文字和焦点框都用加深后的颜色。
- 字体：UI `Instrument Sans`，等宽 `JetBrains Mono`，中文 `Noto Sans SC`（`font-ui / font-mono`；CJK 自动回退到 PingFang SC / Microsoft YaHei UI）。**三族字体随包自带**（`@fontsource-variable/*` 可变字体，`packages/ui/src/tokens.css` 顶部 `@import`），不从 Google Fonts 拉取（离线 / 受限网络下远程字体会回退成系统字体）；CSP 不放行任何远程 font / style 源（`apps/desktop/src/offline-assets.test.ts`）。圆角：4 / 6 / 10 / 14 / 20 / 999。卡片无阴影、1 px hairline；阴影只用于弹出层与胶囊。
- **绿色只做 8 px 状态点**（`ok`），强调色 `accent` 用于链接 / 波形 / 电平 / 热力图 / 进度 / 选中态。

## 2. 组件（`packages/ui`，每个都有 Vitest 测试）

`Card Panel(eyebrow) Eyebrow Badge(tone: neutral|ok|accent|danger|warn|info|ink，ok = 中性底 + 绿点) Chip Button(primary|outline|ghost|danger, disabled) IconButton Toggle Segmented Input(icon, keys, focus) Select Keycap/Keycaps LedMeter Waveform(2px 条 2px 间距圆头镜像，近端 accent 尾迹 subtle 55%) Heatmap Sparkline Progress Table(列类型 text|mono|chip|badge|toggle|progress|lamp|actions|keys|link|radio|two) LampText Readout Pill(胶囊八态) Toast Dialog(叠加时各有名称，Esc 只关最上面的) Menu(菜单按钮：单选行 / 命令行 / 分组，单选行可带右侧次要文字 `detail` 与 `disabled`（键盘跳过），`onOpen` 在每次打开时调用，键盘可用，每行不折行) EmptyState CommandPalette ThemeTile Sidebar Toolbar(读数：中文标签 + 可读值 + 状态点) StatusRow`

## 3. IPC 契约（`packages/shared`）

Tauri command 名与参数（`invoke(name, args)`），返回值 JSON：

| command | args | returns |
|---|---|---|
| `core_state` | — | `UiState` |
| `pairing_start` | — | `null` |
| `pairing_join_code` | `{ code: string }` | `null` |
| `pairing_join_ticket` | `{ uri: string }` | `null` |
| `pairing_join_nearby` | `{ fingerprint: string }` | `null`（`docs/pairing.md`「局域网发现」：加入 `UiState.nearby` 里这台设备等待的配对；没看到它或它不在配对为 `error` 事件） |
| `pairing_confirm` / `pairing_reject` / `pairing_cancel` / `pairing_reset` | — | `null` |
| `device_forget` | `{ publicKey: hex }` | `null` |
| `device_rename` | `{ name: string }` | `null` |
| `send_text` | `{ publicKey: hex, body: string }` | `null` |
| `settings_set_relay` | `{ url: string \| null, enabled: boolean }` | `null` |
| `settings_set_lan_discovery` | `{ enabled: boolean }` | `null`（持久化 `Settings.lan_discovery` 并回发 `settings`，随即开始或停止公布与浏览） |
| `settings_set_pairing_always_on` | `{ enabled: boolean }` | `null`（`docs/pairing.md`「常开配对」：持久化 `Settings.pairing_always_on` 并回发 `settings`；打开即开一个会话，关掉即关闭还没人加入的会话；手机上为 `error` 事件） |
| `settings_set_theme` | `{ theme: ThemeId, followSystem: boolean }` | `null` |
| `devices_refresh` | — | `null` |
| `dictation_start` / `dictation_stop` / `dictation_cancel` | — | `null`（`docs/dictation.md` §5） |
| `settings_set_engines` | `{ engines: EngineSettings }`（全量） | `null` |
| `provider_key_set` | `{ provider: ProviderId, kind: "asr" \| "llm", value: string \| null }` | `null`（值永不回传；`docs/dictation.md` §3） |
| `provider_probe` | `{ provider, kind, baseUrl?, key? }` | `null`（结果为 `provider_probe` 事件：模型列表或不含主机名的失败原因） |
| `engines_quota_reset` | `{ kind: "asr" \| "llm" }` | `null`（「重新检查」：清除该服务额度用完的记录，回发 `engines`；`docs/dictation.md` §3.5） |
| `provider_console_open` | `{ provider }` | `null`（桌面在浏览器里打开该服务商的密钥页） |
| `history_delete` / `history_star` | `{ id }` / `{ id, starred }` | `null` |
| `history_clear` | — | `null` |
| `settings_set_locale` | `{ locale: "system" \| "zh-cn" \| "en" }` | `null`（核心持久化并回发 `settings`，所有窗口与手机同步换语言，§7） |
| `settings_set_auto_update` | `{ enabled: boolean }` | `null` |
| `hotkey_edge` | `{ pressed: boolean, atMs?: number, source?: "hotkey" \| "cli" \| "ui", purpose?: "dictation" \| "edit", chorded?: boolean }` | `null`（一个热键边沿进核心的激活状态机，`docs/dictation.md` §13；界面只传 `pressed`，壳层补 now / `ui` / `dictation`；`purpose: "edit"` 是语音编辑键，§19） |
| `settings_set_edit_hotkey` | `{ hotkey: string \| null }` | `null`（§19：语音编辑热键；与听写键同样校验、不得与听写键相同，`null` 关闭；持久化 `Settings.edit_hotkey` 并回发 `settings`，拒绝为 `error` 事件） |
| `settings_set_solo_key` | `{ key: SoloKey \| null }` | `null`（§13.1：单键触发，`right_ctrl` … `mouse_forward`，`null` 关闭；持久化 `Settings.solo_key` 并回发 `settings`，桌面壳随即安装或卸下输入钩子，结果在 `hotkey` 事件的 `solo_registered` / `solo_error`） |
| `phone_text_send` | `{ publicKey: string, body: string, source: "typed" \| "clipboard" }` | `null`（§20.6：手机把文字发给在线的可信电脑插入光标处；`sent_texts` 事件跟随每个回音；桌面壳返回 `PHONE_TEXT_UNAVAILABLE`） |
| `sent_texts_clear` | — | `null`（手机清空已发送列表） |
| `phone_clipboard_read` | — | 查询：`{ text: string \| null }`，手机剪贴板里的文字（Android 插件；其他构建报错） |
| `phone_share_text` | `{ text }` | `null`（手机经系统分享面板交给其他应用，`docs/dictation.md` §20.7；桌面报错） |
| `settings_set_activation` | `{ activation: "hold" \| "toggle" \| "hold_or_toggle", holdThresholdMs: number, extraRecordingMs: number }` | `null`（三值一起下发；> 5000 ms 核心以 `error` 事件拒绝，设置不变；成功则回发 `settings`） |
| `update_check` / `update_install` | — | `null`（进度以 `update` 事件回报，§7） |
| `update_status` | — | `UpdateStatus`（查询，`Backend.updateStatus()`） |
| `dictionary_add` | `{ entry: { term, heard_as, enabled }, historyId?: string \| null }` | `null`（`docs/dictation.md` §16.4：草稿本身不合法时 reject 核心原文；与其他词条冲突为 `error` 事件、列表不变；成功回发 `dictionary`） |
| `dictionary_update` / `dictionary_remove` | `{ id, entry }` / `{ id }` | `null` |
| `dictionary_reorder` / `rules_reorder` | `{ ids: string[] }`（现有 id 的一个排列） | `null` |
| `rules_add` / `rules_update` / `rules_remove` | `{ rule: { name, kind, pattern, replacement, case_sensitive, enabled } }` / `{ id, rule }` / `{ id }` | `null`（正则在 bridge 里编译，失败即 reject；成功回发 `rules`） |
| `rules_import` | `{ toml: string, mode: "replace" \| "merge" }` | `null`（整份解析与校验，失败 reject 并带行号 / 第 N 条） |
| `rules_export` | — | `string`（查询，`Backend.rulesExport()`：TOML 文本） |
| `vocabulary_preview` | `{ text: string, draft: { id?: string \| null, rule } \| null }` | `{ corrected, output, corrections: [{ id, count }], rules: [{ id, count }], error? }`（查询，`Backend.vocabularyPreview(text, draft?)`：核心同一份代码，不跑 LLM） |
| `scenes_add` | `{ scene: SceneDraft }`（`{ name, enabled, match: { apps, title_contains }, overrides }`，未设的覆盖项省略或为 `null`） | `null`（`docs/dictation.md` §18.6：草稿本身不合法时 reject 核心原文；重名 / 超过 50 个为 `error` 事件、列表不变；成功回发 `scenes`） |
| `scenes_update` / `scenes_remove` | `{ id, scene }` / `{ id }` | `null`（未知 id 为 `error` 事件） |
| `scenes_reorder` | `{ ids: string[] }`（现有 id 的一个排列） | `null` |
| `settings_set_context_sharing` | `{ appName: boolean, windowTitle: boolean }` | `null`（持久化 `Settings.context_sharing` 并回发 `settings`） |
| `recent_apps` | — | `AppRef[]`（查询，`Backend.recentApps()`：历史里最近出现的应用，最新在前，按 id 去重，≤ 20；读历史库） |
| `history_query` | `{ sinceMs?, starred, failed, query, offset, limit }`（`limit` 1–200） | `HistoryPage` `{ entries, matching, total }`（查询，`Backend.historyQuery()`，docs/dictation.md §4.4） |
| `history_entry` | `{ id }` | `HistoryEntry \| null`（查询，`Backend.historyEntry()`） |
| `history_stats` | `{ boundaries: number[] }`（递增的本地零点，2–43 个） | `HistoryStats` `{ buckets, total }`（查询，`Backend.historyStats()`，docs/dictation.md §4.5） |
| `history_hits` | — | `HistoryHits` `{ dictionary, rules }`（查询，`Backend.historyHits()`：全部历史的触发次数） |

事件：`listen("voltip://event", (e) => UiEvent)`；听写相关事件为 `dictation`（`DictationStatus` 展平）、`history`（`{ entries }`）、`engines`（`EngineStatus` 展平），更新器事件为 `update`（`UpdateStatus` 展平，`type` 与 `state` 两个判别字段），词典与规则为 `dictionary`（`{ entries }`）与 `rules`（`{ rules }`，每次变更全量），`UiState` 有同名字段（`schema.ts` 用 `.default()` 兼容缺字段的旧核心）。

```ts
type ThemeId = "light" | "dark" | "warm" | "graphite";
type Platform = "windows" | "macos" | "linux" | "android" | "ios" | "other";
type ConnectionState = "disconnected" | "connecting" | "authenticating" | "connected" | "reconnecting" | "closed";
type OutputMode = "whole_take" | "streaming_final" | "live_inject"; // docs/dictation.md §12
type Activation = "hold" | "toggle" | "hold_or_toggle";               // docs/dictation.md §13
type ProviderId = "builtin" | "local" | "openai" | "groq" | "siliconflow" | "aliyun" | "deepseek" | "ollama" | "custom"; // docs/dictation.md §3
interface EngineSettings { asr_provider: ProviderId /* default "builtin" */; llm_provider: ProviderId /* default "builtin" */; refine_enabled: boolean; providers: Partial<Record<ProviderId, { asr_model?: string; asr_url?: string; llm_model?: string; llm_url?: string }>>; local_model?: string | null; local_device: "auto" | "cpu" | "gpu"; local_gpu?: string | null; local_threads?: number | null /* §10.6 */; language?: string; live_preview: boolean /* default true, §11 */; output_mode: OutputMode /* default "whole_take" */; vad_trim: boolean /* default false */; chinese_script: "simplified" | "traditional" | "as_is" /* default "simplified", §17 */; inject: "paste" | "clipboard_only" }
interface Settings { schema: 1; theme: ThemeId; follow_system_theme: boolean; relay_url?: string; relay_enabled: boolean; hotkey: string; engines: EngineSettings; locale: "system" | "zh-cn" | "en" /* default "system" */; auto_update: boolean /* default false */; activation: Activation /* default "hold" */; hold_threshold_ms: number /* default 300 */; extra_recording_ms: number /* default 0 */; edit_hotkey: string | null /* §19，default "Ctrl+Alt+E"，null = 关闭 */; solo_key: SoloKey | null /* §13.1，default null = 关闭 */; lan_discovery: boolean /* docs/pairing.md，default true */; pairing_always_on: boolean /* docs/pairing.md「常开配对」，default false */ }
interface DeviceIdentityPublic { device_id: string; name: string; platform: Platform; public_key: string /* 64 hex */; fingerprint: string /* "A7:C4:19:8E · 3D:F2:61:09" */ }
interface RelayStatus { endpoint?: string; state: ConnectionState; attempts: number }
type PairingState = { state: "idle" | "creating_session" | "waiting_for_peer" | "key_exchange" | "awaiting_verification" | "trusted" | "expired" | "rejected" }
  | { state: "failed"; reason: { kind: "timeout" | "replay" | "handshake" | "protocol" | "cancelled" | "peer_left" | "identity_changed" } | { kind: "relay"; code: string } };
interface SafetyCode { words: [string, string, string, string]; fingerprint: string }
interface DeviceInfo { device_id: string; name: string; platform: Platform }
interface Snapshot { state: PairingState; session_id?: string; code?: string /* "483 921" */; ticket_uri?: string; expires_at?: number; remaining_secs?: number; safety_code?: SafetyCode; peer?: DeviceInfo; local_confirmed: boolean; peer_confirmed: boolean }
interface TrustedDevice { device_id: string; name: string; platform: Platform; public_key: string; fingerprint: string; trusted_at: number; last_seen?: number; last_connection?: "direct" | "relay" }
type DeviceConnection = { state: "offline" } | { state: "connecting" } | { state: "online"; via: "direct" | "relay" } | { state: "identity_changed"; presented_fingerprint: string };
interface DeviceView { device: TrustedDevice; connection: DeviceConnection }
interface Segment { text: string; start_ms: number; end_ms: number }
interface LiveText { committed: Segment[]; current: string; degraded?: string; injected: number /* default 0：live_inject 已粘贴的句数（§12） */ }
type DictationPhase = { phase: "idle" } | { phase: "listening"; started_at: number; ready: boolean /* default false: 设备尚未送出样本 */; live?: LiveText; locked: boolean /* default false：hold_or_toggle 短按锁定（§13），总在 wire 上 */ } | { phase: "processing"; stage: "transcribing" | "finalizing" | "refining" | "inserting"; started_at: number; preview?: string }
  | { phase: "done"; text: string; raw_text: string; chars: number; via: "paste" | "clipboard"; refined: boolean; duration_ms: number; asr_ms: number; refine_ms?: number; refine_error?: string; mode: OutputMode /* default "whole_take" */; segments?: Segment[]; live_error?: string }
  | { phase: "failed"; message: string; code?: "no_speech" | "audio" | "asr" | "refine" | "inject" | "no_selection" | "selection_too_long" | "selection" | "edit_unavailable" | "edit_in_terminal" | "unknown"; text?: string } | { phase: "cancelled"; injected_chars: number /* default 0 */ };
interface DictationStatus { session: number; phase: DictationPhase; context?: TakeContext /* §18：探针有答案后出现，回到 idle 清除 */; kind: "dictation" | "edit" /* §19，当前 / 上一次 take，default "dictation" */ }
interface HistoryEntry { id: string; at_ms: number; raw_text: string; text: string; refined: boolean; asr_model: string; refine_model?: string; duration_ms: number; asr_ms: number; refine_ms?: number; outcome: { kind: "inserted"; via: "paste" | "clipboard" } | { kind: "clipboard"; reason: string } | { kind: "failed"; reason: string }; starred: boolean; mode: OutputMode /* default "whole_take" */; segments?: Segment[]; live_error?: string; kind: "dictation" | "edit" /* §19，default "dictation" */; edit?: { instruction: string; selection: string } /* 只有语音编辑有 */ }
interface SecretState { set: boolean; source: "builtin" | "user" | "none" }
interface EngineStatus { asr_provider: ProviderId; asr_ready: boolean; asr_issue?: EngineIssue; asr_model: string; asr_host: string /* 用户填写的主机；内置服务与本机为 "" */; local_model?: string | null; local_ready: boolean; live_preview_ready: boolean /* live_preview && 流式模型已安装，与服务商无关 */; effective_output_mode: OutputMode /* 本次 dictation_start 真正会走的模式 */; language?: string; refine_enabled: boolean; llm_provider?: ProviderId; refine_ready: boolean; refine_issue?: EngineIssue; refine_model: string; refine_host: string; inject: "paste" | "clipboard_only"; providers: ProviderStatus[] /* 每家的模型、预设、密钥状态、是否在用 */ }
interface ModelState { id: string; name: string /* 核心的中文档位名；en 下前端按 id 查字典 */; engine: "transcribe_cpp" | "sense_voice" | "paraformer" | "zipformer_streaming" | "silero_vad"; tier: "balanced" | "accurate" | "light" | "streaming" | "auxiliary"; capabilities: ("offline" | "streaming" | "vad")[]; languages: string[]; size_bytes: number; description: string; recommended: boolean; active: boolean; state: { kind: "not_installed" } | { kind: "downloading"; received: number; total: number; file: string } | { kind: "verifying" } | { kind: "installed"; path: string; installed_at: number } | { kind: "failed"; message: string } }
type UpdateStatus = { state: "idle" } | { state: "checking" } | { state: "up_to_date"; version: string; checked_at: number } | { state: "available"; version: string; current: string; notes?: string; date?: string }
  | { state: "downloading"; version: string; received: number; total?: number } | { state: "ready"; version: string } | { state: "installing"; version: string } | { state: "failed"; message: string } | { state: "disabled" };
interface DictionaryEntry { id: string; term: string; heard_as: string[]; enabled: boolean; source: { kind: "manual" } | { kind: "history"; history_id: string }; created_at_ms: number; updated_at_ms: number } // docs/dictation.md §16
interface ReplacementRule { id: string; name: string; kind: "literal" | "regex"; pattern: string; replacement: string; case_sensitive: boolean; enabled: boolean; created_at_ms: number; updated_at_ms: number }
interface SceneOverrides { refine_enabled?: boolean; refine_preset?: PresetId /* 内置预设名或自定义预设 UUID，docs/dictation.md §21 */; output_mode?: OutputMode; language?: string /* "auto" = 不带语言提示 */; chinese_script?: "simplified" | "traditional" | "as_is"; prompt?: string /* ≤ 500 */ } // 缺省 = 跟随全局，docs/dictation.md §18
interface Scene { id: string; name: string; enabled: boolean; match: { apps: string[] /* 规范化 id */; title_contains: string[] }; overrides: SceneOverrides; created_at_ms: number; updated_at_ms: number }
interface AppRef { id: string; name: string }
interface TakeContext { app: AppRef; scene?: { id: string; name: string } } // DictationStatus.context?、HistoryEntry.app? / scene?
interface ContextSharing { app_name: boolean /* default true */; window_title: boolean /* default false */ } // Settings.context_sharing
interface NearbyDevice { fingerprint: string /* 16 位十六进制 */; name: string; platform: Platform; pairing: boolean; trusted: boolean } // docs/pairing.md「局域网发现」
interface UiState { identity: DeviceIdentityPublic | null; settings: Settings; secret_backend: string; relay: RelayStatus; pairing: Snapshot; devices: DeviceView[]; hotkey: HotkeyStatus; dictation: DictationStatus; history_recent: HistoryEntry[] /* 最新 20 条，每行可带 vocabulary?: { corrections, rules }（命中的 id 与次数） */; history_total: number /* 全部条数，其余用 history_query 读 */; engines: EngineStatus; update: UpdateStatus; models: ModelState[]; dictionary: DictionaryEntry[]; rules: ReplacementRule[]; nearby: NearbyDevice[] /* 局域网浏览看到的设备，default [] */ }
type UiEvent = { type: "state" } & UiState | { type: "identity" } & DeviceIdentityPublic | { type: "settings" } & Settings | { type: "relay" } & RelayStatus
  | { type: "pairing" } & Snapshot | { type: "devices"; 0?: never } /* payload is the array: see below */ | { type: "trusted" } & TrustedDevice
  | { type: "identity_changed"; previous: TrustedDevice; presented_fingerprint: string } | { type: "message"; from: string; body: string } | { type: "error"; message: string }
  | { type: "hotkey" } & HotkeyStatus | { type: "dictation" } & DictationStatus | { type: "history"; recent: HistoryEntry[]; total: number } | { type: "engines" } & EngineStatus | { type: "update" } & UpdateStatus
  | { type: "models"; models: ModelState[] } | { type: "dictionary"; entries: DictionaryEntry[] } | { type: "rules"; rules: ReplacementRule[] }
  | { type: "scenes"; scenes: Scene[] } /* UiState.scenes，整份替换 */ | { type: "nearby"; devices: NearbyDevice[] } /* UiState.nearby，整份替换 */;
```

注意 serde 内部标签的细节：`UiEvent::Devices(Vec<DeviceView>)` 与 `UiEvent::State(UiState)` 等 newtype 变体在 JSON 中是 `{"type":"devices", ...}`？**不是**——serde 对 tagged newtype 变体的规则：结构体内容被展平（`state`、`identity`、`settings`、`relay`、`pairing`、`trusted` 都是结构体，会展平成 `{type, ...fields}`），而 `devices` 的内容是数组，serde 无法展平，Rust 侧已改为 `{"type":"devices","devices":[...]}`。用 zod 按上述形状校验，校验失败记录并忽略该事件。

`Backend` 接口：`getState() invoke() on(event) audioDevices() meter()`，以及查询 `updateStatus() vocabularyPreview() rulesExport() recentApps() historyQuery() historyEntry() historyStats() historyHits() permissionsStatus() permissionsRequest() injectPreflight() pasteText()`；提供 `TauriBackend`（`@tauri-apps/api` 的 `invoke/listen`）与 `MockBackend`（内存模拟：完整走完配对状态机与倒计时、设备上下线、消息回显，以及确定性的听写流水线——`dictation_start` → listening（`ready: false`，150 ms 后 `ready: true` 并重取 `started_at`；`live_preview_ready` 时每 400 ms 一条流式部分结果：`current` 增长三步 → 端点提交一条 `committed` → 第二句增长，脚本 `MOCK_LIVE_SCRIPT`；`simulateLiveDegraded(reason)` 置 `live.degraded` 并停发部分结果），`dictation_stop` → 400 ms transcribing（带 `preview` = committed + current）→ 300 ms refining → done + 一条 `HistoryEntry`，2.5 s 后回 idle；**输出模式（§12）**：`engines.effective_output_mode` = `output_mode`，流式模式在流式模型未安装 / `live_preview` 关时回落 `whole_take`；模式在 `dictation_start` 时定下——`streaming_final` 走 finalizing（120 ms，`MOCK_FINALIZE_MS`）→ refining? → done（`mode`、`segments` = committed + tail、`raw_text` = 预览文本、`asr_ms` = 120），`live_inject` 每个 committed 句立刻 `live.injected += 1`，松开后 finalizing → inserting → done 且不润色，取消时 `cancelled.injected_chars` = 已粘贴字数；流式取无文本或 `degraded` 时回落整段并记 `live_error`（`MOCK_EMPTY_STREAM_ERROR`）；**激活（§13）**：`hotkey_edge` 按 `settings.activation` 驱动——`hold` 按下开始 / 松开结束，`toggle` 按下切换 / 松开忽略，`hold_or_toggle` 松开时按住 ≥ `hold_threshold_ms` 结束、否则 `listening.locked = true` 并等下一次按下；`source: "cli"` 按下切换、松开取消；去抖、宽限与处理中 pending 不模拟。`settings_set_activation` 持久化三值并回发 `settings`，> 5000 ms 发 `error` 且不改；`extra_recording_ms > 0` 时 stop 延后关麦，窗口内第二次 stop 立即关、cancel 连录音一起丢弃；目录五条产品档位（`qwen3-asr-0.6b` 均衡 / `qwen3-asr-1.7b` 高精度 / `sense-voice-small` 轻量 / `paraformer-zh` 轻量 · 中文 / `zipformer-stream-zh-en` 实时预览，尺寸同真实文件），`live_preview_ready = live_preview && 流式模型已安装`；`settings_set_engines` / `provider_key_set` / `provider_probe` / `history_*` 真实作用于内存状态；**场景（§18）**：`scenes_*` / `settings_set_context_sharing` 按核心同样的上限与拒绝原文（`packages/shared/src/scenes.ts`），`recentApps()` 取内存历史；假探针 `foregroundApp` 选项 / `setForegroundApp(app | null)`，每次 `dictation_start` 按核心规则清洗答复并匹配场景，覆盖输出方式（流式不可用时整段 + `live_error`）与润色开关，`context` 进状态、`app` / `scene` 进历史；**语音编辑（§19）**：`setSelection(text | null)` 模拟前台选区（默认没有），`hotkey_edge { purpose: "edit" }` 按同一激活语义开始编辑的一次（另一种用途的一次进行中时边沿丢弃）——没有润色密钥即 `failed{edit_unavailable}` 不开麦；假探针 `foregroundApp` 的 id 属于身份平台的终端表（`isTerminalApp`，与 `voltip_platform::foreground` 同一份表，macOS 为空）即 `failed{edit_in_terminal}`，不复制、不写历史；60 ms（`MOCK_COPY_MS`）后复制：无选区 / 只有空白 `no_selection`、超过 2000 字 `selection_too_long`；松开后 transcribing → refining → inserting → done（`MOCK_EDIT_INSTRUCTION` → `MOCK_EDIT_TEXT`，永远整段、规则不跑），写一条 `kind: edit` 的历史（带指令、原选区与探针的 `app`，不匹配场景）；`settings_set_edit_hotkey` 与 `settings_set_hotkey` 互相拒绝对方的组合键，`hotkey` 状态带 `edit_registered`；内置服务在界面上只显示为「内置服务」，没有主机名；**粘贴到上一个窗口**（docs/dictation.md §4.1）：`pasteText` 照壳层与核心的规则拒绝（空白或超长 `invalid`、录音或处理中 `busy`），其余记进 `pastes` 并答 `pasteOutcome` 选项（默认 `pasted`，`setPasteOutcome` 可改），不写历史；手机角色写进 `phoneClipboard` 并答 `copied { clipboard_only }`，手机听写的结果同样写进 `phoneClipboard`，`phone_share_text` 记进 `shared`，手机的历史从空开始，只放它自己识别的条目，docs/dictation.md §20.7）。

`MockBackend` 与只在开发时用的页面（胶囊规格页 `OverlaySheet`）从 `@voltip/shared/mock` 与动态 `import()` 加载，只在 `import.meta.env.DEV` 且不在 Tauri 里时启用；发行包的前端里没有它们，`scripts/check-web-bundle.sh` 检查构建产物。

## 4. 页面与路由

桌面（`apps/desktop/src`，窗口 1152×720 起，最小 960×600，可缩放）：侧栏 224 + **标题栏 40** + 主区。

- **标题栏只做标题栏，但把事实放回标题栏**：主窗口无原生装饰（`tauri.conf.json` `decorations: false, shadow: true`；macOS 由 `tauri.macos.conf.json` 保留交通灯 `titleBarStyle: "Overlay"`，并按 RFC 7396 完整重述窗口几何），`packages/ui` `TitleBar` 与侧栏品牌行构成一条连续的 40 px `data-tauri-drag-region="deep"`。标题栏从左到右：页标题（拖拽区）→ **读数菜单**（`readout` 插槽，`role="group"`、`data-testid="title-bar-readout"`：mono 11 px `text-fg-subtle`，两个菜单按钮以 ` · ` 分隔——语音模型 + 令牌状态灯（本地模型带「本机」标签）、麦克风短名——窄时省略号截断，窗口窄于 `md`（768 px）时 `hidden md:flex` 整体隐藏；按钮之间的空白仍是拖拽区；点击就地切换，见下条「切换菜单」；`readouts` prop 保留，给只有文字读数的场合）→ `Ctrl K` 搜索**图标按钮**（tooltip「搜索或输入命令 · Ctrl K」，没有输入框）→ **AI 润色开关**（`wand` 图标 + 文字「AI 润色」/「AI Polish」+ 状态灯，`aria-label="润色 · 开/关"`，tooltip 说明开关方向；点击真实写 `settings_set_engines { refine_enabled }`，`aria-pressed` 跟随 `state.engines.refine_enabled`）、预设菜单与**润色模型菜单**（`refine_model` 的短名，`data-testid="polish-model"`）→ Windows / Linux 最右 46×40 的最小化 · 最大化/还原 · 关闭（`apps/desktop/src/app/window.ts` 包一层 `getCurrentWindow()`，浏览器预览无 Tauri 时不画按钮；平台由 UA 判定 `app/platform.ts`，identity 只作兜底）。**标题栏下方没有第二行页头**（原 `PageHeader` 组件已删除）；标题栏上没有示例数据 chip。不自写双击最大化；触控 / 笔用 `startDragging()` 兜底。已接受的降级：Windows 11 Snap Layouts 悬停面板不可用（WebView2 上游限制）。
- **设置是模态对话框**：`/settings/:section` 仍是路由（深链、测试），但渲染为 `SettingsDialog`（`min(960px, 100vw-48px) × min(660px, 100vh-48px)`，左 200 px 分组导航 + 右侧流式内容），压在路由 `background`（上一个非设置页面，默认首页）之上；Esc / 遮罩 / 关闭按钮回到 background；标题栏读数保持 background 页面的。
- **设置 › 听写**（2026-09-29）：插入方式（粘贴到光标处 / 仅复制到剪贴板，`settings_set_engines { inject }`）只在这里设置；首页识别引擎卡的插入方式读数链接到这里。
- **切换菜单**（`apps/desktop/src/features/switchers/`，2026-09-30 用户要求，2026-10-01 确认方案 A：预设与模型是两个下拉）：标题栏与首页共用三个菜单，行与选中后的设置由纯函数 `switchers.ts` 生成（行 id 由 `choiceId` / `parseChoice` 往返）。`SpeechModelMenu`：「内置服务」组在前，然后是每个已设置、能运行的云端服务商（未设置密钥或缺少地址的服务商不列出，2026-10-01 用户决定），各列其模型；「本地模型」组列出已安装的识别模型（档位名 + 产品名，`modelFamilyName`）；末行「管理语音模型…」进 `/speech`。`PolishModelMenu`：按提供 LLM 的可用服务商分组列模型，末行「管理 AI 模型…」进 `/ai`。选中写 `settings_set_engines`（`engineSettingsFor`：内置服务只切换服务商；其他服务商经 `applyProviderDraft` 写入模型并保留已保存的地址；本地模型经 `activateLocalModel`）。`MicrophoneMenu`：挂载时和每次打开时重读 `audio_devices`（刚插上的 USB 麦克风也在），「系统默认（{名称}）」在前，再列各设备短名；所选设备未连接时仍列出，带「未连接」、选中且不可选；选中写 `settings_set_microphone`；标题栏的菜单另有「录音来源」组（麦克风 / 电脑声音 / 混合，电脑声音不可用的平台不出现），末行「录音来源设置…」打开 设置 › 录音来源。首页：就绪行的语音模型 chip（`home-engine-chip`）、麦克风卡的设备名（`home-mic-device`，原「选择设备」按钮去掉；只录电脑声音时是纯文字）、识别引擎卡的模型名（`home-engine-model`）与「润色模型」（`home-refine-model`）都是这三个菜单的按钮。
- **引擎页是服务商卡片**：「语音识别」「文本润色」「识别设置」三组。前两组各列一排服务商卡片（`ProviderCard`，按目录顺序，在用的那张带环、默认展开并标「使用中」），展开后选模型（预设 → 测试连接列出的 → 任意 id）、填接口地址（仅自定义接口）与密钥（`password` 字段，永远空着打开，只经 `provider_key_set` 写入；已保存时只显示「已保存」），「测试连接」= `provider_probe`，「使用」写 `settings_set_engines`；内置服务卡只写「内置服务的地址和密钥在构建时写进应用，这里不显示」。「本机」卡片里是真实的模型库（`LocalModels`）：只列识别模型（`capabilities` 含 `offline`），按产品档位排序 均衡 → 高精度 → 轻量 → 轻量 · 中文（`offlineModelsByTier`，同档位保持核心顺序，`listitem` 带 `data-tier`），卡片标题在中文下用核心的 `name`、英文下按 id 查字典（`modelDisplayName`），描述同理（`modelDescription`），下载 / 取消 / 重试 / 删除 / 使用此模型全部是真实核心命令。第三节「实时预览」（`LivePreview`，docs/dictation.md §11）：开关写 `settings_set_engines { live_preview }`（整块下发），状态灯行读 `state.engines.live_preview_ready`（已就绪 / 模型未下载 / 已关闭，`data-state`），下方是流式模型 `zipformer-stream-zh-en` 的同款卡片——只有下载 / 取消 / 重试 / 删除，**没有**「使用此模型」（`modelAction` → `installed`：它不是识别模型，永不 `active`）。第四节「输出方式」（`OutputMode`，docs/dictation.md §12，2026-09-26）：三张 `OptionCard`（`role="listbox"`，`aria-label="输出方式"`）——整段输出 / 流式定稿 / 实时注入，选中写 `settings_set_engines { output_mode }`；状态灯行与「当前生效」徽标读 `state.engines.effective_output_mode`（`data-mode` = 所选，`data-effective` = 实际），流式模型未下载时两张流式卡带「模型未下载」徽标、所选流式卡下方出「流式模型未下载，当前按整段输出运行」（`output-mode-fallback`）；`output_mode = live_inject` 且润色开着时 LLM 润色卡内出「实时注入下不润色」（`refine-live-inject`）。第五节「静音裁剪」（`VadTrim`）：`Toggle` 写 `settings_set_engines { vad_trim }`，仅本机识别时可用，云端下禁用并注明「仅本地识别可用；云端识别不裁剪」（`data-state` = on / off / cloud）。「本机」卡片最后是「运行设备」（`LocalCompute`，§10.6）：自动 / CPU / GPU（没有可用 GPU 时禁用 GPU）、多块 GPU 时选哪块、推理线程。服务商列表之后是「额度用完后改用其他模型」（`packages/ui` `FallbackSection`，docs/dictation.md §3.5，识别与润色各一份，手机同用）：开关与列表写 `settings_set_engines { asr_fallback | llm_fallback }`，每行的状态读 `state.engines.asr_fallback | llm_fallback`（`fallbackRows`：使用中 / 可用 / 额度已用完 · 何时再试 / 服务商问题 / 跳过），「重新检查」= `engines_quota_reset`；候补顶替时服务商区块标题为「当前：服务商 · 模型（候补）」，隐私说明加上额度用完后会收到数据的服务商。
- **自适应**：页面根 `mx-auto w-full max-w-[1440px]`（首页 1600），网格只用 `minmax(0,1fr)` / `auto-fill`，没有固定像素面板宽；每页一条 `regression: … is fluid` 测试禁止 `w-[NNNpx]` 面板。
- 路由 `/`（首页）`/history` `/dictionary` `/rules` `/engines` `/devices`（配对面板 = 二维码 + 6 位码 + 倒计时 + 设备表）`/settings/:section`（对话框）`/onboarding`（权限 → 热键 → 引擎 → 试说，四步）；`Ctrl K` 命令菜单；悬浮胶囊作为独立路由 `/overlay?state=` 渲染单个胶囊（独立小窗，`state=blank` 什么都不画，**`state=live` 跟随 `state.dictation`**：idle 不画；listening 波形来自 `audio_meter_start` 的实时电平 + 已用时长——`ready: false`（设备尚未送出样本）时计时停在 `00:00` 并显示「等待麦克风」提示，`ready: true` 后从重取的 `started_at` 起计；有 `live` 时胶囊长高到两行（`h-14`，窗口 64 px 减 8 px 顶部内边距），波形上方一行两色实时字幕：`committed` 正常色 + `current` 弱化色（`text-pill-muted`），中日韩边界不加空格、拉丁边界加空格（`liveTextGap`），单行、只保留末尾 40 个字符（`clipLiveCaption`，前置省略号，`data-clipped`），旁边一枚「预览」小 chip；`live.degraded` 胶囊不额外显示任何东西（首页引擎卡出一行「实时预览已中断 · 最终文本不受影响」）；processing 有 `preview` 时用弱化色的预览文本（`pill-preview`，同样只留末尾 40 字）替代「转写中…」/「润色中…」直到最终结果；inserted 显示字数与 `via`，failed 显示原因并在有 `text` 时提供真实「复制文本」，cancelled 用 danger 胶囊），**§12 / §13（2026-09-26）**：`listening.locked` 时锁标（`pill-lock`，`aria-label`「已锁定 · 再按一次结束」）替代状态灯、字幕与计时照旧；`live.injected > 0` 时已粘贴的 committed 句以更淡样式（`pill-live-injected`，`title`「已打进前台应用」）排在字幕最前、裁剪时最先被裁；`processing.stage = finalizing` 时模式 tag 显示「补齐最后一句…」而 `preview` 继续显示；`cancelled.injected_chars > 0` 时文案为「已取消，之前打进去的 N 字保留」）；`/overlay` 不带参数是胶囊规格页（八态总览 + toast / 兜底卡 / 失败原因表；`locked` 样例胶囊的「结束收音」是真实 `dictation_stop`），只在开发构建里有，发行包里这个地址显示「找不到页面」。没有 `/bridge` 路由（Bridge & MCP 不在产品范围内）；首页对应位置是由核心真实数据驱动的「手机麦克风 · 设备」卡。
- 客户端禁用浏览器右键菜单（`apps/desktop/src/app/context-menu.ts`，仅 Tauri 内生效，可编辑控件与 `data-allow-context-menu` 除外）。

所有页面的数据都来自 `Backend`（`state.dictation` / `state.history_recent` 与历史查询 / `state.dictionary` / `state.rules` / `state.engines` / `state.settings` …）；`packages/shared/src/fixtures/` 只剩 IPC 契约夹具与测试数据。

移动端（`apps/mobile/src`，同一套组件，竖屏 390×844 起）：`Welcome → This Device（名称、指纹、平台）→ Pair Device（[扫码] 或 输入 6 位码 `_ _ _   _ _ _`）→ Verify Device（对端名称/平台 + Safety Code 四词 + 指纹 + [Confirm & Trust]/[Reject]）→ Devices（Device / Platform / Online·Offline / Last Seen / Trusted / Connection Type）`。扫码用 `@tauri-apps/plugin-barcode-scanner`（在 Mock 下用文本输入代替）。Welcome 与 Devices 的顶部都是「用手机说话」（`PhoneMic`）：有在线的已配对电脑时录音发给电脑（`phone_take_*`），没有时在手机上识别（`dictation_*`，结果进手机剪贴板），一次录音保持开始时的去向；下面的「最近结果」（`RecentResults`）读 `state.history_recent`，列出手机识别的结果和发送到电脑的听写，点一行打开记录详情，也可以直接复制（`pasteText`）或分享（`phone_share_text`），docs/dictation.md §20.7。

手机的样式（2026-10-03 用户要求：与桌面一致、去掉老式安卓控件）：根字号 16 px（`apps/mobile/src/index.css` 的 `--ui-font-size`；桌面 13 px 起，由设置 › 外观调），所以共享组件的尺寸按标称值显示，`h-11` 即 44 px 的最小触控目标、`p-4` 即 16 px 页边距，文字大小两端都是像素。`App` 用 `PresentationProvider value="touch"` 包住整个应用：`Select` 不再用原生 `<select>`（Android WebView 会弹出系统老式的单选对话框），而是同样外观的触发按钮加页面内的选项列表（与 `Menu` 同一套样式，行高 44 px，下方放不下时翻到上方，点外面、Esc 和系统返回关闭，返回先关列表再关对话框，共用 `components/layers.ts` 的层栈）；Button、IconButton、Toggle、Segmented、Menu 行与输入框去掉点按高亮，带按下状态，点按区域至少 44 × 44 px。没有提供者时为 `native`，桌面渲染与之前逐字节一致。页面沿用桌面的视觉语言：小号眉标、细线卡片、等宽读数、墨色主按钮、强调色文字链接；设置列表一种行样式（14 px 中等字重标签、12 px 说明、右侧控件或箭头）。`useNow` 是 Unix 秒，按天分组、统计与时间筛选的 hook 要毫秒（`useNow() * 1000`），桌面用 `useTickingNow`。

## 5. 质量门

- `pnpm -r run lint`（**oxlint** 1.85 · 插件 typescript / react / react-hooks / import / unicorn / promise / vitest，`--type-aware`（oxlint-tsgolint）；禁 `any`、`as unknown as`、非空断言）+ `tsc --noEmit`；`pnpm -r run format:check`（**oxfmt** 0.70，沿用仓库 `.oxfmtrc.json`：printWidth 100 · 双引号 · trailingComma all）。
- Vitest + jsdom + Testing Library；每个包 `vitest.config.ts` 设 `coverage.thresholds` 行/分支/函数/语句 ≥ 90，provider `v8`，排除 `main.tsx`、`vite-env.d.ts`、`*.d.ts`、fixtures。
- 功能测试：用 `MockBackend` 驱动完整配对流程（桌面发起 → 手机扫码/输码 → 双端确认 → 设备列表 Online），断言 UI 文案与状态；回归测试：过期倒计时归零显示「已过期 · 重新生成」、拒绝、身份变化告警横幅、Relay 断连重连读数、主题切换写回 `data-theme`。
- 无硬编码 Relay 地址；无 `TODO` 骨架；每个页面至少一个渲染测试 + 一个交互测试。

## 6. 范围与诚实状态

听写流水线在 Rust 核心真实运行（`docs/dictation.md`）：录音、识别、润色、剪贴板 + 粘贴注入、`history.sqlite3`、服务商与密钥。**界面上没有「尚未接入」「示例」一类的占位**：一个功能要么真的能用，要么不出现（`packages/shared/src/labels.test.ts` 的回归测试检查两份字典里都没有这类文案）。不做定时器模拟的进度、不伪造接受结果、不为没有发生的事弹成功 toast。

### 6.1 统一措辞（`packages/shared/src/labels.ts`）

| 常量 / 函数 | 文案 | 用途 |
|---|---|---|
| `dictationPhaseLabel` / `outcomeLabel` / `secretStateLabel` / `viaLabel` / `formatElapsed` / `formatMs` / `formatSeconds` / `hostOf` | 待命 · 正在听… 00:03 · 转写中… · 已插入 42 字 · 粘贴 · 已润色 · 已内置 / 已设置 / 未设置 … | 首页、历史页、引擎页、胶囊共用的听写文案 |
| `dictationFailureText(phase, locale?)` | `no_speech` → 没有听到声音 / No speech detected；`audio` / `asr` / `refine` / `inject` 同理；语音编辑的 `no_selection` / `selection_too_long` / `edit_unavailable` / `edit_in_terminal` 同理，`selection` 带核心的原因（去掉 `selection: ` 前缀）；`unknown` 或无 `code` → 核心的 `message` 原文 |
| `takePhaseLabel(status, now, locale?)` / `takeFailureText(status, locale?)` | 按 `DictationStatus.kind`：听写同 `dictationPhaseLabel` / `dictationFailureText`；语音编辑为 正在听编辑指令… 00:03 · 改写中… · 已替换 16 字 · 粘贴 / 改写结果已复制 16 字 · 剪贴板，`refine` 失败为「改写失败，选中文本未改动」 | 首页阶段行、胶囊（§19） | 首页阶段行、胶囊 error 态、试说步骤的失败原因（§7） |

所有 `labels.ts` 助手的文案来自 `packages/shared/src/i18n/` 字典，最后一个参数是 `locale`（默认 `zh-CN`，无 provider 的调用方与既有测试保持中文）；组件与页面通过 `useI18n()` 取 `locale` 传入。

剪贴板复制统一走 `copyWithToast`（`apps/desktop/src/app/shell-context.tsx`）：只有 `navigator.clipboard.writeText` 成功才弹「已复制…」，失败弹 danger toast「无法访问剪贴板 · 未复制」。

### 6.2 各页面状态

| 路由 | 数据来源 | UI 如何表达 |
|---|---|---|
| `/` 首页 | **全部真实**：就绪行读 `state.engines`（令牌是否就位）+ 麦克风；识别引擎卡在 `live_preview_ready` 时带「实时预览」chip（`home-live-preview`），listening 且 `live.degraded` 时出一行「实时预览已中断 · 最终文本不受影响」（`home-live-degraded`，`title` 为核心原因）；本地模式的模型名中文用核心 `name`、英文按 id 查字典；组合键 keycaps 读 `state.settings.hotkey`，就绪行的 chip（按住说话 / 按一下开始 · 再按结束 / 按住说话 · 短按锁定）、页脚说明（按住听写 / 按一下听写 / 按住或按一下听写）与空态提示（`activationHint`）随 `state.settings.activation` 变化（docs/dictation.md §13）；「开始听写」= `dictation_start`，听写中变「停止」= `dictation_stop`，另有「取消」= `dictation_cancel`；就绪行下方一行实时状态（`正在听… 00:03` / `转写中…` / `已插入 42 字 · 粘贴 · 已润色` / `失败 · …`）；识别引擎卡读 `state.engines`，LLM 润色开关写 `settings_set_engines`；最近的结果读 `state.history_recent`，至少 6 行，窗口更高时按页面下方的空白增加行数，直到页面底部不再留空，最多为 `history_recent` 的全部 30 条（`features/history/recent-rows.ts` `useRowsThatFit`，2026-10-01 用户要求）；今日会话 / 统计条 / 近 6 周热力图来自 `history_stats`（`features/history/stats.ts`、`useHomeStats.ts`，docs/dictation.md §4.5）：今日面板写转录字数、修正字数、说话时长、节省时间，四张卡写节省时间和字数，「依据」弹层说明 × 1.9 的来源；麦克风卡为 Rust 电平；手机麦克风卡读 `state.devices` / `state.relay` | 无 chip、无徽标；令牌缺失时「还不能开始听写」+ 按钮禁用并带原因 `识别引擎缺少访问令牌 · 在「引擎」页设置`；历史为空时表格位置是空态 + 热键提示 |
| `/history` | **真实** `history_query` 分页读库（`history.sqlite3`，上限 2 万，docs/dictation.md §4.4） | 横幅写保留条数（`settings.history.keep`）；每页 100 条，滚动到底加载下一页（另有「加载更多」），列表下方写「已显示 n / m 条」；筛选与搜索交给核心（搜索停止输入 200 ms 后发出）；筛选 全部 / 今天 / 本周 / 本月 / 已收藏 / 未插入；搜索文本 / 原文 / 模型 / 应用 / 场景；带上下文的条目在日志行显示应用名 + 场景徽标（`history-context`），详情显示「应用：名称 · id」与「场景」（没有匹配为「未匹配场景」，docs/dictation.md §18.6）；星标 = `history_star`，删除 = 确认 + `history_delete`（选中相邻项），全部清空 = 确认 + `history_clear`；复制走 `copyWithToast`；原文 / 润色后 / 对比 diff 来自 `raw_text` 与 `text`；`mode` 为流式定稿 / 实时注入的条目在日志行与详情页头带 `Badge`（`history-mode` / `history-detail-mode`），整段不显示；`live_error` 有值时详情里出「流式回退：…」（`history-live-error`）；空态提示随 `settings.activation`；详情列出本次触发的词典纠正与规则（`history-vocabulary`，名称取当前列表，已删除的写「已删除的词条 / 规则」）；「加入词典」打开对话框（曾听成用当前选区预填，正确写法手填）提交 `dictionary_add { historyId }`，核心拒绝时对话框保留并显示原因；没有「重新插入」「保留时长」等无后端控件 |
| `/dictionary` | **真实** `state.dictionary`（`dictionary.json`，上限 500，按匹配顺序） | 新建（表上方一行）/ 行内编辑（Enter 保存、Esc 取消）/ 启用开关 / 上移下移 / 删除（行内确认）= `dictionary_*`；输入时本地查空、超长、重复正确写法、写法过多，其余由核心判定（reject 原文留在编辑器，冲突为 `error` toast）；「命中」列与「历史记录里的命中」chip 汇总 `history[*].vocabulary.corrections`；「试一试」= `vocabulary_preview`（防抖，列表变化重问），「用最近一次听写」取 `history[0].raw_text`；说明卡写明词表去向（云端识别与 AI 润色；本地模型不接收）；`Ctrl N` 新建；没有 CSV、权重、复制为规则、示例页脚 |
| `/rules` | **真实** `state.rules`（`rules.json`，上限 200，按执行顺序） | 一张表（序号 / 名称 / 类型 / 匹配 + `Aa` / 替换或「（删除）」/ 命中 / 启用）；编辑器本地查空名称、重名、空匹配，再用 `vocabulary_preview("", draft)` 让核心编译检查（失败原文进状态行、禁用保存），`Ctrl S` 保存、`Esc` 取消、`Ctrl N` 新建；上移下移 = `rules_reorder`，删除 = 确认 + `rules_remove`；「试运行」（`Ctrl ↵`）= `vocabulary_preview`，可带正在编辑的草稿（「名称（未保存）」）；「导入 TOML」文本对话框（合并 / 替换，核心整份校验，拒绝原文含行号）、「导出 TOML」对话框显示 `rules_export` 文本并可复制；没有规则集切换、预算、作用域、逐阶段视图、示例页脚 |
| `/speech` 语音模型页（旧链接 `/engines`、`/settings/engine`、`/settings/speech` 都进这里）；`/ai` AI 模型页（旧链接 `/settings/refine`、`/settings/ai`） | **全部真实**（`state.engines` / `state.models` / `state.hardware` / `settings_set_engines` / `provider_key_set` / `provider_probe` / `model_download` / `model_cancel` / `model_remove`） | 服务商卡片（见 §4）；本地模型库按档位排序（均衡 → 高精度 → 轻量 → 轻量 · 中文）；「实时预览」块：开关写 `live_preview`，状态 已就绪 / 模型未下载 / 已关闭，流式模型卡无「使用此模型」；「输出方式」三卡写 `output_mode`，状态行 / 「当前生效」徽标读 `effective_output_mode`，模型缺失时说明回落；「静音裁剪」开关写 `vad_trim`，云端禁用并说明 |
| `/devices` 手机麦克风 | **真实**（配对、设备表、中继开关、忘记设备、手机正在进行的一次听写） | 配对面板底部是连接说明与中继状态；右侧「手机麦克风」面板在手机按住说话时显示真实电平与阶段（PCM 16 kHz 单声道、手机名、链路），「经过这条通道的」列出文字、手机音频、识别结果三类加密内容；「限制」写明手机不是输入法、中继只转发密文 |
| `/settings/appearance` | **Backend 真实数据**（`settings_set_theme`）+ 本地外观 | 真实 |
| `/settings/hotkey` | **真实**：`settings.hotkey` + `settings.edit_hotkey` + `settings.solo_key` + `state.hotkey`（含 `edit_registered` / `edit_error` / `solo_registered` / `solo_error` / `solo_pressed` / `capabilities.solo_keys`）+ `settings.activation / hold_threshold_ms / extra_recording_ms` | 录制走 `useChordRecorder`；「编辑选中文本」一行是第二个录制器（`settings_set_edit_hotkey`，与听写录制器互斥，「关闭 / 启用」，没有润色密钥时注明需要 AI 润色服务，§19）；「激活方式」三张 `OptionCard`（按住说话 / 按一下开始，再按一下结束 / 按住或按一下，`role="listbox"`）写 `settings_set_activation`（三值一起）；「短按判定阈值」只在 按住或按一下 下显示（50–5000 ms，步进 50），「松开后继续录音」始终显示（0–5000 ms，0 = 立即停止）；两个数字框本地起草、失焦 / 回车时吸附到 50 ms 网格并夹到范围内再下发（`clampActivationMs`）；卡片下一行 mono 提示随模式与组合键（`activation-hint`） |
| `/feedback` 反馈页 | **真实**（`feedback_diagnostics` / `feedback_submit`） | 左边是类型、描述、可选联系方式与发送；右边列出随附的诊断；发送后清空并提示；没有反馈地址的构建改为在 GitHub 上反馈 |
| `/settings/scene` 场景 | **真实**：`state.scenes`（`scenes.json`，上限 50，按匹配顺序）+ `settings.context_sharing` + `recent_apps` | 顶部「发送给 AI 润色的上下文」两个 `Toggle`（应用名称默认开 / 窗口标题默认关，`settings_set_context_sharing`）；场景卡片（`role="article"`，序号 / 名称 / 启用开关 / 上移下移 = `scenes_reorder` / 编辑 / 删除确认 = `scenes_remove`；应用 id chip、标题关键词或「任何窗口」、覆盖摘要或「全部跟随全局设置」）；`SceneEditor` 对话框：名称、应用 chip（输入 id 回车规范化，或点「最近的应用」）、标题关键词 chip、五个覆盖 `Select` 以「跟随全局」开头、补充要求带字数；缺项在第一次保存后提示、重名与超限输入时提示，核心拒绝原文留在对话框里，`Ctrl S` 保存、Esc 只关编辑器；标题栏读数「场景 N 个 · 启用 M · 上下文 应用名称 开 · 窗口标题 关」 |
| `/settings/general` 通用 | **真实**：`settings.locale`（`settings_set_locale`）、`settings.auto_update`（`settings_set_auto_update`）、`state.update`（`update_check` / `update_install`） | 语言 `Segmented`（跟随系统 / 简体中文 / English）+ 自动更新 `Toggle` + 更新状态行与唯一允许的动作按钮（§7） |
| `/settings/about` 关于 | **真实**：版本来自 `state.app_version`，许可 AGPL-3.0-or-later（`APP_LICENSE`），模型来源，更新行读 `state.update` | 只读读数行 + 同一条更新状态行（紧凑）+「源代码」「反馈问题」（`project_link_open`，壳层用浏览器打开仓库地址） |
| `/settings/privacy` 隐私 | **真实**：`state.engines` 与上下文开关推出的「现在离开这台电脑的内容」、`settings.history`（`settings_set_history`：开关与保留条数）、`state.secret_backend` | 读数行 + 历史开关与保留条数（改小立即裁剪）+「清空历史」（确认） |
| `/overlay` | 规格页：波形为确定性伪造序列；**`/overlay?state=live`：真实**（见 §4：等待麦克风 / 两色实时字幕 + 「预览」chip / processing 的 `preview` / 锁标 / 已粘贴句更淡 / 「补齐最后一句…」/ 取消保留字数 / 模式标签后的场景名 `pill-scene`） | 规格页「复制文本」真实；`locked` 样例胶囊的「结束收音」= 真实 `dictation_stop`（锁定态由 `hold_or_toggle` 短按产生，§13）；示例 toast 的「撤销」不做事 |
| `Ctrl K` 命令菜单 | **真实** | 「开始听写 / 停止听写」= `dictation_start` / `dictation_stop`（处理中禁用并说明）；「复制上一条结果」复制 `state.history_recent[0].text`；「删除全部历史…」确认后 `history_clear`；历史为空时两项禁用并写「历史记录为空」 |
| `/onboarding` | **真实** | 第 1 步每秒轮询 `permissions_status`（`docs/dictation.md` §15.1），必需权限被拒时不能继续，Linux 直接通过；第 2 步读核心的组合键与 shell 的注册结果；**第 3 步**三选一（内置服务 / 本机模型，可当场下载推荐模型 / 其他服务商，填模型与密钥，密钥经 `provider_key_set`），「保存并继续」写 `settings_set_engines`；**第 4 步**「试说一句」= `dictation_start`，变「停止」= `dictation_stop`，电平表在录音期间跟随真实电平，`done` 的 `text` 显示在框里并说明文本也已按 `via` 送到光标处，「完成设置」始终可用；没有跳过链接 |

标题栏读数菜单（`features/switchers/SwitcherMenus.tsx`，按钮文字来自 `shell/page-meta.ts` `engineReadout` 与 `audio_devices`）：语音模型一律来自 `state.engines`（`Qwen3-ASR-1.7B`，tooltip 主机 + 令牌状态；核心未报告时显示「等待核心…」），麦克风是所选设备（未选时为系统默认）的短名，设备列表未返回前用原生电平表枚举到的设备；`pageMeta().readouts`（历史条数、热词、中继 …）仍按页计算，供测试使用；设置对话框页头的读数来自 `settingsReadouts`（热键、外观、场景）；热键行来自 `state.settings.hotkey`；侧栏历史计数为 `state.history_total`。

### 6.3 回归测试

`apps/desktop` / `packages/*` 中以 `regression:` 为前缀的测试锁定上述状态。关键回归用例：`regression: the title bar shows the compact engine and microphone readout inline and no second header row` 与 `regression: the polish toggle carries the AI 润色 text label`（`packages/ui/src/components/TitleBar.test.tsx` 与 `apps/desktop/src/shell/Shell.test.tsx`）、`regression: every locale dictionary has the same key set`（`packages/shared/src/i18n/i18n.test.ts`）、`regression: switching the locale to English re-renders the shell, home, history, engines, settings and onboarding without CJK text` 与 `regression: dictation failure codes are localized`（`apps/desktop/src/app/locale.test.tsx`、`packages/shared/src/i18n/i18n.test.ts`）、`regression: changing the hotkey updates the home readiness row and footer immediately`（`Home.test.tsx`）、`regression: 开始听写 starts a real session and shows the phase; 停止 finishes with the inserted text`（`Home.test.tsx` 与 `packages/shared/src/mock-backend.test.ts`）、`regression: no page shows the phase-2 chip or the sample-data notice for dictation, history, engines or hotkey`（`Shell.test.tsx`）；另有 `regression: another provider needs its key, which goes to provider_key_set and is never echoed`（`Engines.test.tsx`）、`regression: provider_key_set flips only the set/source flags and never stores or echoes the value`（`mock-backend.test.ts`）、`regression: the trial step runs a real dictation and shows the inserted text`（`Onboarding.test.tsx`）、`regression: the live pill paints nothing while idle, then listening with the real meter, the processing stage, and the inserted count` 与 `regression: with live preview ready the pill shows the two-tone caption and the 预览 chip while listening, the preview in place of 转写中… while processing, and nothing extra when degraded`（`Overlay.test.tsx`）、`regression: listening draws a two-tone live caption above the waveform with a 预览 chip; the capsule grows to two rows`（`packages/ui/src/components/Pill.test.tsx`）、`regression: the library lists recognition models by tier (均衡 → 高精度 → 轻量 → 轻量 · 中文) and the streaming model only in the 实时预览 block, without 使用此模型` 与 `regression: the 实时预览 toggle writes live_preview through settings_set_engines and the state line follows live_preview_ready`（`EnginesPane.test.tsx`）、`regression: with live preview ready the mock emits ready → throttled partials → a committed sentence → processing with the preview (docs/dictation.md §11)`（`mock-backend.test.ts`）、`regression: the model tiers, the 实时预览 block, the home chip and the live pill render in English under en`（`locale.test.tsx`）、`regression: the palette's dictation, copy-last and clear-history entries are real core actions`（`Shell.test.tsx`）、`regression: no string in either dictionary says a feature is not wired yet`（`labels.test.ts`），以及 `features/history/stats.test.ts` 对今日 / 本周 / 本月 / 热力图的纯函数断言。2026-09-26（§12 / §13 接入）：`regression: section 12 and 13 dictation fields survive parsing`（`ipc-contract.test.ts`）、`regression: a core that always serialises the section 12 and 13 fields round-trips them`（`schema.test.ts`）、`regression: settings_set_activation persists all three values`、`regression: hold_or_toggle stops on a release held past the threshold`、`regression: extra_recording_ms keeps the microphone open after the stop`、`regression: streaming_final goes finalizing then refining then done`、`regression: live_inject counts pasted sentences`（`mock-backend.test.ts`）、`regression: the activation cards write settings_set_activation with all three values`（`Settings.test.tsx`）、`regression: the output mode cards`（`EnginesPane.test.tsx`）、`regression: a hold_or_toggle short press locks the take and the pill shows the lock mark`（`Overlay.test.tsx`）、`regression: streaming rows carry a mode badge`（`History.test.tsx`）、`regression: the readiness chip and the empty-state hint follow settings`（`Home.test.tsx`）、`regression: a locked take shows the lock mark in place of the lamp`（`Pill.test.tsx`）。2026-09-26（§16 词典与规则、§17 中文字形接入）：`regression: the dictionary page renders the core entries with counts and history hits and no sample data or deferred controls`、`regression: add edit enable reorder and delete go through the dictionary commands and the list follows the core`、`regression: the test panel corrects through the core preview`（`Dictionary.test.tsx`）、`regression: the rules page renders the core rules in execution order`、`regression: the editor has the core check the draft`、`regression: the dry run is the core preview of the dictionary and every enabled rule`、`regression: TOML export shows the core text and import merges or replaces refusing a bad file with its position`（`Rules.test.tsx`）、`regression: the detail names the corrections and rules that fired and add to dictionary sends dictionary_add with the row id`（`History.test.tsx`）、`regression: the sidebar counts come from the core dictionary and rules`（`Shell.test.tsx`）、`regression: the Chinese script choice writes chinese_script through settings_set_engines`（`EnginesPane.test.tsx`）、`regression: dictionary commands behave like the core`、`regression: a mock take corrects the transcript with the dictionary and runs the rules last`（`mock-backend.test.ts`）、`regression: the dictionary and rules events and the history hits and chinese_script survive parsing from the Rust fixtures`（`ipc-contract.test.ts`）；英文扫描（`locale.test.tsx`）现在也覆盖 `/dictionary` 与 `/rules`。2026-09-26（§18 场景与上下文接入）：`regression: the 场景 group lists the scenes in matching order`、`regression: the context switches start at app name on and window title off and write settings_set_context_sharing`、`regression: 新建场景 builds a scene from a typed id and a recent app`、`regression: the editor refuses a duplicate name and a long prompt before sending`、`regression: 新建场景 is disabled at the 50-scene cap`（`settings/scenes/ScenesPane.test.tsx`）、`regression: the matched scene is a tag right after the mode tag`（`Pill.test.tsx`）、`regression: the live pill names the scene the take runs under`（`Overlay.test.tsx`）、`regression: a take with a context shows its app and scene in the row and the detail`（`History.test.tsx`）、`regression: scene commands behave like the core`、`regression: the fake probe picks the scene of a take`、`regression: the phone refuses every scene command and the query`（`mock-backend.test.ts`）、`regression: the scenes events, the take context, the history app and scene and the context switches survive parsing from the Rust fixtures`（`ipc-contract.test.ts`）；中英文两套扫描（`locale.test.tsx`）也覆盖 `/settings/scene`。

### 6.4 单语言界面

规则：**同一时刻界面只说一种语言，由当前 locale 决定**。眉题（`eyebrow` 工具类：JetBrains Mono 11 px、0.08 em 字距、`uppercase`）保留视觉角色，但文字来自字典——中文下是「录音来源」「识别引擎」「今日会话」「会话日志」「已配对设备」「实时电平」「状态同步」「局限 · 不是什么」「耗时 · 松开热键到插入」「历史记录里的命中」「边沿监测 · 按一下试试」「悬浮胶囊 · 八态」「胶囊解剖」……，英文下是 `Recording source` / `Session log` / `Edge monitor · give it a press` 等（CSS 负责大写）。眉题与其下方标题在中文里会重复的地方（设置对话框的 `SETTINGS` 与各分组 `GENERAL` / `APPEARANCE` / `TRIGGER` / `ENGINE` / `PRIVACY` / `ABOUT`，`LOCAL MODELS · 本地模型`，手机端「本机」卡）直接去掉眉题或只留一份，不再叠加；设置导航的分组标签不再带 `general` / `hotkey` 这类英文 key 后缀（id 仍用于 `aria` 与测试）。字典里「中文短语 · 英文注释」一类写法（`语音识别 · ASR`、`访问令牌 · Bearer`、`LOCAL · 不传输任何数据`）改为只保留中文；保留原样的只有产品名、模型名、按键名与数据（API 密钥、CSV、TOML、模型 id、主机名、`Ctrl` 键帽、平台名 Windows / macOS / Android、悬浮胶囊规格页的状态 id 前缀 `armed · …`）。ASR、LLM 和 `*.json` 文件名不再上屏，用词见 §8。原始来源值（`builtin` / `user` / `none`、`relay.state`）一律经 `secretStateLabel` / `relayLabel` / 字典渲染，不再裸露。锁定用例：`apps/desktop/src/app/locale.test.tsx@regression: under zh-CN no page shows an ASCII-caps eyebrow or an English gloss after a Chinese title`（渲染全部路由，收集 `[class~="eyebrow"]` 与设置导航 `.mono`，断言无 `/^[A-Z][A-Z0-9 &·/'-]{2,}$/`、无「中文 · 英文词」且不在专有名词白名单）与 `apps/mobile/src/App.test.tsx@regression: under zh-CN no phone screen shows an ASCII-caps eyebrow or an English detail label or a raw relay state`；英文方向由既有的无 CJK 扫描覆盖（已扩展到眉题）。

## 7. 国际化与自动更新

### 7.1 语言

- **来源**：`state.settings.locale`（`"system" | "zh-cn" | "en"`，核心持久化）。`resolveLocale(setting, navigator.language)`（`packages/shared/src/i18n/runtime.ts`）：`system` 在浏览器 / OS 语言以 `zh` 开头时解析为 `zh-CN`，否则 `en`；主窗口、悬浮胶囊窗口和手机端读同一字段，改一处全部跟随。`<html lang>` 跟随解析结果（`zh-CN` / `en-US`）。
- **字典**：`packages/shared/src/i18n/zh-CN.ts`（键集的唯一真相，`Messages = typeof zhCN`）与 `en.ts`（`const en: Messages`，缺键 / 多键都是类型错误）。嵌套键，叶子是字符串（`{name}` 插值）或 `{ one, other }` 复数对（`{n} 条` vs `{n} entry` / `{n} entries`，中文永远取 `other`）。`t(key, params?)` 的 `key` 是 `MessageKey`（`LeafPaths<Messages>`），拼错编译不过；运行时缺键返回键名本身，肉眼可见而不是空白。
- **接入**：`packages/ui` 导出 `I18nProvider` / `useI18n()`（`{ locale, tag, t, messages }`）/ `useT()` / `useLocale()`；没有 provider 时一律 `zh-CN`，所以组件默认文案不变、既有测试不变。桌面 `App.tsx` 与手机 `App.tsx` 在 `BackendProvider` 内挂 `LocaleProvider`（`systemLanguage` prop 供测试注入 OS 语言；`renderApp` 默认 `zh-CN`，因为 jsdom 自报 `en-US`）。
- **覆盖范围**：`packages/ui` 内含文案的组件（TitleBar 搜索 tooltip 与窗口控制、Sidebar、Dialog `Esc 取消`、CommandPalette、Pill / LiveCaption、Banner、各可视化组件的 aria 默认名、ThemeTile 主题名）；`apps/desktop` 全部页面、shell、对话框、命令菜单、页脚快捷键、引导、胶囊文案、toast、`page-meta` 标题与读数标签、`labels.ts` 助手；`apps/mobile` 全部屏幕。设置分组、权限表、触发方式、后端矩阵备注等原 fixture 文案迁入字典，fixture 只剩 id / 状态 / 深链。日期：`Intl.DateTimeFormat(locale)`（`features/history/stats.ts` `dayLabel` / `todayLabel`；今天 / 昨天 → Today / Yesterday）。
- **不翻译的东西**：模型 id、主机名、命令名、`settings.json` 键、keycaps、`voltip://` 链接、示例词条 / 规则 / 历史文本等用户或核心产生的数据（历史文本节点带 `data-user-text`）；语言选项以各自语言书写（`简体中文` / `English`）。
- **失败码**：`DictationPhase.failed.code`（`no_speech | audio | asr | refine | inject | no_selection | selection_too_long | selection | edit_unavailable | edit_in_terminal | unknown`）由 `dictationFailureText` / `takeFailureText` 映射为本地化短句，`unknown` 或缺 `code` 回落到核心的 `message`。
- **语音编辑（`docs/dictation.md` §19）**：设置 › 热键的「编辑选中文本」行（第二个 `useChordRecorder`，与听写录制器互斥；「关闭 / 启用」；`edit_registered` / `edit_error`；没有润色密钥时注明需要 AI 润色服务）；胶囊 `tag="编辑"`（`KindTag`，listening / locked / processing / inserted / error / cancel-armed 前导）；首页阶段行与最近结果表、历史页的编辑行（「编辑」徽标、「指令 → 结果」、详情里可展开的原选区，复制取改写结果，搜索匹配指令与原选区）。

### 7.2 自动更新

- **状态**：`state.update: UpdateStatus`（§3），核心以 `update` 事件推送；`Backend.updateStatus()` 对应查询 `update_status`。
- **UI**（`apps/desktop/src/pages/settings/General.tsx` `UpdateControls`，通用分组完整版，关于分组紧凑版）：`自动更新` Toggle → `settings_set_auto_update`；状态行 `updateStatusLine`：`idle` 尚未检查更新 / `checking` 正在检查更新… / `up_to_date` 已是最新 · {version} · 检查于 {at} / `available` 有新版本 {version} · 当前 {current} / `downloading` 正在下载 {version} · {percent}%（无 `total` 时显示已收 MB）/ `ready` {version} 已下载 · 重启后生效 / `installing` 正在安装 {version}… / `failed` 更新失败 · {message} / `disabled` 此构建未配置更新源。按钮只有一个且随状态变化：`检查更新`（`update_check`；checking / installing / disabled 时禁用）、`查看新版本`（available / downloading → 打开更新对话框）、`重启并更新`（ready → `update_install`）。
- **更新对话框**（`apps/desktop/src/features/update/UpdateDialog.tsx`，参考 clash-verge 的做法）：有新版本、下载中或待重启时，标题栏出现提示（`新版本 {version}` / `下载中 {percent}` / `重启以更新`），点它或设置里的「查看新版本」打开。对话框写明当前版本与发布日期，提供「查看发布页」（`project_link_open { link: "releases" }`，由壳层拼地址），更新内容取自更新器清单 `latest.json` 的 `notes`（release-please 写的 markdown，按标题、列表、段落渲染成文本：不解释 HTML，链接只留文字，开头重复版本号的标题去掉）。「立即更新」之后同一个对话框显示下载进度、速度（最近 3 s 的读数）与剩余时间；「后台下载」只关对话框，不取消下载；下载校验完是「重启并更新」，失败时写原因并可「重试」。
- **MockBackend**：`update_check` → `checking` → 300 ms 后 `available`（固定 `0.0.2`，当前 `0.0.1`，`notes` 为 release-please 格式的 `MOCK_UPDATE_NOTES`）；`update_install` → 三次 200 ms `downloading` tick（`received` 16 / 32 / 48 MB，`total` 48 MB）→ `ready` → 200 ms → `installing`；`simulateUpdate(status)` 供测试到达 `failed` / `disabled` / `up_to_date`；构造参数 `update` 设初始状态。`settings_set_locale` / `settings_set_auto_update` 只回发 `settings`。

## 8. 界面文案

界面上的每句话都说明用户能做什么、看到什么、得到什么，不讲实现细节：不出现进程、后端、状态机、文件名和协议名。

- 语体（用户 2026-09-29 决定）：规范、易读的书面语，减少 AI 腔，参照 macOS 和 Windows 中文界面的写法，如「未设置」「尚未下载」「无法连接」「请稍后重试」「仅保存在本机」。不用口语化说法（还没、没能、免得），不层层解释，不用拟人化的保证。英文同样使用标准界面用语，不写口语。
- 原文没有内部术语、本来就规范的句子保持原样；改动以替换术语和理顺语句为主。
- 一个概念只用一个词，用下表的写法。「本地」指在这台电脑上处理（本地模型、本地识别），「本机」指这台设备本身（本机指纹、仅保存在本机）。中文界面不夹英文术语，产品名、模型名、按键名和平台名除外（Voltip、Qwen3-ASR、Ctrl、Windows）。
- 密码学名称（X25519、ChaCha20-Poly1305、Noise XX）只出现在以「技术细节：」开头的一行里（`devices.syncPanel.crypto`）。
- 核心和壳层的报错原文带有 `scenes:`、`phone text:` 这类机器前缀，界面显示前由 `coreMessageText`（`packages/shared/src/labels.ts`）去掉。
- 守卫：`packages/shared/src/i18n/copy.test.ts` 用下表左栏和一张口语词表扫描两种语言的全部叶子。只有胶囊规格页（`/overlay`，只在开发构建里加载）读取的键豁免，同一测试确认发行代码不读取这些键。

| 内部词 | 中文界面 | English UI |
|---|---|---|
| 核心 | 不出现；等待时写「正在启动…」 | nothing; "Starting…" |
| 热键、组合键 | 快捷键（组合方式仍称组合键、修饰键） | shortcut, key combination, modifier |
| 热键后端 | 快捷键方式（`hotkeyMethodText`：`Windows`、`Linux · Wayland`） | shortcut method |
| 边沿 | 按下 / 松开 | press / release |
| 激活方式 | 录音方式 | recording mode |
| 注入、投递、送出 | 插入、粘贴到光标处 | insert, paste at cursor |
| 输出方式：整段输出 / 流式定稿 / 实时注入 | 输出方式：整段输出 / 边说边识别 / 边说边输入 | output mode: All at once / While you speak / Type as you speak |
| 流式模型 | 实时识别模型 | live transcription model |
| LLM | AI、大模型 | AI, large language model |
| ASR | 语音识别、识别 | speech recognition, transcription |
| 握手、密钥协商 | 建立加密连接 | establish the encrypted connection |
| 对端 | 对方设备 | the other device |
| 票据、会话（配对） | 配对信息、配对 | pairing info, pairing |
| 枚举 | 检测 | detect |
| 持久化 | 保存 | save |
| TTL | 有效期 | valid for |
| VAD | 静音检测（功能名仍为「静音裁剪」） | silence detection |
| UIPI、提权窗口 | 以管理员身份运行的窗口 | a window that runs as administrator |
| `*.json` 文件名 | 本机 | on this computer |
| 收音 | 录音 | recording |
| 悬浮胶囊 | 悬浮窗 | overlay |

前面各节引用的界面文字可能是改写前的旧说法；逐字以两份字典为准（§0）。
