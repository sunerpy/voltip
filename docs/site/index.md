---
layout: home
markdownStyles: false
title: Voltip — dictation for Windows, macOS, Linux and Android
titleTemplate: false
description: Hold a shortcut, speak, and the text appears at your cursor in any app. Recognition on your computer or with a cloud service you choose, with optional AI polish.

hero:
  text: Hold to speak,<br>release to type
  tagline: Open-source dictation that types at your cursor in any app. Recognition runs offline with a local model, or on the built-in service or your own provider; AI polish tidies the wording with the preset you choose.
  actions:
    - theme: brand
      text: Download
      link: /guide/install
    - theme: alt
      text: Try it online
      link: /guide/try

home:
  checks:
    - Windows, macOS, Linux and Android
    - With a local model, the audio stays on your computer
    - The built-in service is free and needs no key
    - Open source, AGPL-3.0

  scene:
    steps:
      - Hold Ctrl+Alt+Space and speak; the overlay shows what you say
      - Release the keys; the text is proofread and inserted at your cursor
    live:
      src: /screens/hero-live-en.webp
      width: 884
      height: 104
      alt: "The overlay while you speak: …on Monday at ten. No, sorry, at two. Uh."
    label: Inserted at the cursor
    result: The review is on Monday at two. Please bring the latest numbers.
    inserted:
      src: /screens/hero-inserted-en.webp
      width: 506
      height: 72
      alt: "The overlay after inserting: Inserted 64 chars, Paste."
    caption: A real dictation with the built-in service and the Proofread preset. The overlays are frames of a screen recording.

  try:
    title: Try Voltip in your browser, with nothing to install
    body: Say a sentence and see the recognised text and its AI-polished version a few seconds later.
    heard: ~~um so~~ let's meet on tuesday
    polished: Let's meet on Tuesday.
    action:
      text: Try it now
      link: /guide/try

  flow:
    title: What happens between the shortcut and the cursor
    intro: Each step has its own settings, and AI polish can be turned off. If the dictionary, AI polish or the rules fail, the dictation completes with the text from the step before.
    steps:
      - icon: mic
        name: Recording
        text: Hold to talk, or press once to start and again to stop; record the microphone, the sound the computer plays, or both.
      - icon: audio
        name: Recognition
        text: A local model, the built-in service or your provider, on the whole recording or as you speak. Your dictionary's right spellings and the scene's terms go with the recording as hotwords, so cloud recognition keeps them; local models take none.
      - icon: book
        name: Dictionary
        text: After recognition, what a word is heard as is replaced with its right spelling, for local models and cloud services alike.
      - icon: wand
        name: AI polish
        tag: Optional
        text: "Tidies the text with the current preset or scene: filler words go, punctuation comes in, corrections are kept."
      - icon: replace
        name: Rules
        text: Rewrite the final text in list order, by a literal match or a regular expression.
      - icon: cursor
        name: Insert
        text: Pastes at your cursor and puts the clipboard back, keeping a copy in the history on this computer.
    note: The whole process is described on [What is Voltip](/guide/what-is-voltip).

  pairs:
    title: The text is tidied before it goes in
    intro: AI polish removes filler words, fixes the punctuation and keeps the correction you made, without rewording what you said. You can turn it off.
    labels:
      said: Recognised words
      inserted: Inserted text
      edit: Select text, hold Ctrl+Alt+E and say
      selection: Selected text
      replaced: Replaced with
      actual: Actual output
      example: Example
    items:
      - kind: dictation
        source: actual
        said: ~~Um,~~ the review is on Monday ~~at ten, no, sorry,~~ at two. ~~Uh,~~ please bring the latest numbers.
        result: The review is on Monday at two. Please bring the latest numbers.
      - kind: dictation
        source: example
        said: ~~um so~~ I think we should ~~uh~~ ship the fix on friday and then update the docs next week
        result: I think we should ship the fix on Friday and then update the docs next week.
      - kind: edit
        source: actual
        said: Turn it into a bulleted list.
        selection: "Let's meet on Thursday at three in the main conference room to go over the release plan.\n\nThe review is on Monday at two. Please bring the latest numbers."
        result: "- Let's meet on Thursday at three in the main conference room to go over the release plan.\n- The review is on Monday at two. Please bring the latest numbers."
    note: “Actual output” comes from a screen recording of Voltip. “Example” is illustrative; real results depend on the AI model you choose.

  presets:
    title: A preset decides how the text is tidied
    intro: Eight are built in, with their prompts open to read, and any of them can be copied into a preset of your own and changed.
    points:
      - Proofread is the default; switch from the home page, the tray or next to the AI Polish switch
      - With scenes on, the app you are writing in picks the preset
    saidLabel: Recognised words
    said: um so the thursday release let's push it to next monday and uh testing still has two open issues I'll let everyone know once they're fixed
    outputs:
      - name: Proofread
        desc: Fixes typos and punctuation, removes filler words
        text: So the Thursday release, let's push it to next Monday, and testing still has two open issues. I'll let everyone know once they're fixed.
      - name: Clarify intent
        desc: Keeps the corrections, lists several points
        text: "- Push the Thursday release to next Monday.\n- Testing still has two open issues; let everyone know once they're fixed."
      - name: Formal
        desc: Complete, formal written language
        text: The Thursday release will be postponed to next Monday. Testing still has two open issues, and I will inform everyone once they have been resolved.
      - name: Chinese ⇄ English
        desc: Fixes recognition errors, then translates
        text: 周四的发布推迟到下周一。测试还有两个问题待解决，修复后我会通知大家。
    note: Example outputs; real results depend on the AI model you choose. The others are Prompt optimizer, Casual chat, Key points and Punctuation only.
    scenes:
      title: "Scenes: the preset and extra instructions follow the app"
      note: Built-in scenes apply once you turn them on; you can add a scene for any app.
      labels:
        preset: Preset
        instruction: Extra instructions
      items:
        - name: Coding
          preset: Proofread
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
          more: And Windows Terminal, PowerShell and each system's own terminal.
          tools:
            label: Coding agents in the terminal
            apps:
              - { name: Codex, mark: codex }
              - { name: OpenCode, mark: opencode }
              - { name: Pi Agent, mark: pi }
            more: And Claude Code and other coding agents that run in a terminal.
          body: Keeps code, commands, paths, identifiers and Markdown as they are.
        - name: Office writing
          preset: Formal
          apps:
            - { name: Notion, mark: notion }
            - { name: Obsidian, mark: obsidian }
            - { name: Thunderbird, mark: thunderbird }
            - { name: LibreOffice Writer, mark: libreoffice-writer }
          more: And Outlook, Word, WPS, OneNote, Typora, and Mail and Pages on a Mac.
          body: Complete sentences and natural paragraphs for mail and documents, with no invented greetings or facts.
        - name: Chat
          preset: Casual chat
          apps:
            - { name: WeChat, mark: wechat }
            - { name: QQ, mark: qq }
            - { name: Telegram, mark: telegram }
            - { name: Discord, mark: discord }
          more: And WeCom, Slack, Teams, DingTalk, Feishu, Lark, and Messages on a Mac.
          body: "No extra instructions: the Casual chat preset writes short, conversational sentences without a closing full stop."
        - name: Legal, Medical, Finance, Academic
          preset: Proofread
          apps:
            - { name: Legal, icon: scale }
            - { name: Medical, icon: stethoscope }
            - { name: Finance, icon: chart }
            - { name: Academic, icon: cap }
          more: Each field is a scene of its own and applies once you add the apps you use.
          body: Strict proofreading that keeps specialist terms as they are and writes numbers, units and dates correctly.
  modes:
    title: Three ways to recognise speech
    intro: On this computer, with the built-in service or with your own provider; switch at any time on the Speech models page.
    items:
      - icon: cpu
        label: On this computer
        title: Uses a model downloaded to this computer
        assure: The audio stays on this computer
        specs:
          - { term: Cost, text: Free }
          - { term: Needs, text: "One model download, from 227 MB" }
          - { term: GPU, text: "Qwen3-ASR uses Vulkan on Windows and Linux and Metal on macOS, and the processor without a graphics card" }
        chipsLabel: Models
        chips:
          - name: Qwen3-ASR 0.6B
          - name: Qwen3-ASR 1.7B
          - name: SenseVoice Small
          - name: Paraformer
      - icon: cloud
        label: Built-in service
        badge: Default
        title: Run by the Voltip project
        assure: The server keeps no recordings or text
        specs:
          - { term: Cost, text: "Free, shared by all users, limited per address" }
          - { term: Needs, text: No key }
          - { term: Preview, text: The overlay shows the words as you speak }
        chipsLabel: AI polish models
        chips:
          - name: Qwen3.8 27B
          - name: GPT-OSS 120B
          - name: GPT-OSS 20B
      - icon: key
        label: Your provider
        title: Uses your own API key
        assure: The recording goes straight to the provider you chose
        specs:
          - { term: Cost, text: "The provider's prices; some have a free allowance" }
          - { term: Needs, text: "An API key from the provider" }
          - { term: Key, text: Kept in the system keychain }
        chipsLabel: Recognition providers
        chips:
          - { name: OpenAI, mark: openai }
          - { name: Groq, mark: groq }
          - { name: SiliconFlow, mark: siliconflow }
          - { name: Alibaba Cloud Model Studio, mark: aliyun }
          - { name: OpenAI-compatible, icon: plug }
    polish:
      title: AI polish
      body: After recognition, a language model tidies the text. It can run on the built-in service, your provider, or Ollama on your own computer.
      marks:
        - { name: OpenAI, mark: openai }
        - { name: Groq, mark: groq }
        - { name: Google AI Studio, mark: google }
        - { name: SiliconFlow, mark: siliconflow }
        - { name: Model Studio, mark: aliyun }
        - { name: DeepSeek, mark: deepseek }
        - { name: Kimi, mark: kimi }
        - { name: Zhipu GLM, mark: glm }
        - { name: Volcengine Ark, mark: volcengine }
        - { name: Tencent Hunyuan, mark: hunyuan }
        - { name: Xiaomi MiMo, mark: mimo }
        - { name: OpenRouter, mark: openrouter }
        - { name: Ollama, mark: ollama }
    note: On an NVIDIA L40S, Qwen3-ASR 0.6B recognised 16 seconds of Chinese speech in 0.16 s on the GPU and in 2.87 s on 8 processor threads. See [Local recognition](/recognition/local) and [Cloud services](/recognition/cloud).

  more:
    title: Make it fit the way you work
    intro: These are in the desktop app; the ones that start off can be turned on in Settings.
    items:
      - title: Dictionary and rules
        wide: true
        body: Add a word the recogniser gets wrong once, and it is spelled right from then on; cloud recognition and AI polish get the right spelling as a hint too. Rules rewrite the text last, with regular expressions if needed. In the history, select a misheard word to add it.
        swap: { from: cube control, to: kubectl }
        guide: "[Dictionary and rules](/recognition/dictionary)"
      - title: Start with a single key
        body: Right Ctrl, Right Alt, Fn on a Mac, or the middle or side mouse buttons can start dictation on their own. Pressing another key while you hold one cancels the recording, so Right Ctrl + C keeps working.
        keys: [Right Ctrl, Right Alt, Fn]
        guide: "[Shortcuts and recording modes](/dictation/shortcuts)"
      - title: Computer audio and long recordings
        body: Record the microphone, the sound the computer plays, or both, with the speakers' echo removed when mixing. A recording can run up to 2 hours and be exported as SRT subtitles or text.
        chips: [Microphone, Computer audio, Mixed]
        guide: "[Recording source](/dictation/shortcuts#recording-source)"
      - title: When a model's quota runs out
        wide: true
        body: Recognition and AI polish each keep a list of up to 8 fallback models. When the selected model's quota is used up, Voltip moves on to the next and the dictation still goes through; other errors, such as a lost connection or a wrong key, are reported and never switch models.
        chain: [Qwen3.8 27B, GPT-OSS 120B, GPT-OSS 20B]
        guide: "[When a model's quota runs out](/recognition/cloud#when-a-model-s-quota-runs-out)"
      - title: AI tools
        body: Claude Code, Codex and other AI tools can read the history, dictionary and rules over MCP, collect the misrecognitions that keep coming back, and add entries and rules with your permission. Every change can be undone.
        chips: [Claude Code, Codex, MCP]
        guide: "[AI tools](/recognition/mcp)"
      - title: Local service
        wide: true
        body: Other programs send a recording to an OpenAI-compatible API and get back text processed by the same recognition, dictionary, presets and rules. Turn it on in the app, or run voltip-server on a Linux server without a desktop; by default it accepts connections from this computer only, and every request needs an access token.
        code: voltip-server --scene coding --preset prompt
        guide: "[Local service](/recognition/service)"

  phone:
    title: Your phone as a microphone and keyboard
    intro: Pair an Android phone with the computer and hold to talk on the phone. The computer recognises the speech with its own settings and inserts the text at its cursor. With no computer online, the phone transcribes on its own.
    items:
      - icon: qr
        title: Pair once
        body: Scan the QR code on the computer and confirm on the phone; with the 6-digit code, also check that both screens show the same safety code.
      - icon: mic
        title: Hold to talk on the phone
        body: The audio streams to the computer, compressed with Opus and encrypted end to end. The computer recognises it and inserts the text at its cursor.
      - icon: keyboard
        title: Send text or the clipboard
        body: Type on the phone or send its clipboard, and the computer inserts it like a dictation result.
      - icon: history
        title: The computer's history on the phone
        body: The computer's history and settings are synced to the phone, to read there.
      - icon: relay
        title: On any network
        body: The devices connect through a relay that forwards encrypted data only, and reconnect on their own after a change of network.
    figure:
      back:
        src: /screens/phone-pair-en-light.webp
        dark: /screens/phone-pair-en-dark.webp
        width: 1482
        height: 597
        alt: "The Phone page on the computer: pairing with a QR code and a 6-digit code."
      front:
        src: /screens/phone-mic-en-light.webp
        dark: /screens/phone-mic-en-dark.webp
        width: 760
        height: 501
        alt: "Phone microphone: the paired phone is online; hold to talk on it."
      caption: "The Phone page on the computer: pairing, and the paired phone's microphone."
    links: "[How the phone works](/phone/) · [Install the Android app](/guide/install#android)"

  platforms:
    title: "Windows, macOS, Linux\nand Android"
    intro: The desktop app is the same on all three systems; the Android app makes a phone the computer's microphone and keyboard.
    cards:
      - name: Windows 10 and 11
        status: available
        specs:
          - { term: Packages, text: "Per-user installer or portable zip, x64" }
          - { term: GPU, text: Vulkan }
      - name: macOS 11 or later
        status: available
        specs:
          - { term: Packages, text: A dmg for Apple silicon and one for Intel }
          - { term: GPU, text: Metal }
      - name: Linux
        status: available
        specs:
          - { term: Packages, text: ".deb or AppImage, x64; X11 and Wayland" }
          - { term: GPU, text: Vulkan }
      - name: Android 8.0 or later
        status: available
        specs:
          - { term: Package, text: "APK, 64-bit Arm" }
          - { term: Recognition, text: "By the paired computer, or on its own" }
    install: One command picks the right package for the computer, checks it against the release's SHA256SUMS and installs it. Packages for every platform and their checksums are on the [releases page](https://github.com/sunerpy/voltip/releases); the [install guide](/guide/install) covers each platform.
    note: An iOS app is planned for a later release. Packages for Windows on ARM and Linux on ARM are not planned for now.

  faq:
    title: Questions
    intro: More questions and troubleshooting are on the [Questions](/reference/faq) page.
    items:
      - q: Can I use Voltip without an internet connection?
        a: Yes, with a [local model](/recognition/local). Download it once; recognition then works offline. AI polish needs a service, unless you run Ollama on your computer.
      - q: Does Voltip listen all the time?
        a: No. The microphone opens when you press the shortcut and closes when the recording ends.
      - q: Where do my recordings and text go?
        a: It depends on the services you choose. Local recognition sends nothing; cloud recognition sends the recording to the built-in service or the provider you set up; AI polish sends text only, never audio. See [Privacy](/privacy).
      - q: Does the built-in service cost anything or need a key?
        a: No. The Voltip project runs it, all users share it, and requests are limited per address. For AI polish you can rely on, use a provider of your own.
      - q: My network needs a proxy
        a: Choose how Voltip connects under Settings → Network. It follows the system's proxy settings by default, or uses a proxy server you enter. Recognition, polish, model downloads, updates and the connection to your phone all follow this setting.
      - q: Do I have to hold the shortcut the whole time?
        a: No. Choose Press to start, again to stop, or Hold or press, where a short press locks the recording. A recording stops on its own at the length you set; Voltip has no wake word and does not stop when it hears silence. See [Shortcuts and recording modes](/dictation/shortcuts).
      - q: Are the packages signed?
        a: The Windows installer is not code-signed yet, so SmartScreen warns on the first start. The macOS app is signed with the project's own certificate but not notarized by Apple, so you confirm it once on first opening. Every release comes with SHA256SUMS and build attestations, and the one-line install checks the download against SHA256SUMS. See [Install](/guide/install).
      - q: Is there an iPhone app?
        a: An iOS app is planned for a later release. An Android app is available now.

  closer:
    title: Download Voltip
    body: Free for Windows, macOS, Linux and Android, with the source code under AGPL-3.0.
    keys: [Ctrl, Alt, Space]
    actions:
      - { text: Download, link: /guide/install, theme: brand, icon: download }
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

```bash [Linux and macOS]
curl -fsSL https://raw.githubusercontent.com/sunerpy/voltip/main/scripts/install.sh | sh
```

:::

</HomePlatforms>

<HomeFaq />

<HomeCloser />
