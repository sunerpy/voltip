---
layout: home
markdownStyles: false
title: Voltip：Windows、macOS、Linux 和 Android 上的语音输入
titleTemplate: false
description: 按住快捷键说话，松开后文字出现在任意应用的光标处。可以在本机识别，也可以使用你选择的云端服务，并可选 AI 润色。

hero:
  text: 按住说话，<br>松开即输入
  tagline: 文字会出现在任意应用的光标处。识别可以在本机完成，也可以使用你选择的云端服务；AI 润色可以先把文字整理好。
  actions:
    - theme: brand
      text: 下载安装
      link: /zh/guide/install
    - theme: alt
      text: 在线体验
      link: /zh/guide/try

home:
  checks:
    - Windows、macOS、Linux 和 Android
    - 使用本地模型时，音频不离开这台电脑
    - 开源，AGPL-3.0

  scene:
    steps:
      - 按住 Ctrl+Alt+Space 说话，悬浮窗显示识别出的原话
      - 松开按键，文字经过校对后插入光标处
    live:
      src: /screens/hero-live-zh.webp
      width: 820
      height: 98
      alt: 说话时的悬浮窗：那个就是评审会定在周一上午十点，呃，不对，应该是下午两点。
    label: 插入光标处的文字
    result: 评审会定在周一下午两点，请大家带上最新的数据。
    inserted:
      src: /screens/hero-inserted-zh.webp
      width: 418
      height: 72
      alt: 插入之后的悬浮窗：已插入 23 字，粘贴。
    caption: 一次实际听写，使用内置服务和「校对」预设。悬浮窗为录屏截图。

  try:
    title: 无需安装，在浏览器里体验 Voltip
    body: 说一句话，几秒后就能看到识别出的文字和 AI 润色后的结果。
    heard: ~~嗯那个~~我们周二开个会吧
    polished: 我们周二开个会吧。
    action:
      text: 开始体验
      link: /zh/guide/try

  pairs:
    title: 口头禅和改口，插入之前就整理好
    intro: AI 润色删去口头禅、补全标点、采纳改口，措辞保持不变；不需要时可以关闭。
    labels:
      said: 说的
      inserted: 插入的
      edit: 选中文字，按住 Ctrl+Alt+E 说
      selection: 选中的文字
      replaced: 替换为
      actual: 实际输出
      example: 示例
    items:
      - kind: dictation
        source: actual
        said: ~~那个就是~~评审会定在周一~~上午十点，呃，不对，应该是~~下午两点，请大家带上最新的数据。
        result: 评审会定在周一下午两点，请大家带上最新的数据。
      - kind: dictation
        source: example
        said: ~~嗯那个~~我觉得我们周五把这个修复发出去~~然后~~下周再更新一下文档~~吧~~
        result: 我觉得我们周五把这个修复发出去，下周再更新一下文档。
      - kind: edit
        source: actual
        said: 翻译成英文。
        selection: 评审会定在周一下午两点，请大家带上最新的数据。
        result: The review meeting is scheduled for 2:00 PM next Monday. Please bring the latest data.
    note: 「实际输出」取自 Voltip 的录屏；「示例」是说明用的文字，实际结果取决于所选的 AI 模型。

  presets:
    title: "同一段话，\n按场合选择预设"
    intro: 预设决定 AI 润色怎样整理文字。内置 8 个预设，提示词全部公开，也可以复制为自定义预设后修改。
    points:
      - 默认使用「校对」，可以在首页、托盘或「AI 润色」开关旁切换
      - 开启场景后，按正在使用的应用自动选择预设
    saidLabel: 说的
    said: 呃那个就是周四的发布先推迟到下周一吧，然后测试那边还有两个问题没修完，修完了我再通知大家
    outputs:
      - name: 校对
        desc: 修正错字和标点，删去口头禅
        text: 周四的发布先推迟到下周一吧，然后测试那边还有两个问题没修完，修完了我再通知大家。
      - name: 意图整理
        desc: 采纳改口，把多个要点整理成列表
        text: "- 周四的发布推迟到下周一；\n- 测试那边还有两个问题待修复，修复后通知大家。"
      - name: 书面语
        desc: 改写为完整、规范的书面语
        text: 周四的发布推迟至下周一。测试方面尚有两个问题待修复，修复完成后我会通知大家。
      - name: 中英互译
        desc: 先修正识别错误，再中英互译
        text: Let's push Thursday's release to next Monday. Testing still has two open issues, and I'll let everyone know once they're fixed.
    note: 示例输出，实际结果取决于所选的 AI 模型。另有提示词优化、口语聊天、要点纪要和只加标点。
    scenes:
      title: 场景：按应用切换预设和补充要求
      note: 内置场景开启后才生效；也可以为任意应用新建场景。
      items:
        - name: 编程开发
          body: VS Code、Cursor、Kiro、JetBrains 系列和终端。代码、命令、路径和标识符保持原样。
          preset: 校对
        - name: 办公写作
          body: Outlook、Word、WPS、OneNote、Notion 等应用中的邮件和文档。
          preset: 书面语
        - name: 即时聊天
          body: 微信、QQ、Slack、Teams、钉钉、飞书、Telegram、Discord 等聊天应用。
          preset: 口语聊天
        - name: 法律、医疗、金融、学术
          body: 严格校对，专业术语保持原样。添加你使用的应用后生效。
          preset: 校对

  modes:
    title: 识别在哪里完成，由你选择
    intro: 录音发送到哪里、由谁的模型识别，可以在「语音模型」页随时切换。
    scope: 这台电脑
    items:
      - icon: cpu
        label: 本机识别
        title: 下载一次，离线可用
        flow: { from: 你说话, to: 本地模型, icon: cpu, local: true }
        assure: 音频不离开这台电脑
        specs:
          - { term: 费用, text: 免费 }
          - { term: 准备, text: 下载一个模型，最小 227 MB }
          - { term: 显卡, text: Qwen3-ASR 在 Windows 和 Linux 上使用 Vulkan，在 macOS 上使用 Metal；没有显卡时使用处理器 }
        chipsLabel: 可选模型
        chips:
          - name: Qwen3-ASR 0.6B
          - name: Qwen3-ASR 1.7B
          - name: SenseVoice Small
          - name: Paraformer
      - icon: cloud
        label: 内置服务
        badge: 默认
        title: 安装后即可听写
        flow: { from: 你说话, to: 内置服务, icon: cloud, local: false }
        assure: 服务器不记录录音和文字
        specs:
          - { term: 费用, text: 免费，所有用户共用，按地址限制次数 }
          - { term: 准备, text: 不需要密钥 }
          - { term: 预览, text: 说话时悬浮窗显示识别出的文字 }
        chipsLabel: AI 润色模型
        chips:
          - name: Qwen3.8 27B
          - name: GPT-OSS 120B
          - name: GPT-OSS 20B
      - icon: key
        label: 你的服务商
        title: 使用自己的密钥
        flow: { from: 你说话, to: 服务商, icon: server, local: false }
        assure: 录音直接发送给你选择的服务商
        specs:
          - { term: 费用, text: 按服务商的价格，部分有免费额度 }
          - { term: 准备, text: 服务商的 API 密钥 }
          - { term: 密钥, text: 保存在系统钥匙串中 }
        chipsLabel: 识别服务商
        chips:
          - { name: OpenAI, mark: openai }
          - { name: Groq, mark: groq }
          - { name: 硅基流动, mark: siliconflow }
          - { name: 阿里云百炼, mark: aliyun }
          - { name: OpenAI 兼容接口, icon: plug }
    polish:
      title: AI 润色
      body: 识别之后由大模型整理文字。可以使用内置服务、你的服务商，或在本机运行的 Ollama。
      marks:
        - { name: OpenAI, mark: openai }
        - { name: Groq, mark: groq }
        - { name: Google AI Studio, mark: google }
        - { name: 硅基流动, mark: siliconflow }
        - { name: 阿里云百炼, mark: aliyun }
        - { name: DeepSeek, mark: deepseek }
        - { name: Ollama, mark: ollama }
    note: 在 NVIDIA L40S 上实测，Qwen3-ASR 0.6B 识别一段 16 秒的中文语音，GPU 用时 0.16 秒，8 个处理器线程用时 2.87 秒。详见[本地识别](/zh/recognition/local)和[云端服务](/zh/recognition/cloud)。

  phone:
    title: "手机当作电脑的\n麦克风和键盘"
    intro: 把 Android 手机与电脑配对后，在手机上按住说话：电脑用自己的识别设置处理语音，并把文字插入电脑的光标处。没有在线的电脑时，手机也可以单独识别。
    items:
      - icon: qr
        title: 配对一次
        body: 扫描电脑上的二维码，或输入 6 位验证码，再核对两台设备显示的安全码。
      - icon: mic
        title: 在手机上按住说话
        body: 音频经 Opus 压缩、端到端加密后传到电脑，由电脑识别并插入光标处。
      - icon: keyboard
        title: 发送文字或剪贴板
        body: 手机上输入的文字和剪贴板内容，电脑会像听写结果一样插入光标处。
      - icon: history
        title: 在手机上查看电脑的记录
        body: 电脑的历史记录和设置同步到手机，在手机上只能查看。
      - icon: relay
        title: 不在同一网络也能连接
        body: 两台设备经中继连接，中继只转发加密数据；网络切换后自动重新连接。
    figure:
      back:
        src: /screens/phone-pair-zh-light.webp
        dark: /screens/phone-pair-zh-dark.webp
        width: 1482
        height: 584
        alt: 电脑上的「手机」页：开始配对，生成二维码和 6 位验证码。
      front:
        src: /screens/phone-mic-zh-light.webp
        dark: /screens/phone-mic-zh-dark.webp
        width: 760
        height: 465
        alt: 「手机麦克风」：配对的手机在线，在手机上按住说话。
      caption: 电脑上的「手机」页：开始配对，以及配对手机的麦克风。
    links: "[了解手机端](/zh/phone/) · [安装 Android 应用](/zh/guide/install#android)"

  platforms:
    title: "Windows、macOS、Linux，\n以及 Android"
    intro: 桌面版在三种系统上是同一个应用；Android 应用把手机当作电脑的麦克风和键盘。
    cards:
      - name: Windows 10 和 11
        status: available
        specs:
          - { term: 安装包, text: 按用户安装的安装包或便携版 zip，x64 }
          - { term: 显卡加速, text: Vulkan }
      - name: macOS 11 及以上
        status: available
        specs:
          - { term: 安装包, text: Apple 芯片与 Intel 芯片各一个 dmg }
          - { term: 显卡加速, text: Metal }
      - name: Linux
        status: available
        specs:
          - { term: 安装包, text: .deb 或 AppImage，x64；支持 X11 和 Wayland }
          - { term: 显卡加速, text: Vulkan }
      - name: Android 8.0 及以上
        status: available
        specs:
          - { term: 安装包, text: APK，64 位 Arm }
          - { term: 识别, text: 由配对的电脑完成，也可以单独使用 }
    install: 一条命令为这台电脑选择合适的安装包，按发布附带的 SHA256SUMS 校验后安装。各平台的安装包和校验值都在[发布页面](https://github.com/sunerpy/voltip/releases)，[安装指南](/zh/guide/install)介绍了各平台的安装方法。
    note: iOS 版计划在后续发布。暂不提供 Windows ARM 和 Linux ARM 的安装包。

  faq:
    title: 常见问题
    intro: 更多问题和故障处理，见[常见问题](/zh/reference/faq)页。
    items:
      - q: 没有网络时能用吗？
        a: 可以，使用[本地模型](/zh/recognition/local)即可。下载一次后，识别可以离线进行。AI 润色需要服务，除非你在本机运行 Ollama。
      - q: Voltip 会一直监听吗？
        a: 不会。麦克风在你按下快捷键时打开，录音结束后关闭。
      - q: 录音和文字会发送到哪里？
        a: 取决于你选择的服务。本地识别不发送任何内容；云端识别把录音发送给内置服务或你配置的服务商；AI 润色只发送文字，不发送音频。详见[隐私](/zh/privacy)。
      - q: 内置服务需要付费或填写密钥吗？
        a: 不需要。内置服务由 Voltip 项目运营，所有用户共用，并按地址限制请求次数。需要稳定使用 AI 润色时，可以改用自己的服务商。
      - q: 网络需要经过代理怎么办？
        a: 在「设置 → 网络」中选择代理方式：默认跟随系统的代理设置，也可以手动填写代理服务器。识别、润色、模型下载、更新和与手机的连接都按这个设置连接。
      - q: 支持 iPhone 吗？
        a: iOS 版计划在后续发布，目前提供 Android 应用。

  closer:
    title: 按住快捷键，开始说话
    body: 免费下载，支持 Windows、macOS、Linux 和 Android。
    actions:
      - { text: 下载安装, link: /zh/guide/install, theme: brand, icon: download }
      - { text: GitHub, link: "https://github.com/sunerpy/voltip", theme: alt, icon: github }
---

<HomePairs />

<HomePresets />

<HomeModes />

<HomePhone />

<HomePlatforms>

::: code-group

```powershell [Windows]
irm https://raw.githubusercontent.com/sunerpy/voltip/main/scripts/install.ps1 | iex
```

```bash [Linux 和 macOS]
curl -fsSL https://raw.githubusercontent.com/sunerpy/voltip/main/scripts/install.sh | sh
```

:::

</HomePlatforms>

<HomeFaq />

<HomeCloser />
