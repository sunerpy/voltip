# 手机端 React Native 版（2026-10-07 设计）

> 状态：2026-10-08 起随每个发布版本附带（`Voltip-RN_<版本>_android_arm64.apk`，用户要求「同时发布 RN 版
> APK」），与 Tauri 手机端用同一把密钥签名。`apps/mobile`（Tauri + WebView）照常是主要的手机端；本文的
> `apps/mobile-rn` 是并行的第二个外壳，供用户比较原生感后决定去留。两者共用 Rust 核心与 `@voltip/shared` 的
> IPC 契约，applicationId 不同（`dev.voltip.mobile.rn`），可以和现有手机端同时安装。

## 1. 为什么、做什么

- 用户 2026-10-04 的反馈：手机端「没有原生感」。WebView 里的控件再怎么调，也只是网页模拟的样式；
  React Native 渲染的是系统原生 View（`TextView`、`RecyclerView` 式列表、系统转场与水波纹），无障碍
  树和 Android 的返回手势也是原生的。
- 目标：功能与 `apps/mobile` 0.0.44 对齐的 RN 版，界面按 Material Design 3 重新设计，先在 AWS Device
  Farm 真机上跑通，再交给用户安装验收。
- 不在本次范围：iOS、Google Play 上架、替换 `apps/mobile`、本地模型（手机本来就没有）、应用内更新（更新状态
  固定为 `disabled`，关于页因此不显示更新卡片；新版本从发布页下载 APK 覆盖安装）。2026-10-08 起接入 CI 与发布
  候选，发布的 APK 用发布密钥签名（§6）；本机构建默认仍用 Android 调试密钥签名，这样的包不能被发布版覆盖，要先卸载。

## 2. 总体结构

```
apps/mobile-rn (Expo SDK 57 · RN 0.86.3 · React 19.2.3 · react-native-paper 5 MD3 · React Navigation 7)
  src/          界面：页面、主题、导航；@voltip/shared 的 TauriBackend + RN 传输层
  modules/voltip-native   本地 Expo 模块（Kotlin）：invoke / 事件 / 平台回调；Keyring.kt
  rust/         crate voltip-mobile-rn → libvoltip_rn.so（cdylib）：命令分发 + UniFFI 接口（ffi.rs）
  rust/bindgen  crate voltip-uniffi-bindgen：UniFFI 的绑定生成器（构建时生成 Kotlin，以后生成 Swift）

JS ──invoke(command, argsJson)──▶ Kotlin VoltipNativeModule ──UniFFI（JNA）──▶ VoltipShell ──▶ voltip-tauri-bridge::Bridge ──▶ voltip-core
JS ◀──sendEvent("voltip://event")── Kotlin VoltipHost（PlatformHost）◀──UniFFI 回调── 事件循环（UiEvent、电平帧）
```

- **一份契约**：JS 端仍是 `TauriBackend`（`packages/shared/src/tauri-backend.ts`），只换掉传输层
  `TauriTransport`：`invoke` 走原生模块，`listen` 订阅原生事件，`channel` 换成按 id 分发的电平流。
  zod 校验、`applyEvent`、i18n 字典、标签函数全部复用。
- **一套命令**：命令名与参数和 `apps/mobile/src-tauri/src/lib.rs` 的 110 个命令一致。凡是
  `bridge.dispatch(UiCommand::X { .. })` 的命令走通用路径：把 `{ "command": name, ...args }` 反序列化成
  `UiCommand`（`crates/voltip-tauri-bridge/tests/contract.rs` 与 `voltip-bridge-harness` 已证明这条映射对
  每个变体成立）；其余的查询和外壳命令逐个实现，行为照搬 Tauri 手机端（见 §4）。
- **原生优先不变**（`docs/architecture.md` §1.1）：音频、识别、剪贴板、密钥、网络、配对都在 Rust；
  Kotlin 只做必须调用 Android API 的部分（剪贴板、分享面板、组播锁、打开网址、系统强调色）；JS 只
  渲染 `UiState`、发命令。两项只涉及交互、不经手数据的能力放在 JS：扫码时的相机取景（扫到的字符串原样交给
  Rust 的 `pairing_join_ticket`）和触感反馈。麦克风的运行时权限由 JS 在开始录音前申请（系统对话框），
  录音本身仍是 Rust 的 cpal/AAudio。

## 3. Rust：`voltip-mobile-rn`

- 位置 `apps/mobile-rn/rust`，加入根 workspace（`cargo clippy --workspace` 与测试照常覆盖它）。
- 一个 crate，两层：
  - 外壳（`shell.rs`、`commands.rs`，与平台无关，可在 Linux 上测试）：`Shell` 持有 `Bridge`、tokio 运行时、
    电平订阅、反馈附件；`Shell::invoke(command, args)` 的答复是 `Result<Value, String>`。平台能力经 `Host`
    trait 注入（剪贴板读写、分享文本/文件、组播锁、打开网址），测试用假实现。
  - 对外接口（`ffi.rs`，UniFFI 0.32 的 proc-macro，2026-10-08 由手写 JNI 改来）：
    - `VoltipShell`：`start(dataDir, appVersion, host)` 每个进程只启动一次，JS 重新加载后再调用会交回正在运行的
      外壳并换上新的 host；`invoke(command, args)` 是异步方法，在 Kotlin 里是挂起函数（Expo 的 `Coroutine`
      异步函数直接调用），命令在外壳自己的 tokio 运行时上执行，返回 JSON 或与 Tauri 版相同的错误文本
      （`ShellError`，Kotlin 里的 `ShellException.message` 就是这段文本）。
    - `PlatformHost`：由 Kotlin 的 `VoltipHost` 实现的外部 trait（事件、电平帧、剪贴板、分享、组播锁、打开网址），
      失败以 `HostError` 返回；Kotlin 抛出的其他异常经 `UnexpectedUniFFICallbackError` 变成同一个错误，不会 panic。
    - 绑定由 `scripts/build-android-rn.sh` 在编出 `.so` 之后用 `voltip-uniffi-bindgen` 从库里的元数据生成
      （`uniffi.toml`：包名 `dev.voltip.rn.uniffi`），写进模块的 Kotlin 源码目录，不提交。元数据在符号表里，所以
      release 配置为这个 crate 保留符号（根目录 `Cargo.toml`），生成绑定后脚本再用 NDK 的 `llvm-strip` 去掉；
      不为构建主机另编一份（2026-10-08：CI 的 Android runner 没有 ALSA 的开发包，主机构建在 `alsa-sys` 停下）。
      Kotlin 侧经 JNA 调用（`net.java.dev.jna:jna` 5.19.1）。
    - FFI 胶水全由 UniFFI 生成，这个 crate 自己没有 unsafe 代码，所以继承 workspace 的 lint 表，
      `unsafe_code = "forbid"` 照样生效；调用里的 panic 由 UniFFI 转成错误，不会越过 FFI 边界。
    - iOS 用同一套接口：生成 Swift 绑定、把库编成静态库，再写一个 Swift 版的 Expo 模块即可，不必再手写一层桥。
- 从 `apps/mobile/src-tauri` 复制、去掉 Tauri 依赖的模块：麦克风端口（`PhoneMicrophone`）、电平订阅、剪贴板
  注入器。两个外壳并存期间是重复代码；哪个外壳留下，就把这些模块合回一处（见 §10）。
- 日志：`android-native-keyring-store` 的 `android-log` 特性提供 `KeyringLog.setLog`，Kotlin 在启动前调用一次，
  Rust 的 `tracing` 输出进 logcat（按 target 分 tag）。
- `CoreConfig` 与 Tauri 手机端相同（数据目录改为 RN 应用自己的 `filesDir`）：`accepts_phone_takes = false`、
  `shows_live_preview = false`、`manual_scenes = true`、`SyncRole::Phone`、mDNS 发现。
- 密钥：`AndroidKeystoreSecretStore("dev.voltip.mobile", "voltip")`，Keystore 不可用时拒绝启动，绝不退回不安全
  的存储；其他平台目前没有安全存储，`VoltipShell::start` 直接拒绝。`Keyring.initializeNdkContext(applicationContext)`
  （`android-native-keyring-store` 自带的 JNI 函数，同在 `libvoltip_rn.so` 里）在 Rust 启动前由 Kotlin 调用，
  它同时初始化 `ndk-context`，cpal 的 AAudio 枚举也靠它。
- TLS：Android 上 `voltip_cloud::http_client_builder` 用 Mozilla 根证书，不需要 rustls-platform-verifier 的
  JNI 初始化。

## 4. 命令面

| 类别 | 命令 | RN 版做法 |
| --- | --- | --- |
| 通用 | 所有映射到 `UiCommand` 的命令（配对、设备、设置、听写、词典、规则、场景、预设、历史增删…） | `{command, ...args}` → `UiCommand` → `bridge.dispatch` |
| 状态 | `core_state`、`update_status` | `bridge.state()`；更新状态固定为 `disabled` |
| 查询 | `history_query/entry/stats/hits`、`mirror_*`、`recent_apps`、`rules_export`、`vocabulary_preview`、`presets_builtin`、`scenes_builtin`、`feedback_diagnostics` | 阻塞线程上读，与 Tauri 版同一函数 |
| 手机专有 | `phone_take_start`、`dictation_start` | JS 先申请 `RECORD_AUDIO`，Rust 再 dispatch |
|  | `phone_clipboard_read`、`paste_text`、`phone_share_text`、`history_export` | `Host` 回调 Kotlin（剪贴板 / 分享面板 / 分享文件） |
|  | `settings_set_lan_discovery` | dispatch 后按开关持有或释放组播锁 |
|  | `audio_meter_start/stop` | 订阅 `bridge.levels()`，帧按订阅 id 发往 JS |
|  | `provider_console_open`、`project_link_open`、`guide_open` | Rust 拼出网址，Kotlin `ACTION_VIEW` 打开；JS 从不打开收到的网址 |
|  | `update_check`、`update_install` | 固定错误（验收版没有更新渠道），`update_status` 为 `disabled` |
|  | `feedback_submit`、`feedback_attachment_*` | 与 Tauri 版相同；附件字节经 base64 传入 |
| 拒绝 | `hotkey_edge`、`model_*` | 同样的错误文本（手机没有快捷键和本地模型） |
| 空答 | `audio_devices`、`audio_outputs`、`overlay_state`、`permissions_*`、`inject_preflight`、`hotkey_capture` | 与 Tauri 版相同的固定回答 |

- 错误是字符串，形状与 Tauri 的 `Result<_, String>` 一样，JS 端 `errorText` / `coreMessageText` 照常解析。
- 每次调用是一个挂起的 `VoltipShell.invoke`：命令在 tokio 上执行，慢命令（网络、数据库、分享面板）不占住其他调用。

## 5. 界面

- **组件库**：react-native-paper 5 的 MD3 组件（`Appbar`、`List.Item`、`Switch`、`TextInput`、`Menu`、
  `Dialog`、`Snackbar`、`SegmentedButtons`、`Card`、`ProgressBar`、`Chip`），水波纹和按压态由原生提供。
  下拉选择用 `Menu` 锚定在字段下方，不出现老式单选对话框。Paper 5.15.3 的 `Menu` 有一个竞态：它以关闭状态挂载时
  会跑一次 250 ms 的隐藏动画，动画结束时无条件卸载菜单项；在这段时间里（页面刚打开，或刚关上又马上打开）展开的
  菜单因此只剩一个“已展开”的字段。`patches/react-native-paper@5.15.3.patch`（pnpm `patchedDependencies`）让隐藏动画
  结束时先看菜单是否又被打开，打开了就保留菜单项，并让动画期间的再次打开重新播放展开动画；回归测试
  `regression: a dropdown opened while its page is still settling stays open` 用假时钟复现。升级 Paper 时先检查上游
  是否已修复，再决定去掉补丁。
- **配色（2026-10-07 用户选定「简约中性」）**：表面接近纯中性色，白卡片加 1 dp 细描边、14 dp 圆角，分组标题用
  灰色小字，强调色只用在能点的东西和选中态上：按住说话是实心主色圆形按钮（带同色阴影），按钮 12 dp 圆角
  （`ui/Button.tsx`），底栏是卡片白加顶部细线、选中的标签直接变成主色、不画胶囊指示器，顶栏和页面同色。
  颜色由 Google 的 Material Color Utilities（Android 动态配色用的同一套算法）从一个种子色算出（`theme/themes.ts`）：
  强调色取 SchemeFidelity（保留种子的饱和度），它的容器色取同一色板的柔和色调；表面和描边取种子色相、几乎
  没有饱和度的中性色板，暖纸用奶油色调，石墨从纯黑提亮；ok / warning / danger 按 MD3 的做法向种子色相靠拢。
  种子：外观选「跟随系统」时用壁纸取色（Android 12+ 由 Kotlin 的 `systemAccent()` 读系统的
  `system_accent1_500`，回到前台时重新读），Android 12 以下或手动选了主题时用 Voltip 的墨蓝 `#339cff`，暖纸用它的
  赭石色。四套主题（明亮、暗黑、暖纸、石墨）照旧，「跟随系统」按系统深浅色切换；状态栏、导航栏随主题着色，
  窗口铺满到屏幕边缘并处理安全区。当时比较过的另外两种是标准 MD3（TonalSpot）和 Material 3 Expressive。
- **导航**：根是 native-stack，第一层是底部三个标签（说话 / 记录 / 设置，Paper 的 `BottomNavigation.Bar`），
  其余页面压在根栈上（进入详情页时标签栏收起，与现手机端一致）。转场用系统默认动画；Android 返回手势由
  React Navigation 处理：先关菜单和对话框，再退一级；记录、设置回到说话；在说话页第一次返回提示、两秒内
  第二次返回退出。
- **页面**：与 `apps/mobile/src/screens` 一一对应——说话（未配对时的欢迎页 / 已配对的设备页，含按住说话、
  发送文字、最近结果、连接自检）、配对（扫码 / 6 位码 / 附近的电脑）与核对安全码、本机、记录（搜索、筛选、
  统计）、记录详情、电脑记录的副本与详情、设置首页、语音模型、AI 模型（服务商卡片、候补模型、预设）、外观、
  录音、词典、规则（导入 / 导出）、场景、记录设置、电脑设置、关于、反馈。
- **文字**：全部走 `@voltip/shared` 的类型化字典（`zh-CN.ts` 定义键，`en.ts` 对应），新增文案两种语言同时加；
  用语按现有产品文案的标准（memory `feedback-ui-copy-register`）。
- **React 版本**：RN 0.86 要求 React 19.2.3，仓库其余部分是 19.3.0，所以 RN 版不能引用 `@voltip/ui`
  （会出现两份 React）。`BackendProvider`、`I18nProvider`、`useNow`、`useHistoryList` 等不依赖 DOM 的
  hooks 复制到 `apps/mobile-rn/src/backend`，逻辑本身仍在 `@voltip/shared`。

## 6. 构建

```bash
scripts/build-android-rn.sh             # 调试密钥签名：dist/android-rn/Voltip-RN_<version>_android_arm64.apk
scripts/build-android-rn.sh --unsigned  # 不签名：…_android_arm64-unsigned.apk，CI 与发布候选随后自己签名
```

版本号是仓库的版本（根目录 `package.json`，release-please 写它，Tauri 两端也读它）：`app.config.js` 在
`app.json` 之上填 `version` 和 `versionCode`（主版本 × 1000000 + 次版本 × 1000 + 补丁号，与 Tauri 手机端相同），
`apps/mobile-rn/package.json` 的版本不用（`0.0.0`）。

1. `cargo ndk -t arm64-v8a --platform 26 build --release -p voltip-mobile-rn`（`.env.build` 经
   `scripts/lib/build-env.sh` 注入内置服务地址，与现手机端相同），`.so` 放进本地模块的 `jniLibs`；cargo-ndk 顺带
   复制的 `android-native-keyring-store` 自己的 cdylib 删掉（它的 JNI 入口已链接进 `libvoltip_rn.so`）。随后
   `cargo run -p voltip-uniffi-bindgen` 从这个库生成 UniFFI 的 Kotlin 绑定，写进模块源码（§3），然后用 NDK 的
   `llvm-strip --strip-all` 去掉打包那份的符号。
2. `expo prebuild --platform android --clean`：Android 工程每次生成，不提交（`android/` 被 git 忽略）；
   需要的原生配置都写在 `app.json` 与本地模块里（权限、FileProvider、minSdk 26、targetSdk 36）。注意相机和图片
   选择插件的 `recordAudioAndroid: false` / `cameraPermission: false` 会以 `tools:node="remove"` 删掉
   `RECORD_AUDIO` / `CAMERA`，所以不传这两项。第三方许可证文本由
   `scripts/release/third-party-notices.py --app mobile-rn`（Rust 外壳的 crate 与 `apps/mobile-rn` 的 npm 包）生成到
   `assets/THIRD-PARTY-NOTICES.txt`，与 Tauri 手机端一样随 APK 附带（关于页提到这个文件），打包后脚本核对它的首行版本。
3. `gradlew assembleRelease`（Hermes 字节码、只打 arm64-v8a），默认用 Android 调试密钥签名；`--unsigned` 时
   `app.config.js` 的配置插件（`plugins/release.js`）去掉模板给 release 构建类型配的调试签名，模板变了就报错，
   不会悄悄出一个调试签名的包。React Native 的 Gradle 插件要 JDK 17 工具链：`JAVA_TOOLCHAINS` 指向一个 JDK 17
   （Gradle 自动下载会去 GitHub，本机不可用）。
4. 发布前扫描：APK 的每个条目都过 `voltip_scan_provider_keys`；JS 包是 Hermes 字节码，字符串表首尾相接，图标名
   （`task-outline` 等）会被误认成 `sk-…`，所以它改为扫描 Metro 打包前的源码（packager source map 的
   `sourcesContent`），并检查构建环境里的密钥值不在字节码中。`.env.build` 以外的生产主机名不得出现在源码树里
   （`check-no-production-hosts.sh` 照常覆盖 `apps/mobile-rn`）。
5. 发布（2026-10-08 起，`docs/runbook.md` 发布 · Android）：发布候选的 `bundle-android` 腿在 Tauri 的包之后、
   签名之前运行 `build-android-rn.sh --unsigned`（内置服务的值与 Tauri 手机端相同），签名一步用同一把密钥签它
   （`.github/scripts/sign-android-package.sh --app mobile-rn`），`check-android-package.sh --app mobile-rn` 检查
   证书、16 KB 对齐、版本、targetSdk、许可声明和原生库里的密钥，`updater-json.py collect --extra` 把它作为这条腿的
   附加文件封存；`device-android` 在模拟器上启动它（`android-device-smoke.sh --app mobile-rn`）。CI 的 `android-rn`
   与 `android-rn-device` 对每个改了代码的 PR 走同一条路，用当次生成的临时密钥签名，不注入内置服务。

## 7. 测试

- Rust：`Shell` 的命令分发在 Linux 上用假 `Host` 和内存密钥库测试：每个命令名都能分发（对照
  `packages/shared/src/fixtures/ipc/commands.json`）、查询返回能被 zod schema 接受的形状、外壳命令的固定
  回答、平台回调的参数。
- JS：jest-expo + @testing-library/react-native，用 `@voltip/shared/mock` 的内存后端渲染各页面，覆盖导航、
  按住说话的状态、配对流程、下拉选择、返回键。
- 真机：AWS Device Farm（us-west-2），`scripts/devicefarm-rn.sh <apk> <设备池> <输出目录>`。自定义测试环境
  （`apps/mobile-rn/devicefarm/testspec.yml`）+ adb 驱动的 `acceptance.sh`（与 `.github/scripts/android-device-smoke.sh`
  同一思路），设备语言设为 zh_CN：安装、冷启动计时、首屏出现「按住说话」、授予麦克风后按住说话录 4 秒（设备
  房间里的声音，验证录音和识别链路走到结果或原因）、三个标签页、语音模型 / AI 模型 / 外观（切到暗黑）/ 录音
  （下拉菜单，返回键先关菜单）/ 词典（全屏编辑器）、本机与配对页、返回逐级、说话页两次返回退出、内存与掉帧；
  `summary.txt` 逐项记录，截图与界面树回传后逐张查看。
- 结果（2026-10-07，换成「简约中性」配色后的 `rn-style-c2`，同一个 APK 在三台手机上全部通过）：

  | 设备 | 冷启动 | 首屏 | 内存（TOTAL PSS） | 跟随系统时的强调色 |
  | --- | --- | --- | --- | --- |
  | Pixel 8 · Android 15 | 473 ms | 3 s | 258,942 KB | 壁纸取色（这台机器是绿色） |
  | Galaxy A54 · Android 13 | 662 ms | 3 s | 250,038 KB | 壁纸取色（青蓝） |
  | Pixel 3 · Android 10 | 916 ms | 2 s | 288,094 KB | 系统不提供，用墨蓝 |

  三个标签页、设置各页、暗黑主题、跟随系统（开关打开后强调色换成壁纸色）、下拉菜单（返回键先关菜单）、词典编辑器、
  规则、场景、记录设置、反馈、关于、本机、配对页（扫码与 6 位验证码两种方式的界面）、逐级返回、说话页两次返回退出
  都通过，没有崩溃。按住说话录音 4 秒，AAudio 录音正常结束；机房里没人说话，引擎给出「没有听到声音」（`rn-accept-4`
  里 A54 识别出了文字）。Device Farm 上配对不了电脑，配对后的链路（发到电脑、同步）留给用户验收。
- 跑通之前修掉的问题：相机插件删掉了 `RECORD_AUDIO`（§6）；返回键关菜单时页面也跟着返回（Paper 的返回监听
  不拦截，`Select` 自己拦）；上面 §5 的菜单竞态；以及脚本自身的几处（Pixel 8 的标签文字离导航栏太近，改按
  testID 点；欢迎页的「连接电脑」既是标题又是按钮；慢手机上返回提示在读取界面树之前就消失了）。

## 8. 验收（用户）

安装发布页的 `Voltip-RN_<版本>_android_arm64.apk`（或本机构建的 `dist/android-rn/` 下的 APK；两者签名不同，
换装要先卸载），与现手机端并存，对比：

1. 观感：MD3 控件、水波纹、转场、深浅色、状态栏和导航栏、字体缩放后的布局。
2. 手势：返回手势逐级返回、说话页两次返回退出。
3. 功能：未配对时按住说话（内置云服务）结果进剪贴板；扫码或输入 6 位码配对电脑；按住说话发到电脑；
   发送文字；记录与详情；设置各页可改且与电脑同步。

## 9. 风险与取舍

- 两份 React（19.2.3 / 19.3.0）与复制的 hooks：并存期间的代价；RN 胜出后把不依赖 DOM 的 hooks 抽到一个以
  react 为 peer 依赖的包。
- 复制的 Rust 外壳模块：同上，二选一后合并。
- APK 体积：实测 64.0 MB（arm64；`libvoltip_rn.so` 19.8 MB，许可证文本压缩后约 0.13 MB），比预估的 45–50 MB 大；RN 胜出后再看
  拆分 ABI、去掉不用的 Expo 模块。
- 没有模拟器（本机无 KVM），设备上的问题只能在 Device Farm 上复现；先跑一次只含启动的冒烟构建，
  确认工具链和设备链路再铺开页面。

## 10. 如果 RN 版胜出

- `apps/mobile-rn` 接替 `apps/mobile` 的 applicationId（发布密钥和 `release-candidate.yml` 的 Android 腿已经共用），
  补上应用内更新与 AAB；
- 合并复制的 Rust 模块与 hooks；
- `docs/site` 的手机端页面与截图重拍。
