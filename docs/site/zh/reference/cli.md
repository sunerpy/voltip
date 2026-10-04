---
description: voltip-desktop 的命令行选项，用于控制正在运行的应用，以及在不打开窗口的情况下识别音频文件；以及用于没有桌面的电脑的本机服务 voltip-server 的选项。
---

# 命令行

Voltip 的程序 `voltip-desktop`（Windows 上为 `voltip-desktop.exe`）接受的选项有两种用途：控制已经在运行的 Voltip，以及在不打开窗口的情况下识别音频文件。

## 控制正在运行的应用

这些选项会转交给已经在运行的 Voltip。没有运行中的 Voltip 时，`--quit` 直接退出，其他选项会正常启动程序。

| 选项 | 作用 |
| --- | --- |
| `--toggle` | 开始听写，或结束听写并插入文字 |
| `--edit-toggle` | 开始或结束一次对选中文字的[语音编辑](/zh/dictation/voice-edit) |
| `--cancel` | 取消当前的录音或结果 |
| `--start-hidden` | 启动时不显示主窗口，只保留托盘和快捷键 |
| `--quit` | 退出正在运行的 Voltip，与托盘菜单中的「退出 Voltip」相同 |

在 Linux 的 Wayland 会话中，请为 `--toggle`、`--edit-toggle` 和 `--cancel` 绑定系统快捷键，见[快捷键与录音方式](/zh/dictation/shortcuts#linux-与-wayland)。

## 不打开窗口

这些选项输出结果后即退出。它们不会打开窗口、托盘图标、快捷键或麦克风，适合在脚本和服务器上使用。

| 选项 | 输出 |
| --- | --- |
| `--list-devices` | 输入设备，每行一个：标识、制表符、名称；默认设备排在最前 |
| `--list-models` | 本地模型及各自是否已安装 |
| `--list-compute` | 处理器线程数，以及这个版本可以使用的每块显卡 |
| `--download-model <id>` | 下载模型，可接着上次中断的位置继续，并校验哈希值。进度输出到标准错误 |
| `--transcribe-file <文件.wav>` | 用本地模型识别 WAV 文件并输出文字 |

`--transcribe-file` 还接受以下选项：

| 选项 | 作用 |
| --- | --- |
| `--model <id>` | 使用的模型。默认为应用中选用的模型，否则为 Qwen3-ASR 0.6B |
| `--json` | 输出 `{ "text", "model", "latency_ms", "backend" }`，而不只是文字 |
| `--device auto\|cpu\|gpu` | 模型的运行设备。默认使用应用中的设置 |
| `--gpu <名称>` | 使用哪块显卡，名称见 `--list-compute` 的输出 |
| `--threads <n>` | 使用的处理器线程数 |

文件最好是 16 kHz 单声道，其他采样率会自动转换。模型标识为 `qwen3-asr-0.6b`、`qwen3-asr-1.7b`、`sense-voice-small` 和 `paraformer-zh`。

## 退出码

| 退出码 | 含义 |
| --- | --- |
| 0 | 成功 |
| 1 | 模型未安装，或识别失败；原因输出到标准错误 |
| 2 | 无法读取音频文件 |

## 示例

```bash
voltip-desktop --download-model sense-voice-small
voltip-desktop --transcribe-file meeting-note.wav --model sense-voice-small --json
```

在 Windows 上，程序按桌面应用构建，因此 PowerShell 不会直接显示它的输出。请把输出重定向到文件：`voltip-desktop.exe --transcribe-file a.wav --json > out.json`。

## voltip-server

`voltip-server` 在没有桌面的电脑上运行[本机服务](/zh/recognition/service)。它适用于 Linux x64，用 `install.sh --server` 安装。不带选项时，它在 `127.0.0.1:47840` 上监听，直到按下 Ctrl+C 或收到 `SIGTERM`。

| 选项 | 作用 |
| --- | --- |
| `--listen <地址:端口>` | 监听的地址。默认为 `127.0.0.1:47840`。其他电脑可以访问的地址还需要加上 `--allow-remote` |
| `--allow-remote` | 允许使用这类地址。连接不加密 |
| `--token-file <路径>` | 访问令牌的文件，第一次运行时生成。默认为 `~/.local/share/voltip/serve/token` |
| `--scene <场景>` | 每个 `voltip` 请求使用的场景：内置场景（`coding`、`office`、`chat`、`legal`、`medical`、`finance`、`academic`）或场景的名称 |
| `--app <应用标识>` | 代替 `--scene`：使用列出了这个应用的场景，如同它是前台应用 |
| `--preset <预设>` | AI 润色使用的预设：内置预设（`proofread`、`prompt`、`intent`、`chat`、`translate`、`notes`、`punctuation`、`formal`）或预设的名称 |
| `--refine on\|off` | 每个 `voltip` 请求是否进行 AI 润色。默认使用设置或场景中的选择 |
| `--language <代码>` | 每个请求使用的识别语言（`zh`、`en` 等；`auto` 表示由模型判断） |
| `--script simplified\|traditional\|as_is` | 中文字形 |
| `--asr <服务商>`、`--llm <服务商>` | 改用其他识别或 AI 润色服务商，例如 `builtin`、`local` 或 `aliyun` |
| `--local-model <id>`、`--device`、`--gpu`、`--threads` | 用本地模型识别，含义与 `--transcribe-file` 相同 |
| `--max-minutes <1–120>` | 每个请求的音频最长时长（分钟）。默认为 120 |
| `--concurrency <1–8>` | 同时处理的请求数。默认为 2 |
| `--no-preload` | 启动时不加载本地模型，第一个请求会等待模型加载 |

以下选项输出结果后即退出：

| 选项 | 输出 |
| --- | --- |
| `--check` | 使用的服务、默认处理方式、各项上限和提示 |
| `--print-token` | 访问令牌，第一次运行时生成 |
| `--list-models`、`--download-model <id>`、`--list-compute` | 与 `voltip-desktop` 相同 |

| 退出码 | 含义 |
| --- | --- |
| 0 | 服务正常停止，或 `--check` 判定可以识别 |
| 1 | 服务无法启动，或无法识别 |
| 2 | 选项不能同时使用，或选项未知 |
