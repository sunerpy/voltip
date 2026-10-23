---
layout: home
markdownStyles: false
title: Voltip — dictation for Windows, macOS, Linux and Android
titleTemplate: false
description: Hold a shortcut, speak, and the text appears at your cursor in any app. Recognition on your computer or with a cloud service you choose, with optional AI polish.

hero:
  text: Hold to speak,<br>release to type
  tagline: The text appears at your cursor, in any app. Recognition runs on your computer or with a cloud service you choose, and AI polish can tidy it first.
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

  pairs:
    title: Filler words removed, corrections applied
    intro: AI polish removes filler words, fixes the punctuation and keeps the correction you made, without rewording what you said. You can turn it off.
    labels:
      said: You said
      inserted: Inserted
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
    title: "One sentence,\na preset for each occasion"
    intro: A preset decides how AI polish handles the text. Eight are built in, with their prompts open to read, and any of them can be copied into a preset of your own and changed.
    points:
      - Proofread is the default; switch from the home page, the tray or next to the AI Polish switch
      - With scenes on, the app you are writing in picks the preset
    saidLabel: You said
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
      items:
        - name: Coding
          body: VS Code, Cursor, Kiro, the JetBrains IDEs and terminals. Code, commands, paths and identifiers stay as they are.
          preset: Proofread
        - name: Office writing
          body: Mail and documents in Outlook, Word, WPS, OneNote, Notion and similar apps.
          preset: Formal
        - name: Chat
          body: WeChat, QQ, Slack, Teams, DingTalk, Feishu, Telegram, Discord and other messaging apps.
          preset: Casual chat
        - name: Legal, Medical, Finance, Academic
          body: Strict proofreading that keeps specialist terms as they are. They apply once you add the apps you use.
          preset: Proofread

  modes:
    title: You choose where the speech is recognised
    intro: Where the recording goes and whose model recognises it can be changed at any time on the Speech models page.
    scope: This computer
    items:
      - icon: cpu
        label: On this computer
        title: Download once, work offline
        flow: { from: You speak, to: Local model, icon: cpu, local: true }
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
        title: Dictate right after installing
        flow: { from: You speak, to: Built-in service, icon: cloud, local: false }
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
        title: Use your own key
        flow: { from: You speak, to: Provider, icon: server, local: false }
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
        - { name: Ollama, mark: ollama }
    note: On an NVIDIA L40S, Qwen3-ASR 0.6B recognised 16 seconds of Chinese speech in 0.16 s on the GPU and in 2.87 s on 8 processor threads. See [Local recognition](/recognition/local) and [Cloud services](/recognition/cloud).

  phone:
    title: Your phone as a microphone and keyboard
    intro: Pair an Android phone with the computer and hold to talk on the phone. The computer recognises the speech with its own settings and inserts the text at its cursor. With no computer online, the phone transcribes on its own.
    items:
      - icon: qr
        title: Pair once
        body: Scan the QR code on the computer or enter its 6-digit code, then check that both screens show the same safety code.
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
        height: 600
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
      - q: Is there an iPhone app?
        a: An iOS app is planned for a later release. An Android app is available now.

  closer:
    title: Hold the shortcut and start speaking
    body: Free to download for Windows, macOS, Linux and Android.
    actions:
      - { text: Download, link: /guide/install, theme: brand, icon: download }
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

```bash [Linux and macOS]
curl -fsSL https://raw.githubusercontent.com/sunerpy/voltip/main/scripts/install.sh | sh
```

:::

</HomePlatforms>

<HomeFaq />

<HomeCloser />
