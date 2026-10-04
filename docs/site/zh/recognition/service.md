---
description: 让这台电脑上的其他程序（例如 Paseo 的手机听写）通过 OpenAI 兼容接口使用 Voltip 的语音识别、词典和 AI 预设，可以在应用中开启，也可以在没有桌面的服务器上运行。
---

# 本机服务

本机服务让这台电脑上的其他程序通过 OpenAI 兼容接口使用 Voltip 的语音识别、词典、场景和 AI 预设。其他程序发送一段录音，收到的文字与在应用中听写的处理方式相同：语音识别、词典、按预设进行 AI 润色，最后执行规则。文字不会插入到光标处，也不会写入历史记录。

有两种运行方式，接口相同：

- **在应用中**：在设置中开启。服务在 Voltip 中运行，与听写共用已加载的模型、设置和密钥。
- **在没有桌面的服务器上**：`voltip-server` 是用于 Linux x64 的独立程序，不需要窗口、图形环境或声音系统。

## 在应用中使用

打开「设置 → 本机服务」：

- 「启用本机服务」开启服务；「状态」显示「运行中」和「接口地址」，例如 `http://127.0.0.1:47840/v1`。在其他程序中把它填写为 OpenAI 接口地址（Base URL）。
- 「端口」用于更改端口（1024–65535），服务会在新端口上重新启动。端口已被占用时，状态显示「无法启动」和原因。
- 「处理方式」设置请求默认的处理方式：AI 润色使用的「预设」（「与全局设置相同」或某个预设），以及「场景」（「不使用场景」或某个场景，场景的预设、术语和补充要求同时生效）。请求仍可以另选预设或场景，见[选择处理方式](#选择处理方式)。
- 「复制令牌」把访问令牌复制到剪贴板，窗口中不显示令牌内容。「重新生成」在确认后更换令牌：旧令牌立即失效，已配置的程序都需要改用新令牌。

关闭开关或退出 Voltip 时，服务随之停止。服务只接受来自这台电脑的连接。

## 在没有桌面的服务器上运行

用安装脚本安装 `voltip-server`，不需要管理员权限：

```bash
curl -fsSL https://raw.githubusercontent.com/sunerpy/voltip/main/scripts/install.sh | sh -s -- --server
```

脚本按发布附带的 `SHA256SUMS` 校验下载的文件，解压到 `~/.local/share/voltip-server/<版本>`，并把命令链接为 `~/.local/bin/voltip-server`。缺少系统库时，脚本会列出需要安装的软件包，例如 `sudo apt-get install libblas3`，但不会自行安装。升级时重新运行脚本即可；带上 `--systemd-user --enable` 时脚本会同时重启服务，否则请用 `systemctl --user restart voltip-server` 重启正在运行的服务。

然后检查配置并启动服务：

```bash
voltip-server --check          # 使用的服务、默认处理方式和各项上限；可以识别时退出码为 0
voltip-server --print-token    # 访问令牌，第一次运行时生成
voltip-server --scene coding --preset prompt --language zh
```

服务在前台运行，按 Ctrl+C 或收到 `SIGTERM` 时，等正在处理的请求完成（最多 30 秒）后停止。全部选项见[命令行](/zh/reference/cli#voltip-server)。

- **设置**：`voltip-server` 读取与应用相同的数据目录 `~/.local/share/voltip`。如果要使用你的预设、场景、词典和规则，把应用的 `settings.json`、`presets.json`、`scenes.json`、`dictionary.json` 和 `rules.json` 复制到这个目录。服务端只读取这些文件，文件改动后，下一个请求即按新内容处理。
- **服务**：正式版内置了默认的识别和 AI 润色服务，不需要密钥。也可以使用本地模型：先执行 `voltip-server --download-model qwen3-asr-0.6b`，再用 `voltip-server --local-model qwen3-asr-0.6b` 启动。
- **证书**：云端识别和 AI 润色（包括内置服务）用系统的 CA 证书验证服务器证书。容器镜像等精简系统可能没有这些证书，这时 `--check` 会显示无法识别，服务也不会启动，安装 `ca-certificates` 后即可使用。
- **密钥**：其他服务商的密钥从系统钥匙串读取，不会改动钥匙串。没有桌面会话的服务器通常没有钥匙串，这时 `--check` 会把需要密钥的服务商显示为未配置。
- **后台运行**：安装时改用 `sh -s -- --server --systemd-user --enable`，脚本会另外写入 systemd 用户服务并启动它。如果要在注销后继续运行，请执行 `loginctl enable-linger`。

## 接入其他程序

| 其他程序中的设置 | 填写 |
| --- | --- |
| 接口地址（Base URL） | `http://127.0.0.1:47840/v1`（应用中显示的接口地址） |
| API 密钥 | 访问令牌 |
| 模型 | `voltip` |

例如用 `curl`：

```bash
curl http://127.0.0.1:47840/v1/audio/transcriptions \
  -H "Authorization: Bearer $(voltip-server --print-token)" \
  -F file=@note.wav -F model=voltip -F language=zh
```

返回 `{"text": "…"}`。加上 `-F response_format=text` 时只返回文字；使用 `verbose_json` 时还会列出这次使用的预设、场景和模型。录音中没有说话时，返回的文字为空。

### 选择处理方式

`model` 字段决定请求的处理方式：

| `model` | 处理方式 |
| --- | --- |
| `voltip`，或其他任何名称，例如 `whisper-1` | 默认处理方式：应用中设置的预设和场景，或 `voltip-server` 启动时的选项 |
| `voltip:raw` | 不进行 AI 润色 |
| `voltip:preset=prompt` | 用这个预设进行 AI 润色 |
| `voltip:scene=coding` | 使用这个场景，以及它的预设、术语和补充要求 |
| `voltip:scene=coding;preset=notes` | 使用这个场景，但改用另一个预设 |

请求一旦指定了自己的处理方式，就不再使用默认处理方式中的任何一项。预设和场景可以用标识指定，也可以用它们在应用中的名称指定。`GET /v1/models` 列出全部选择，每一项的 `model` 值都可以原样发送。

`language` 字段指定识别语言，例如 `zh` 或 `en`；`auto` 表示由模型判断。场景设置的语言，或 `voltip-server` 的 `--language` 选项，优先于这个字段。

## Paseo

Paseo 可以把本机服务用于手机应用中的听写。在 Paseo 的 `~/.paseo/config.json` 中填写：

```json
{
  "providers": { "openai": { "stt": { "baseUrl": "http://127.0.0.1:47840/v1", "apiKey": "<访问令牌>" } } },
  "features": { "dictation": { "stt": { "provider": "openai", "model": "voltip", "language": "zh" } } }
}
```

- 请设置 `language`：不设置时，Paseo 会按英文请求识别。
- Paseo 在守护进程启动时读取这些设置，`paseo daemon reload` 不会使它们生效。修改文件后请重新启动守护进程：`paseo daemon restart`。重新启动也会结束守护进程中正在运行的智能体。
- Paseo 每收到 15 秒音频就切出一段，不考虑说话的停顿，并用空格连接各段文字。在 Paseo 守护进程的环境中设置 `PASEO_DICTATION_AUTO_COMMIT_SECONDS=0`，口述结束后会整段发送；守护进程使用启动它的命令的环境，例如 `PASEO_DICTATION_AUTO_COMMIT_SECONDS=0 paseo daemon restart`。如果使用较慢的本地模型且口述较长，可以设为 `110`，让每段在说话过程中就开始识别。
- `model` 可以填写 `GET /v1/models` 中的任何一项，例如 `voltip:scene=coding`。

## 限制

- **音频**：只接受 WAV，采样率 8 至 192 kHz，1 至 8 个声道。每个请求的音频最长 2 小时（可用 `voltip-server --max-minutes` 调低）。超过 2 分钟的录音与应用中的长录音一样分段识别；文字超过 2000 字时不进行 AI 润色。
- **请求**：同时处理 2 个请求，另有 4 个可以排队等待；全部占满时，服务返回 `503`，程序应在几秒后重试。`voltip-server --concurrency` 最多可同时处理 8 个，排队数为它的两倍。
- **返回**：处理完成后一次返回全部文字，不提供时间戳，也不提供字幕格式。

## 安全

- 服务只接受来自这台电脑的连接。`voltip-server --listen` 必须同时加上 `--allow-remote`，才能向其他电脑开放；这时连接不加密，请优先使用 SSH 隧道或 TLS 代理。
- 每个请求都必须携带访问令牌。令牌保存在 `~/.local/share/voltip/serve/token`，只有你的用户可以读取，应用与 `voltip-server` 共用同一个令牌。
- 请求中的内容会按与相同设置下的听写一样的方式发送：录音发送给当前使用的识别服务，文字发送给 AI 服务。见[隐私](/zh/privacy#本机服务)。
