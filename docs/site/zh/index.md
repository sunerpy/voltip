---
layout: home
title: Voltip：Windows、macOS 和 Linux 上的语音输入
titleTemplate: false
description: 按住快捷键说话，松开后文字出现在任意应用的光标处。可以在本机识别，也可以使用你选择的云端服务，并可选 AI 润色。

hero:
  name: Voltip
  text: 按住说话，松开即输入
  tagline: 文字会出现在任意应用的光标处。识别可以在本机完成，也可以使用你选择的云端服务；AI 润色可以先把文字整理好。
  actions:
    - theme: brand
      text: 下载安装
      link: /zh/guide/install
    - theme: alt
      text: 快速开始
      link: /zh/guide/quick-start
    - theme: alt
      text: 观看视频
      link: /zh/guide/what-is-voltip#观看教程视频
    - theme: alt
      text: GitHub
      link: https://github.com/sunerpy/voltip

home:
  facts:
    - term: 适用平台
      text: Windows、macOS（Apple 芯片与 Intel 芯片）和 Linux；另有 Android 应用，可把手机当作电脑的麦克风和键盘。
    - term: 音频去向
      text: 使用本地模型时，音频不会离开这台电脑；使用云端服务时，只发送给你选择的服务。

  visual:
    home:
      light: /screens/home-zh-light.webp
      dark: /screens/home-zh-dark.webp
      width: 1440
      height: 900
      alt: Voltip 首页：快捷键、麦克风、语音模型和最近的结果。
    pill:
      - light: /screens/overlay-listening-zh-light.webp
        dark: /screens/overlay-listening-zh-dark.webp
        width: 880
        height: 70
        alt: 说话时的悬浮窗：显示输入音量和用于取消的 Esc 键。
      - light: /screens/overlay-processing-zh-light.webp
        dark: /screens/overlay-processing-zh-dark.webp
        width: 880
        height: 70
        alt: AI 润色时的悬浮窗：显示所用的预设和这一步已用的时间。
      - light: /screens/overlay-inserted-zh-light.webp
        dark: /screens/overlay-inserted-zh-dark.webp
        width: 880
        height: 70
        alt: 文字插入之后的悬浮窗：显示插入的字数和目标应用。

  index:
    title: Voltip 能做什么
    intro: 全部功能一览。标记为「开发中」的功能正在开发，尚未包含在正式版本中。
    groups:
      - name: 在任何地方听写
        items:
          - title: 任意应用里都能用的快捷键
            body: 按住 Ctrl+Alt+Space 说话，松开即可。也可以按一次开始、再按一次结束，或短按锁定，用于长段口述。
            status: available
            link: /zh/dictation/shortcuts
          - title: 单个按键或鼠标侧键
            body: 右 Ctrl、右 Alt、Mac 上的 Fn 键或鼠标侧键，都可以单独用来开始听写。
            status: available
            link: /zh/dictation/shortcuts#单个按键或鼠标键
          - title: 边说边出字
            body: 悬浮窗实时显示识别出的文字。文字可以整段插入、逐句确定，也可以边说边输入。
            status: available
            link: /zh/dictation/output
          - title: 语音编辑
            body: 选中文字，按住 Ctrl+Alt+E 说出修改要求，例如「改得正式一点」或「翻译成英文」，选中的内容会被直接替换。
            status: available
            link: /zh/dictation/voice-edit
      - name: 识别方式由你决定
        items:
          - title: 本地模型，离线可用
            body: Qwen3-ASR、SenseVoice 和 Paraformer 在本机运行。模型下载完成后，音频不再离开这台电脑。
            status: available
            link: /zh/recognition/local
          - title: 有显卡时使用 GPU
            body: Qwen3-ASR 在 Windows 和 Linux 上通过 Vulkan、在 macOS 上通过 Metal 使用显卡，没有可用的显卡时使用 CPU。
            status: available
            link: /zh/recognition/local#cpu-与-gpu
          - title: 云端服务
            body: 正式版安装包内置默认服务；也可以使用自己的密钥，改用 OpenAI、Groq、硅基流动或任意 OpenAI 兼容接口。
            status: available
            link: /zh/recognition/cloud
      - name: 让文字准确易读
        items:
          - title: AI 润色
            body: 修正标点、错别字和口头禅，不改变原意。可以使用默认服务、自己的服务商或本机的 Ollama。
            status: available
            link: /zh/recognition/polish
          - title: 词典与替换规则
            body: 让 Voltip 认识你常用的人名和术语，用字面或正则规则改写短语；中文可以统一为简体或繁体。
            status: available
            link: /zh/recognition/dictionary
          - title: 按应用切换场景
            body: 根据正在使用的应用，自动选择 AI 预设、输出方式、语言，以及给 AI 的补充要求。
            status: available
            link: /zh/recognition/scenes
          - title: AI 预设与内置场景
            body: 提供校对、提示词优化、意图整理、口语聊天、中英互译和要点纪要等预设，也可以自定义预设，可在首页、标题栏和托盘切换；另有编程开发、办公写作、即时聊天和专业领域等内置场景。
            status: available
            link: /zh/recognition/polish#预设
      - name: 手机、历史记录与长录音
        items:
          - title: 手机当麦克风和键盘
            body: 通过二维码、6 位验证码，或在同一局域网内点选，即可与 Android 手机配对。在手机上按住说话或输入文字，内容会出现在电脑的光标处。
            status: available
            link: /zh/phone/
          - title: iOS 应用
            body: 在 iOS 上同样把手机当作麦克风和键盘，尚未开始开发。
            status: planned
            link: /zh/roadmap
          - title: 历史记录，一键复制粘贴
            body: 每条结果都保存在本机，可以搜索，也可以复制或粘贴到之前使用的窗口。
            status: available
            link: /zh/dictation/history
          - title: 2 万条历史与每日统计
            body: 按日、周、月统计转录字数、修正字数和节省的时间。
            status: available
            link: /zh/dictation/history#统计
          - title: 长录音与电脑声音
            body: 单次录音最长 2 小时，可以录制麦克风、电脑声音或两者混合；录音时分段识别，可用 AI 预设处理全文或导出 SRT 字幕。
            status: available
            link: /zh/dictation/shortcuts#录音来源

  steps:
    title: 一次听写的四个步骤
    items:
      - title: 按住快捷键
        keys: [Ctrl, Alt, Space]
        body: 此时才会打开麦克风。屏幕底部的悬浮窗显示输入音量。
      - title: 说话
        body: 使用内置服务时，或安装实时识别模型后，识别出的文字会随着说话显示在悬浮窗里。
      - title: 松开按键
        body: 语音被识别，按你的词典纠正，可选经过 AI 润色，再按替换规则改写。
      - title: 文字出现在光标处
        body: Voltip 把文字粘贴到你正在使用的应用里，并在历史记录中保存一份。任何时候按 Esc 都可以取消。

  models:
    columns: [模型, 大小, 语言, 运行设备]
    rows:
      - name: Qwen3-ASR 0.6B
        note: 推荐
        size: 690 MB
        languages: 30 种语言，自动识别
        device: GPU 或 CPU
      - name: Qwen3-ASR 1.7B
        note: 最准确
        size: 1.7 GB
        languages: 30 种语言，自动识别
        device: GPU 或 CPU
      - name: SenseVoice Small
        note: 体积最小
        size: 240 MB
        languages: 中文、英文、日文、韩文、粤语
        device: CPU
      - name: Paraformer
        note: 不带标点
        size: 227 MB
        languages: 中文（含方言）与英文混说
        device: CPU
      - name: 实时识别
        note: 用于边说边预览
        size: 169 MB
        languages: 中文与英文
        device: CPU
    caption: 模型下载支持断点续传，使用前会校验 SHA-256。在 NVIDIA L40S 上实测，Qwen3-ASR 0.6B 识别一段 16 秒的中文语音，GPU 用时 0.16 秒，8 个处理器线程用时 2.87 秒。

  polish:
    heardLabel: 识别结果
    heard: 嗯那个我觉得我们周五把这个修复发出去然后下周再更新一下文档吧
    typedLabel: 插入到光标处
    typed: 我觉得我们周五把这个修复发出去，下周再更新一下文档。
    caption: AI 润色效果示例：去掉口头禅，补全标点，措辞保持不变。

  phone:
    items:
      - title: 配对一次
        body: 扫描电脑上的二维码、输入 6 位验证码，或在手机的「附近的电脑」里点选这台电脑。随后两台设备会显示同一组安全码，由你核对确认。
      - title: 在手机上按住说话
        body: 音频经 Opus 压缩、端到端加密后实时传到电脑，由电脑完成识别，并把文字插入电脑的光标处。
      - title: 发送文字或剪贴板
        body: 在手机上输入文字或发送手机剪贴板，电脑会像处理听写结果一样把它插入光标处。
      - title: 不受网络限制
        body: 在同一局域网内直接连接；不在同一网络时，可选的中继负责转发加密数据，中继无法读取其中的内容。

  platforms:
    title: 支持的平台
    intro: Voltip 在三种桌面系统上是同一个应用，各平台之间的差异见「各平台说明」。
    columns: [平台, 安装包, 快捷键与单键, 本地识别, GPU]
    rows:
      - name: Windows 10 和 11
        status: available
        cells:
          - 按用户安装的安装包或便携版 zip，x64
          - 都支持
          - 支持
          - Vulkan
      - name: macOS 11 及以上
        status: available
        cells:
          - Apple 芯片与 Intel 芯片各一个 dmg
          - 都支持，需要辅助功能权限
          - 支持
          - Metal
      - name: Linux
        status: available
        cells:
          - .deb 或 AppImage，x64；支持 X11 和 Wayland
          - X11 上都支持；Wayland 上通过系统快捷键运行切换命令
          - 支持
          - Vulkan
      - name: Android
        status: available
        cells:
          - APK，Android 8.0 及以上，64 位 Arm
          - 在应用里按住说话
          - 由配对的电脑完成
          - —
    note: iOS 尚未开始开发。暂不提供 Windows ARM 和 Linux ARM 的安装包。

  privacy:
    title: 哪些数据会离开这台电脑
    intro: 取决于你选择的服务。首页和「语音模型」页始终显示当前使用的是哪一项。
    sendsLabel: 发送
    modes:
      - name: 本地识别
        sends: 不发送任何内容
        detail: 识别在 Voltip 内完成，使用磁盘上的模型文件。唯一的网络访问是下载模型，下载内容会按 SHA-256 校验。
      - name: 云端识别
        sends: 音频，发送给你选择的服务
        detail: 录音发送给内置服务或你配置的服务商，同时附带你的密钥；如果设置了词典，词条也会作为识别提示一并发送。密钥保存在系统钥匙串中。
      - name: AI 润色与语音编辑
        sends: 文字，发送给你选择的 AI 服务
        detail: 发送识别出的文字；语音编辑时发送选中的文字和你的指令。默认附带当前应用的名称，可以关闭；窗口标题仅在你开启后才会发送。不会发送音频。

  roadmap:
    title: 接下来的计划
    intro: 按预计顺序列出，不承诺日期；每项功能发布后，本站会随之更新。
    items:
      - title: Google Play 上架
        status: building
        body: Android 应用已在发布页面提供。Google Play 上架正在准备中，按 Google 的要求需要先完成封闭测试。
      - title: 其他云端服务的流式识别
        status: planned
        body: 使用内置服务时，边说边出字由内置服务提供；其他云端服务的流式识别结果尚未支持。
      - title: 更多本地模型
        status: planned
        body: 包括 Whisper 等模型，以及在 AMD 和 Intel 显卡上的实测。
      - title: iOS 应用
        status: planned
        body: 在 iOS 上同样把手机当作麦克风和键盘，尚未开始开发。
    notPlanned:
      title: 刻意不做的功能
      items:
        - 一直监听，或检测到静音就自动停止。麦克风只在录音时打开。
        - 说话人分离、会议检测和批量转写文件。
        - 纯浏览器版本。全局快捷键、向其他应用输入文字和本地模型都需要原生应用。
        - 把历史记录、词典和规则同步到手机。手机只负责麦克风和键盘，处理都在电脑上完成。
---

<HomeIndex />

<HomeSteps />

<SplitBlock proof="models">

## 在本机识别，或使用云端服务

下载一次模型，听写即可离线使用。Qwen3-ASR 可识别 30 种语言并自带标点；SenseVoice 和 Paraformer 体积小，在普通处理器上也很快。电脑有显卡时，Qwen3-ASR 会通过 Vulkan 或 Metal 使用显卡，否则使用处理器。

正式版安装包还内置了默认的云端服务，无需任何设置即可开始听写。你也可以随时改用 OpenAI、Groq、硅基流动或任意 OpenAI 兼容接口，并使用自己的密钥。

[本地识别](/zh/recognition/local) · [云端服务](/zh/recognition/cloud)

</SplitBlock>

<SplitBlock proof="polish" flip>

## 让文字符合你的原意

识别负责听出每一个字，后续步骤负责让文字准确易读：词典纠正识别器容易听错的人名和术语，AI 润色去掉口头禅并补全标点，替换规则按你的设定改写短语，场景再根据当前使用的应用调整这些处理。

预设决定 AI 润色的处理方式：默认为校对，也可以在首页、标题栏或托盘中选择提示词优化、意图整理、口语聊天、中英互译和要点纪要，或使用你自己编写的预设。

[AI 润色](/zh/recognition/polish) · [词典与替换规则](/zh/recognition/dictionary) · [场景](/zh/recognition/scenes)

</SplitBlock>

<SplitBlock proof="phone">

## 手机当麦克风和键盘

<StatusTag status="available" />

把 Android 手机与电脑配对后，在手机上按住说话：电脑使用自己的识别设置处理语音，并把文字插入电脑的光标处。手机也可以发送输入的文字或剪贴板内容。

没有在线的电脑时，手机也可以单独识别，结果自动复制到手机剪贴板。

Android 应用以 APK 形式在发布页面提供。[安装方法](/zh/guide/install#android) · [了解手机端](/zh/phone/)

</SplitBlock>

<HomePlatforms />

<HomePrivacy />

## 安装

一条命令即可为这台电脑选择合适的安装包，按发布附带的 `SHA256SUMS` 校验后再安装。

::: code-group

```powershell [Windows]
irm https://raw.githubusercontent.com/sunerpy/voltip/main/scripts/install.ps1 | iex
```

```bash [Linux 和 macOS]
curl -fsSL https://raw.githubusercontent.com/sunerpy/voltip/main/scripts/install.sh | sh
```

:::

各平台的安装包、校验值和构建证明都在[发布页面](https://github.com/sunerpy/voltip/releases)。[安装指南](/zh/guide/install)介绍了各个平台的安装方法，包括在 Mac 上首次打开时的步骤。

<HomeRoadmap />
