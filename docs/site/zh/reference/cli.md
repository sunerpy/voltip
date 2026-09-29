---
description: voltip-desktop 的命令行选项，用于控制正在运行的应用，以及在不打开窗口的情况下识别音频文件。
---

# 命令行

Voltip 的程序 `voltip-desktop`（Windows 上为 `voltip-desktop.exe`）接受的选项有两种用途：控制已经在运行的 Voltip，以及在不打开窗口的情况下识别音频文件。

## 控制正在运行的应用

这些选项会转交给已经在运行的 Voltip；没有运行中的 Voltip 时，程序正常启动。

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
