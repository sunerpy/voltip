---
layout: home
markdownStyles: false
title: Voltip：Windows、macOS、Linux 和 Android 上的语音输入
titleTemplate: false
description: 按住快捷键说话，松开后文字出现在任意应用的光标处。可以在本机识别，也可以使用你选择的云端服务，并可选 AI 润色。

hero:
  text: 按住说话，<br>松开即输入
  tagline: 开源的语音输入工具，文字出现在任意应用的光标处。识别可以用本机模型离线完成，也可以交给内置服务或你自己的服务商；AI 润色按你选择的预设整理措辞。
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
    - 内置服务免费，不需要密钥
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

  flow:
    title: 按下快捷键之后，文字依次经过这几步
    intro: 每一步都可以单独设置，AI 润色也可以关闭。词典、AI 润色或替换规则出错时，听写会用前一步的文字完成。
    steps:
      - icon: mic
        name: 录音
        text: 按住说话，或按一下开始、再按一下结束；可以录麦克风、电脑正在播放的声音，或两者混合。
      - icon: audio
        name: 语音识别
        text: 使用本机模型、内置服务或你的服务商，整段识别或边说边识别。词典的正确写法和场景的术语会作为热词随录音发送，让云端识别保持这些写法；本地模型不使用热词。
      - icon: book
        name: 词典
        text: 识别之后，把曾听成的写法替换为正确写法，本地模型和云端识别都适用。
      - icon: wand
        name: AI 润色
        tag: 可关闭
        text: 按当前的预设或场景整理文字：删去口头禅，补全标点，采纳改口。
      - icon: replace
        name: 替换规则
        text: 按列表顺序改写最终文字，支持字面匹配和正则表达式。
      - icon: cursor
        name: 插入
        text: 粘贴到光标处后恢复剪贴板原来的内容，并在本机的历史记录中保存一份。
    note: 详细过程见 [Voltip 是什么](/zh/guide/what-is-voltip)。

  pairs:
    title: 插入文字之前，先整理口头禅和改口
    intro: AI 润色删去口头禅、补全标点、采纳改口，措辞保持不变；不需要时可以关闭。
    labels:
      said: 识别出的原话
      inserted: 插入的文字
      edit: 选中文字后，按住 Ctrl+Alt+E 说
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
    title: 选一个预设，决定文字怎样整理
    intro: 内置 8 个预设，每个预设的提示词都可以查看，也可以复制为自定义预设后修改。
    points:
      - 默认使用「校对」，可以在首页、托盘或「AI 润色」开关旁切换
      - 开启场景后，按正在使用的应用自动选择预设
    saidLabel: 识别出的原话
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
      labels:
        preset: 预设
        instruction: 补充要求
      items:
        - name: 编程开发
          preset: 校对
          apps:
            - { name: VS Code, mark: vscode }
            - { name: Cursor, mark: cursor }
            - { name: Kiro, mark: kiro }
            - { name: JetBrains, mark: jetbrains }
            - { name: iTerm2, mark: iterm2 }
            - { name: Warp, mark: warp }
            - { name: Ghostty, mark: ghostty }
            - { name: Alacritty, mark: alacritty }
            - { name: WezTerm, mark: wezterm }
          more: 以及 Windows Terminal、PowerShell 和各系统自带的终端。
          tools:
            label: 终端中的编程助手
            apps:
              - { name: Codex, mark: codex }
              - { name: OpenCode, mark: opencode }
              - { name: Pi Agent, mark: pi }
            more: 以及 Claude Code 等其他在终端中运行的编程助手。
          instruction: 保留代码、命令、路径、英文标识符和 Markdown，不把技术词翻译成中文。
        - name: 办公写作
          preset: 书面语
          apps:
            - { name: Notion, mark: notion }
            - { name: Obsidian, mark: obsidian }
            - { name: Thunderbird, mark: thunderbird }
            - { name: LibreOffice Writer, mark: libreoffice-writer }
          more: 以及 Outlook、Word、WPS、OneNote、Typora，和 Mac 上的「邮件」与 Pages。
          instruction: 邮件和文档：句子完整，自然分段，不编造称呼和事实。
        - name: 即时聊天
          preset: 口语聊天
          apps:
            - { name: 微信, mark: wechat }
            - { name: QQ, mark: qq }
            - { name: Telegram, mark: telegram }
            - { name: Discord, mark: discord }
          more: 以及企业微信、Slack、Teams、钉钉、飞书、Lark，和 Mac 上的「信息」。
          body: 这个场景没有补充要求，由「口语聊天」预设整理：口语化的短句，句末不加句号。
        - name: 法律、医疗、金融、学术
          preset: 校对
          apps:
            - { name: 法律, icon: scale }
            - { name: 医疗, icon: stethoscope }
            - { name: 金融, icon: chart }
            - { name: 学术, icon: cap }
          more: 四个领域各是一个场景，添加你使用的应用后生效。
          instruction: 严格校对，保持专业术语原样；数字、单位、日期写规范。每个领域另有一句，例如法律文本的法条、案号和当事人名称照原样保留。
  modes:
    title: 三种识别方式
    intro: 本机、内置服务和你的服务商，可以在「语音模型」页随时切换。
    items:
      - icon: cpu
        label: 本机识别
        title: 使用下载到本机的模型
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
        title: 由 Voltip 项目运营
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
        title: 使用你自己的 API 密钥
        assure: 录音只发给你选择的服务商
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
        - { name: Kimi, mark: kimi }
        - { name: 智谱 GLM, mark: glm }
        - { name: 火山方舟, mark: volcengine }
        - { name: 腾讯混元, mark: hunyuan }
        - { name: 小米 MiMo, mark: mimo }
        - { name: OpenRouter, mark: openrouter }
        - { name: Ollama, mark: ollama }
    note: 在 NVIDIA L40S 上实测，Qwen3-ASR 0.6B 识别一段 16 秒的中文语音，GPU 用时 0.16 秒，8 个处理器线程用时 2.87 秒。详见[本地识别](/zh/recognition/local)和[云端服务](/zh/recognition/cloud)。

  more:
    title: 按你的工作方式调整
    intro: 以下功能都在桌面版中，默认关闭的可以在设置中开启。
    items:
      - title: 词典与替换规则
        wide: true
        body: 把识别器容易听错的词登记一次，以后都按正确写法插入；正确写法还会作为提示交给云端识别和 AI 润色。替换规则在最后改写文字，可以使用正则表达式。在历史记录中选中听错的词，就能直接加入词典或新建规则。
        swap: { from: 酷伯 control, to: kubectl }
        guide: "[词典与替换规则](/zh/recognition/dictionary)"
      - title: 用单个按键开始听写
        body: 除了组合键，也可以用右 Ctrl、右 Alt、Mac 上的 Fn 键，或鼠标的中键、侧键开始听写；按住时再按其他键会取消这次录音，右 Ctrl + C 照常可用。
        keys: [右 Ctrl, 右 Alt, Fn]
        guide: "[快捷键与录音方式](/zh/dictation/shortcuts)"
      - title: 电脑声音与长录音
        body: 可以录麦克风、电脑正在播放的声音，或两者混合，混合时去掉扬声器的回声。单次录音最长 2 小时，可以导出 SRT 字幕和文本。
        chips: [麦克风, 电脑声音, 混合]
        guide: "[录音来源](/zh/dictation/shortcuts#录音来源)"
      - title: 额度用完后改用其他模型
        wide: true
        body: 识别和 AI 润色各有一张候补模型列表，最多 8 个模型。所选模型的额度用完时，按顺序改用下一个，这次听写照常完成；网络中断、密钥错误等其他错误照常报告，不会切换。
        chain: [Qwen3.8 27B, GPT-OSS 120B, GPT-OSS 20B]
        guide: "[额度用完后改用其他模型](/zh/recognition/cloud#额度用完后改用其他模型)"
      - title: AI 工具接入
        body: Claude Code、Codex 等 AI 工具可以通过 MCP 读取历史记录、词典和规则，汇总反复出现的识别错误，在你的授权下添加词条和规则。每次修改都可以撤销。
        chips: [Claude Code, Codex, MCP]
        guide: "[AI 工具接入](/zh/recognition/mcp)"
      - title: 本机服务
        wide: true
        body: 其他程序通过 OpenAI 兼容接口发送录音，得到经过同样识别、词典、预设和规则处理的文字。可以在应用中开启，也可以在没有桌面的 Linux 服务器上运行 voltip-server；默认只接受来自本机的连接，每个请求都须携带访问令牌。
        code: voltip-server --scene coding --preset prompt
        guide: "[本机服务](/zh/recognition/service)"

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
      - q: 必须一直按住快捷键吗？
        a: 不必。可以改为按一下开始、再按一下结束，或在「按住或按一下」方式中短按锁定录音。录音达到设置的最长时长时自动停止；Voltip 没有唤醒词，也不会因为检测到静音而停止。见[快捷键与录音方式](/zh/dictation/shortcuts)。
      - q: 安装包经过签名吗？
        a: Windows 安装包尚未进行代码签名，首次启动时 SmartScreen 会提示；macOS 版用项目自己的证书签名，但未经 Apple 公证，首次打开时需要确认一次。安装脚本会按 SHA256SUMS 校验下载的文件，见[安装](/zh/guide/install)。
      - q: 支持 iPhone 吗？
        a: iOS 版计划在后续发布，目前提供 Android 应用。

  closer:
    title: 下载 Voltip
    body: Windows、macOS、Linux 和 Android 版均可免费下载，源代码按 AGPL-3.0 许可开放。
    keys: [Ctrl, Alt, Space]
    actions:
      - { text: 下载安装, link: /zh/guide/install, theme: brand, icon: download }
      - { text: GitHub, link: "https://github.com/sunerpy/voltip", theme: alt, icon: github }
---

<HomeFlow />

<HomePairs />

<HomePresets />

<HomeModes />

<HomeMore />

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
