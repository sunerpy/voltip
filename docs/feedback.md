# 应用内反馈

侧栏底部的「反馈」打开一个对话框：选类型（问题 / 建议 / 其他），写描述，可选留联系方式。发送前，对话框会列出随反馈附带的全部信息，桌面壳再把报告发到反馈接口。

## 发出去的内容

| 字段 | 内容 |
|---|---|
| `kind` | `bug` / `idea` / `other` |
| `message` | 用户写的描述，去掉首尾空白，1–5000 个 UTF-16 码元 |
| `contact` | 可选，最多 200 个码元；留空就不发 |
| `diagnostics.app_version` | 应用版本 |
| `diagnostics.os` / `arch` | `windows` / `linux` / `macos`，`x86_64` / `aarch64` |
| `diagnostics.session` | 仅 Linux：`wayland` / `x11` / `xwayland` |
| `diagnostics.locale` | webview 解析出的界面语言，比如 `zh-CN` |
| `diagnostics.asr_provider` | 识别服务商的 id（`builtin`、`local`、`openai`…） |
| `diagnostics.local_model` / `compute` | 只在本机识别时带：模型 id、运行设备（`auto` / `cpu` / `gpu`） |
| `diagnostics.llm_provider` | 只在开着润色时带：服务商 id |
| `diagnostics.output_mode` | 下一次听写实际使用的输出方式 |

不会发出去的：主机名、密钥、端点地址、听写内容、历史、词典和规则。诊断信息由 `apps/desktop/src-tauri/src/feedback.rs` 根据核心当前状态生成；对话框里展示的，就是 `feedback_diagnostics` 返回、`feedback_submit` 实际发送的同一份。

## 地址是构建期机密

反馈接口的地址和应用令牌在编译时注入，和内置服务一样用 `option_env!`：

- `VOLTIP_FEEDBACK_URL`：完整地址，比如 `https://<feedback-host>/v1/feedback`
- `VOLTIP_FEEDBACK_TOKEN`：应用令牌，以 `Authorization: Bearer` 发送

发布构建从同名的 GitHub Secrets 读取（`.github/workflows/release.yml`），本地打包从 `.env.build` 读取。仓库和界面里都不出现这个地址；出错时的提示只写原因（`rate_limited`、`network`…），不写地址。没有配置地址的构建，对话框会改为提供仓库的 Issue 页面。

应用令牌就在发出去的安装包里，所以它只能挡住随手的滥用，防不住有心人。限流由接口负责。

## 接口

`services/feedback` 是一个 Cloudflare Worker，报告存进 D1（表结构见 `services/feedback/schema.sql`）：

- `POST /v1/feedback`：检查令牌、`Content-Type` 和体积（≤ 32 KiB），按上面的规则校验字段，多出来的诊断字段直接丢掉（新版应用加字段，旧接口也不会拒收）。同一客户端地址每小时最多 10 条，所有人合计每天最多 5000 条。客户端地址只存加盐哈希，只用于限流。成功时返回 `201 {"id": …}`。
- `GET /v1/feedback?limit=&before=`：用另一个管理员令牌（`ADMIN_TOKEN`）读回报告，按时间从新到旧，结果里不含地址哈希。没有配置管理员令牌时返回 404。
- `GET /healthz`：返回 `ok`。

Worker 需要三个 secrets：`FEEDBACK_TOKEN`（与构建里的 `VOLTIP_FEEDBACK_TOKEN` 相同）、`IP_SALT`，以及可选的 `ADMIN_TOKEN`；D1 绑定名为 `DB`。部署到哪个账号、哪个域名由运维者决定，部署配置不放进仓库。
