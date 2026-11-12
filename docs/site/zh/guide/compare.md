---
title: 开源的 Wispr Flow 替代：支持 Windows、macOS、Linux 和 Android
description: 从支持的平台、能否在设备上识别、是否开源和价格几个方面，对比 Voltip 与 Wispr Flow、Superwhisper、Typeless、Handy、OpenWhispr 和 VoiceInk。
---

# Voltip 与其他语音输入工具的对比

本页帮助你按可以核实的事实对比 Voltip 与其他语音输入和听写工具：各自支持的平台、语音识别能否在你自己的设备上完成、是否公开源代码，以及价格。

## 概览

| 应用 | 平台 | 在设备上识别 | 源代码 | 价格 |
| --- | --- | --- | --- | --- |
| **Voltip** | Windows、macOS、Linux、Android | 支持，在电脑上使用本地模型；Android 版使用云端服务 | 公开，AGPL-3.0 | 免费 |
| Wispr Flow | macOS、Windows、iOS、Android | 不支持，转写在云端进行 | 未公开 | 免费版每周有字数限制；另有付费的 Pro |
| Superwhisper | macOS、Windows、Linux、iOS、Android | 支持，使用离线模型 | 未公开 | 有免费版；另有付费的 Pro |
| Typeless | macOS、Windows、Linux、iOS、Android | 不支持，音频在 Typeless 的云端服务器上处理 | 未公开 | 免费版每周有字数限制；另有付费的 Pro |
| Handy | Windows、macOS、Linux | 支持，可离线使用 | 公开，MIT | 免费 |
| OpenWhispr | macOS、Windows、Linux；Pro 包含 iPhone 和 iPad 版 | 支持，使用本地模型 | 公开，MIT | 有免费版；另有付费的 Pro |
| VoiceInk | macOS（Apple 芯片）、iOS | 支持，转写在设备上完成 | 公开，GPL-3.0 | 一次性购买，可免费试用 |

「未公开」指该应用的网站没有提供应用本身的源代码链接。除 Voltip 外，各行内容来自[来源](#来源)中列出的页面，读取日期为 2026 年 11 月 12 日；功能和价格可能变化，以各厂商的页面为准。

## 版本与价格

价格为各厂商网站标注的美元价格，税费和地区价格可能不同。

- **Voltip**：免费。内置服务免费提供识别和 AI 润色，由所有用户共用；自己的云端服务商按其价格收费，本地模型不产生费用。
- **Wispr Flow**：免费版每周有字数限制，Mac 和 Windows 上额度为 2,000 词，最多 5,000 词。Pro 每月 15 美元，或每年 144 美元。
- **Superwhisper**：有免费版；Pro 每月 8.49 美元、每年 84.99 美元，或终身授权 249.99 美元。
- **Typeless**：免费版每周可用的字数有限。Pro 按年付费每月 12 美元，按月付费每月 30 美元。
- **Handy**：免费。
- **OpenWhispr**：本地听写免费且不限量，免费版每周另含 2,000 词的云端转写。Pro 每位用户每年 80 美元。
- **VoiceInk**：一次性购买，一台 Mac 25 美元起，可免费试用。

## Voltip 的特点

- **一个应用覆盖四个系统**：Windows、macOS、Linux（X11 和 Wayland）和 Android，均在同一个[发布页面](https://github.com/sunerpy/voltip/releases)提供。iOS 版计划在后续发布。
- **在电脑上离线语音转文字**：Qwen3-ASR、SenseVoice 和 Paraformer 可以在 Windows、macOS 和 Linux 上运行，有显卡时使用显卡；音频不离开这台电脑。见[本地识别](/zh/recognition/local)。
- **无需密钥即可开始**：内置服务免费提供识别和 AI 润色，也可以随时改用自己的服务商。见[云端服务](/zh/recognition/cloud)。
- **手机作为麦克风和键盘**：Android 手机通过端到端加密的连接把说的话发送到电脑的光标处，也可以独立完成识别。见[手机](/zh/phone/)。
- **按 AGPL-3.0 开源**：源代码在 [GitHub](https://github.com/sunerpy/voltip) 上。

[Voltip 是什么](/zh/guide/what-is-voltip)介绍一次听写的过程，[安装](/zh/guide/install)列出各系统的安装包。

## 来源

读取日期为 2026 年 11 月 12 日。如果这里的信息已经过时，请[反馈给我们](https://github.com/sunerpy/voltip/issues)。

- Wispr Flow：[价格](https://wisprflow.ai/pricing)、[Flow plans and what's included](https://docs.wisprflow.ai/articles/9559327591-flow-plans-and-what-s-included)、[数据控制](https://wisprflow.ai/data-controls)（"Transcription always occurs on the cloud"）。
- Superwhisper：[首页与价格](https://superwhisper.com/)、[Superwhisper for Linux](https://superwhisper.com/linux)。
- Typeless：[价格](https://www.typeless.com/pricing)、[数据控制](https://www.typeless.com/data-controls)（"processed in real time on our cloud servers"）。
- Handy：[handy.computer](https://handy.computer/)、[代码仓库与许可证](https://github.com/cjpais/Handy)。
- OpenWhispr：[价格](https://openwhispr.com/pricing)、[代码仓库与许可证](https://github.com/OpenWhispr/openwhispr)。
- VoiceInk：[价格](https://tryvoiceink.com/pricing)、[代码仓库与许可证](https://github.com/Beingpax/VoiceInk)。
- Voltip：[代码仓库](https://github.com/sunerpy/voltip)、[许可证](https://github.com/sunerpy/voltip/blob/main/LICENSE)和本网站。
