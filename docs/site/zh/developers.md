---
description: 从源码构建 Voltip、参与贡献，以及说明其工作原理的设计文档。
---

# 开发者

Voltip 基于 [Apache License 2.0](https://github.com/sunerpy/voltip/blob/main/LICENSE) 开源，代码、issue 和发布版本都在 [GitHub](https://github.com/sunerpy/voltip) 上。

## 构成

- 一个 Rust workspace 承担全部原生能力：全局快捷键、音频、本地与云端识别、AI 润色、文字插入、配对，以及与手机之间的加密连接。
- 桌面端和 Android 端是两个 Tauri 2 应用，共用 React 界面组件。界面只负责显示状态和发送命令。
- 本地识别使用 transcribe.cpp 运行 Qwen3-ASR，使用 sherpa-onnx 运行 SenseVoice、Paraformer 和实时识别模型。

## 从源码构建

需要 Rust（版本见 `rust-toolchain.toml`）、Node 22 和 pnpm、`cmake`；在 Linux 上还需要 Tauri 所需的系统软件包，完整列表见 CI 工作流。

```bash
pnpm install --frozen-lockfile
make desktop-dev      # 带热重载运行桌面端
make verify           # CI 运行的全部检查
make help             # 其他目标
```

构建不含默认服务的安装包：

```bash
VOLTIP_ALLOW_NO_BUILTIN_ENGINES=1 make linux-x64     # dist/linux-x64 下的 .deb、.rpm 和 .AppImage
VOLTIP_ALLOW_NO_BUILTIN_ENGINES=1 make windows-x64   # 在 Linux 上交叉构建安装包和便携版 zip
```

从源码构建的版本没有内置服务，需要在应用中选择服务商或本地模型。如果要内置自己的默认值，把 `.env.build.example` 复制为 `.env.build` 并填写，各项的含义见 `.github/README-secrets.md`。

## 参与贡献

欢迎提交 issue 和 pull request。[AGENTS.md](https://github.com/sunerpy/voltip/blob/main/AGENTS.md) 说明了代码结构和需要遵守的约定，[CONTRIBUTING.md](https://github.com/sunerpy/voltip/blob/main/CONTRIBUTING.md) 说明了工作流程，安全问题请按 [SECURITY.md](https://github.com/sunerpy/voltip/blob/main/SECURITY.md) 私下报告。

本站的页面写在仓库的 `docs/site/` 下，包括英文和中文两个版本。修改了应用行为的 pull request，需要同时更新对应的页面。

## 设计文档

设计文档用中文写成，是代码遵循的契约：

| 文档 | 内容 |
| --- | --- |
| [架构](/zh/dev/architecture) | Rust 核心、两个 Tauri 应用，以及它们之间的通信方式 |
| [听写流水线](/zh/dev/dictation) | 录音、识别、模型、快捷键和文字插入 |
| [界面与 IPC 契约](/zh/dev/frontend) | 前端界面，以及它与 Rust 之间交换的消息 |
| [配对](/zh/dev/pairing) | 配对、局域网发现和连接自检 |
| [线协议](/zh/dev/protocol) | 设备之间的消息格式 |
| [威胁模型](/zh/dev/threat-model) | 加密连接能防范哪些威胁 |
| [状态机](/zh/dev/state-machines) | 配对和连接的各个状态 |
| [应用内反馈](/zh/dev/feedback) | 反馈对话框发送的内容，以及接收它的服务 |

另有两份文档只放在 GitHub 上：[acceptance.md](https://github.com/sunerpy/voltip/blob/main/docs/acceptance.md) 把每个功能对应到实现代码和测试，[runbook.md](https://github.com/sunerpy/voltip/blob/main/docs/runbook.md) 介绍构建、打包、中继和发布。
