# 应用内反馈

侧栏底部的「反馈」和「设置 · 关于」里的「反馈」打开同一个对话框（2026-09-28 用户选定；从设置进入时它取代设置对话框），浮在当前页面上：选类型（问题 / 建议 / 其他），写描述，可选留联系方式，可选附上截图或录屏。发送前，对话框会列出随反馈附带的全部信息，桌面壳再把报告和附件发到反馈接口。

草稿放在对话框之外（`apps/desktop/src/app/feedback-draft.tsx`）：按 Esc、点遮罩或 × 关掉对话框，再打开时文字、类型、联系方式和已添加的附件都还在，直到发送成功或点「清空」。发送成功后弹出提示、清空草稿并关闭对话框。

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
| `attachments` | 可选，用户添加的截图或录屏：每项是文件名、类型、字节数和 SHA-256，字节随后分块上传 |

不会发出去的：主机名、密钥、端点地址、听写内容、历史、词典和规则。诊断信息由 `apps/desktop/src-tauri/src/feedback.rs` 根据核心当前状态生成；页面里展示的，就是 `feedback_diagnostics` 返回、`feedback_submit` 实际发送的同一份。

## 附件

页面上的「添加截图或录屏」打开系统的文件选择框，也可以把截图直接粘贴到表单里。限制在页面、桌面壳和接口三处一致（`packages/shared/src/schema.ts`、`feedback.rs`、`services/feedback/src/validate.ts`）：

- 类型：`image/png`、`image/jpeg`、`image/gif`、`image/webp`、`video/mp4`、`video/webm`、`video/quicktime`
- 每份反馈最多 3 个；图片每个不超过 5 MiB，视频每个不超过 20 MiB，合计不超过 25 MiB
- 文件名只取最后一段路径，去掉控制字符和引号，最多 120 个字符（过长时保留扩展名）

页面先按类型和大小判断，超限的文件不会被读进内存；通过的文件以原始字节作为 IPC 请求体交给桌面壳（`feedback_attachment_add`，文件名百分号编码后放在 `x-voltip-name` 头，类型放在 `x-voltip-type` 头），桌面壳再检查一遍并暂存在内存里，返回一个 id。`feedback_attachment_remove` 删掉一个，`feedback_attachments_clear` 全部清空：主窗口启动时清一次（刷新过的 webview 不会留下占着名额的文件），「清空」草稿时再清一次；关掉对话框不清。`feedback_submit` 带上这些 id；报告发出后，暂存的文件就被丢弃。

拒绝原因以线上名返回，页面据此给出提示：`attachment_type`、`attachment_too_large`（也包括空文件）、`attachment_too_many`、`attachment_total`、`attachment_name`。发送时多两个原因：`storage_full`（接口的附件存储满了，报告没有发出）和 `attachments`（报告已经发出，但有附件没传完；页面照常清空表单，避免重复发送）。

## 地址是构建期机密

反馈接口的地址和应用令牌在编译时注入，和内置服务一样用 `option_env!`：

- `VOLTIP_FEEDBACK_URL`：完整地址，比如 `https://<feedback-host>/v1/feedback`
- `VOLTIP_FEEDBACK_TOKEN`：应用令牌，以 `Authorization: Bearer` 发送

发布构建从同名的 GitHub Secrets 读取（`.github/workflows/release-candidate.yml`），本地打包从 `.env.build` 读取。仓库和界面里都不出现这个地址；出错时的提示只写原因（`rate_limited`、`network`…），不写地址。没有配置地址的构建，对话框会改为提供仓库的 Issue 页面。

应用令牌就在发出去的安装包里，所以它只能挡住随手的滥用，防不住有心人。限流由接口负责。

## 接口

`services/feedback` 是一个 Cloudflare Worker，报告存进 D1（表结构见 `services/feedback/schema.sql`）：

- `POST /v1/feedback`：检查令牌、`Content-Type` 和体积（≤ 32 KiB），按上面的规则校验字段，多出来的诊断字段直接丢掉（新版应用加字段，旧接口也不会拒收）。同一客户端地址每小时最多 10 条，所有人合计每天最多 5000 条。客户端地址只存加盐哈希，只用于限流。成功时返回 `201 {"id": …}`；声明了附件时再带上 `upload: {"token": …, "chunk_bytes": 1048576}`。附件另有两道限额：同一客户端地址每天声明的附件合计不超过 60 MiB（超出返回 429），附件库总量不超过 400 MiB（超出返回 `507 storage_full`，报告不入库）。
- `PUT /v1/feedback/<id>/attachments/<序号>/<块号>`：上传一个附件的一块，每块 1 MiB（最后一块是余下的部分），带应用令牌和 `x-upload-token`。重传同一块会覆盖；所有块到齐后附件标记为完整。令牌不对返回 404，已完整返回 409，报告发出一小时后返回 410，长度或块号不对返回 400。桌面壳对网络错误和 5xx 每块最多试 3 次。过了这一小时仍没传完的附件连同已到的块，会在下一份带附件的报告到来时删掉，不再占额度。
- `GET /v1/feedback?limit=&before=`：用另一个管理员令牌（`ADMIN_TOKEN`）读回报告，按时间从新到旧，结果里不含地址哈希；每条带上附件列表（文件名、类型、大小、SHA-256、是否完整）。没有配置管理员令牌时返回 404。
- `GET /v1/feedback/<id>/attachments/<序号>`：用管理员令牌下载一个完整的附件，以 `Content-Disposition: attachment` 返回，不在浏览器里直接打开。
- `GET /healthz`：返回 `ok`。

附件的字节存在另一个 D1 数据库（表结构见 `services/feedback/schema-files.sql`，绑定名 `FILES`），报告和附件各占一个数据库的容量上限。没有绑定 `FILES` 时，接口不接收附件（`503 attachments_unavailable`），纯文字反馈不受影响。

Worker 需要三个 secrets：`FEEDBACK_TOKEN`（与构建里的 `VOLTIP_FEEDBACK_TOKEN` 相同）、`IP_SALT`，以及可选的 `ADMIN_TOKEN`；D1 绑定名为 `DB` 和 `FILES`。部署到哪个账号、哪个域名由运维者决定，部署配置不放进仓库。
