# 架构（Tauri + Rust + React + TypeScript + Tailwind）

> 本文描述仓库当前的分层、边界与数据落地。协议细节见 `docs/protocol.md`，配对见 `docs/pairing.md`，听写流水线见 `docs/dictation.md`，前端见 `docs/frontend.md`。

## 1. 分层

```
apps/desktop  (Tauri 2 · Windows / macOS · React 19 + TS + Tailwind 4)
apps/mobile   (Tauri 2 · Android · 同一套 React 代码，移动端路由)
packages/ui       设计系统：四套主题令牌、原子组件
packages/shared   TS 侧协议类型 / IPC 契约 / 状态机类型（与 Rust `voltip-protocol` 一一对应）

crates/voltip-protocol   版本化线协议（version 字段、tagged enum、CBOR/JSON 编解码、PairingTicket）
crates/voltip-crypto     Noise_XX_25519_ChaChaPoly_SHA256 握手/传输、指纹与 Safety Code、随机数
crates/voltip-identity   长期 Device Identity、SecretStore trait（keyring / 内存 / 加密文件）、TrustedDevice 存储
crates/voltip-pairing    配对状态机（sans-IO）：会话、二维码票据、6 位短码、过期、一次性、防重放
crates/voltip-transport  Transport trait、连接状态机、重连策略、Direct(WS over LAN：每台设备常驻的 DirectHost) 与 WsRelay 两种实现、SecureChannel
crates/voltip-relay      可选中继服务（axum WS）：会话/短码索引、限流、失败计数、只转发密文
crates/voltip-core       应用外观层：把 identity + pairing + transport + trusted store 组合成 CoreCommand → CoreEvent；Hotkey 解析/规范化与 Settings.hotkey
crates/voltip-tauri-bridge  UiCommand / UiEvent / UiState：webview 看到的唯一契约；Bridge::publish 让 shell 自己产生的状态（热键注册结果）并入同一状态流
crates/voltip-audio      麦克风枚举与输入电平（cpal：WASAPI / CoreAudio / ALSA / Android AAudio），纯 DSP 可脱离硬件测试
crates/voltip-cloud      云端识别与润色（voltip-asr、voltip-refine 的客户端；阿里云百炼的实时模型经 WebSocket 推流）作为核心的 Transcriber / Refiner / StreamingTranscriber 端口，桌面与手机共用
crates/voltip-serve      本机语音服务的 HTTP 层（axum）：OpenAI 兼容的 /v1/audio/transcriptions、令牌、准入、边收边解码的 WAV 上传；处理在 voltip-core 的 serve 模块（docs/dictation.md §23）
apps/desktop/src-tauri   桌面 shell：全局热键（tauri-plugin-global-shortcut）、悬浮胶囊窗口（预热、事件驱动）、无边框主窗口、音频电平 Channel；设置里打开本机服务时经 voltip-serve 监听 127.0.0.1
apps/server              无头服务端 voltip-server（Linux x64 tar.gz）：命令行、只读地读 App 的数据目录与系统钥匙串，没有 GUI 和音频依赖
```

依赖方向只允许向下：`apps → core → {pairing, transport, identity} → {crypto, protocol}`；`relay` 只依赖 `protocol`，不依赖 `crypto`（它拿不到、也不需要会话密钥）。`voltip-serve` 是 `voltip-core` 之上的适配层（实现核心的 `ServeHost` 端口，调用 `SpeechService`），只有桌面壳和 `apps/server` 依赖它。

## 1.1 原生优先（硬约束）

- **凡是系统能力都在 Rust 里做**：全局热键、音频设备与电平、文本注入、窗口管理、剪贴板、文件。webview 只渲染与收集输入；浏览器 API（`getUserMedia`、`navigator.keyboard`…）不作为正式实现，只能是浏览器预览的降级。
- **Rust → webview 的通道按语义选**：离散状态变化（设置、设备列表、配对、热键注册结果）走 `voltip://event` 事件总线并折叠进 `UiState`；**连续流**（电平帧、进度、识别流）走 `tauri::ipc::Channel`，由发起该流的 command 返还，随 command 生命周期结束；一次性查询走 command 返回值。事件总线不承载 30 Hz 以上的数据。
- **shell 自己产生的状态**（热键注册 / 按下 / 录制态）经 `Bridge::publish` 并入 `UiState`，`core_state` 与事件流始终一致；webview 不维护第二份真相。
- **录制快捷键时挂起自己的热键**（`hotkey_capture`）：`RegisterHotKey` 会吞掉已注册的组合键，不挂起就永远录不到同一个键。
- **悬浮胶囊窗口**启动即预热（隐藏，`state=blank`），之后只 show / hide；状态经窗口事件 `voltip://overlay` 下发而非导航；隐藏前先画空一帧。窗口不取焦点，因此取消键只能是「监听期间临时注册」的热键（尚未实现）。
- 客户端禁用浏览器右键菜单（可编辑控件除外），主窗口无原生装饰，40 px 标题栏即工具栏。

## 2. 关键边界

| 边界 | 规则 |
|---|---|
| Rust ↔ TS | 只经 Tauri command / event / `ipc::Channel`；TS 不做任何密码学，也不实现系统能力；`packages/shared` 的类型镜像 Rust 的 serde 输出，用 zod 校验，`packages/shared/src/fixtures/ipc/*.json` 由 Rust 契约测试生成 |
| UI ↔ Transport | UI 只见 `CoreEvent`（状态机快照 + 设备列表），不见 socket |
| Crypto ↔ 业务 | `voltip-crypto` 只暴露 `Handshake` / `SessionCipher` / `SafetyCode`；业务层不拼 nonce、不选算法 |
| Relay ↔ 协议 | Relay 只解析 `RelayFrame`（控制帧），`Forward.payload` 是不透明字节 |
| Endpoint | 生产地址不进源码：`VOLTIP_RELAY_URL` 由构建期注入（Tauri config `build.beforeBuildCommand` 读环境变量写入 `relay.json`），运行时可被用户设置覆盖；默认可完全不开启 Relay |

## 3. 数据落地

| 数据 | 位置 |
|---|---|
| Identity 私钥 | 平台安全存储：Windows Credential Manager / macOS Keychain（`keyring`；macOS 发布包按构建建条目，应用内更新时交接，见 `docs/runbook.md` 发布），Android Keystore（移动端 `SecretStore` 实现），测试用内存实现 |
| Trusted devices | `app_data_dir/trusted-devices.json`（只含公钥指纹、名称、平台、last_seen、connection_type） |
| Settings | `app_data_dir/settings.json`（主题、relay url、热键等） |
| 本机语音服务的令牌 | `app_data_dir/serve/token`（0600；桌面 App 与 `voltip-server` 共用，`docs/dictation.md` §23.7） |
| 本机语音服务的临时上传 | `app_data_dir/serve/uploads/`（解码后的 16 kHz PCM，请求结束即删除，启动时清理残留） |
| Pairing session | 只在内存；过期或用完即销毁 |

## 4. 构建与配置

- Rust 1.98 stable（`rust-toolchain.toml`），Tauri 2.11，React 19，Vite 7，Tailwind 4，Vitest 3。
- 环境：debug 构建默认 `ws://127.0.0.1:47830/ws`；发行构建的 `wss://…/ws` 只在 GitHub secret `VOLTIP_RELAY_URL`（本机 `.env.build`）里，构建期注入，源码只有 dev 默认值与"未配置 = 关闭 Relay"。
- 覆盖率门：Rust `cargo llvm-cov` 过滤行覆盖 ≥ 90%（排除 `main.rs`、平台 keychain 胶水、Tauri app crate）；TS `vitest --coverage` 行/分支/函数/语句 ≥ 90%（排除 `bootstrap.tsx`、`vite-env.d.ts`）。
