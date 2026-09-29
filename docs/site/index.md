---
layout: home
title: Voltip — dictation for Windows, macOS and Linux
titleTemplate: false
description: Hold a shortcut, speak, and the text appears at your cursor in any app. Recognition on your computer or with a cloud service you choose, with optional AI polish.

hero:
  name: Voltip
  text: Hold a key, speak, and let go.
  tagline: The text appears at your cursor, in any app. Recognition runs on your computer or with a cloud service you choose, and AI polish can tidy it first.
  actions:
    - theme: brand
      text: Download
      link: /guide/install
    - theme: alt
      text: Quick start
      link: /guide/quick-start
    - theme: alt
      text: GitHub
      link: https://github.com/sunerpy/voltip

home:
  facts:
    - term: Runs on
      text: Windows, macOS (Apple silicon and Intel) and Linux. An Android app is in development.
    - term: Your audio
      text: With a local model it never leaves your computer. With a cloud service it goes only to the service you chose.

  visual:
    home:
      light: /screens/home-en-light.webp
      dark: /screens/home-en-dark.webp
      width: 1440
      height: 900
      alt: The Voltip home page with the shortcut, the microphone, the speech model and the latest results.
    pill:
      - light: /screens/overlay-listening-en-light.webp
        dark: /screens/overlay-listening-en-dark.webp
        width: 784
        height: 70
        alt: The overlay while you speak, showing the input level and the Esc key that cancels.
      - light: /screens/overlay-processing-en-light.webp
        dark: /screens/overlay-processing-en-dark.webp
        width: 784
        height: 70
        alt: The overlay during AI polish, showing the preset in use and the time the step has taken.
      - light: /screens/overlay-inserted-en-light.webp
        dark: /screens/overlay-inserted-en-dark.webp
        width: 784
        height: 70
        alt: The overlay after the text has been inserted, showing the number of characters and the app.

  index:
    title: What Voltip does
    intro: All of Voltip's features. Those marked "In development" are being built and are not in a release yet.
    groups:
      - name: Dictation wherever you type
        items:
          - title: A shortcut that works in any app
            body: Hold Ctrl+Alt+Space, speak and release. You can also press once to start and again to stop, or lock a long take with a short press.
            status: available
            link: /dictation/shortcuts
          - title: One key or a mouse button
            body: Right Ctrl, Right Alt, the Fn key on a Mac or a mouse side button can start dictation on their own.
            status: available
            link: /dictation/shortcuts#a-single-key-or-mouse-button
          - title: Text while you speak
            body: The overlay shows the words as they are recognised. The text can be inserted all at once, sentence by sentence, or as you speak.
            status: available
            link: /dictation/output
          - title: Voice edit
            body: Select text, hold Ctrl+Alt+E and say what to change, such as "make it more formal" or "translate into English". The selection is replaced in place.
            status: available
            link: /dictation/voice-edit
      - name: Recognition on your terms
        items:
          - title: Local models that work offline
            body: Qwen3-ASR, SenseVoice and Paraformer run on your computer. Once a model is downloaded, no audio leaves it.
            status: available
            link: /recognition/local
          - title: The GPU when there is one
            body: Qwen3-ASR uses the graphics card through Vulkan on Windows and Linux and Metal on macOS, and the processor otherwise.
            status: available
            link: /recognition/local#cpu-or-gpu
          - title: Cloud services
            body: Release packages include a default service. OpenAI, Groq, SiliconFlow or any OpenAI-compatible endpoint can be used with your own key.
            status: available
            link: /recognition/cloud
      - name: Text that reads right
        items:
          - title: AI polish
            body: Fixes punctuation, typos and filler words without changing what you said. Works with the default service, your own provider or a local Ollama.
            status: available
            link: /recognition/polish
          - title: Dictionary and replacement rules
            body: Teach Voltip your names and terms, and rewrite phrases with literal or regular-expression rules. Chinese can be written in Simplified or Traditional characters.
            status: available
            link: /recognition/dictionary
          - title: Scenes by app
            body: The app you speak into can choose the AI preset, the output mode, the language and extra instructions for the AI.
            status: available
            link: /recognition/scenes
          - title: AI presets and built-in scenes
            body: Proofread, prompt, intent, chat, translation and notes presets and presets of your own, switched from the home page, the title bar or the tray, plus ready-made scenes for coding, office writing, chat and professional fields.
            status: available
            link: /recognition/polish#presets
      - name: Phone, history and long recordings
        items:
          - title: Your phone as a microphone and keyboard
            body: Pair an Android phone by QR code, a 6-digit code or a tap on the same network. Hold to talk or type on the phone, and the text appears at the computer's cursor.
            status: building
            link: /phone/
          - title: An iOS app
            body: The phone as a microphone and keyboard on iOS as well. Development has not started.
            status: planned
            link: /roadmap
          - title: History with copy and paste
            body: Every result is kept on your computer. Search it, copy a result, or paste it into the window you used before.
            status: available
            link: /dictation/history
          - title: 20,000 entries and daily statistics
            body: Characters transcribed, characters corrected and time saved, by day, week and month.
            status: available
            link: /dictation/history#statistics
          - title: Long recordings and computer audio
            body: Up to two hours per recording, from the microphone, the computer's own audio or both, with export to SRT subtitles.
            status: building
            link: /roadmap

  steps:
    title: One dictation, four steps
    items:
      - title: Hold the shortcut
        keys: [Ctrl, Alt, Space]
        body: The microphone opens only now. The overlay at the bottom of the screen shows the input level.
      - title: Speak
        body: With the live transcription model installed, the words appear in the overlay as you say them.
      - title: Release the keys
        body: The speech is recognised, corrected with your dictionary, polished by AI if you want, and rewritten by your rules.
      - title: The text is at your cursor
        body: Voltip pastes the text into the app you were using and keeps a copy in the history. Esc cancels at any point.

  models:
    columns: [Model, Size, Languages, Runs on]
    rows:
      - name: Qwen3-ASR 0.6B
        note: Recommended
        size: 690 MB
        languages: 30 languages, detected automatically
        device: GPU or CPU
      - name: Qwen3-ASR 1.7B
        note: Most accurate
        size: 1.7 GB
        languages: 30 languages, detected automatically
        device: GPU or CPU
      - name: SenseVoice Small
        note: Smallest download
        size: 240 MB
        languages: Chinese, English, Japanese, Korean, Cantonese
        device: CPU
      - name: Paraformer
        note: No punctuation
        size: 227 MB
        languages: Chinese, including dialects, mixed with English
        device: CPU
      - name: Live transcription
        note: Preview while you speak
        size: 169 MB
        languages: Chinese and English
        device: CPU
    caption: Downloads resume after an interruption and are checked against a SHA-256 hash before use. On an NVIDIA L40S, Qwen3-ASR 0.6B recognised 16 seconds of Chinese speech in 0.16 s on the GPU and in 2.87 s on 8 processor threads.

  polish:
    heardLabel: Recognised
    heard: um so I think we should uh ship the fix on friday and then update the docs next week
    typedLabel: Inserted at your cursor
    typed: I think we should ship the fix on Friday and then update the docs next week.
    caption: An example of AI polish. Filler words are removed and the punctuation is fixed; the wording stays yours.

  phone:
    items:
      - title: Pair once
        body: Scan the QR code on the computer, enter its 6-digit code, or tap the computer in the phone's list of computers nearby. Both screens then show the same safety code for you to confirm.
      - title: Hold to talk on the phone
        body: The audio streams to the computer, compressed with Opus and encrypted end to end. The computer recognises it and inserts the text at its own cursor.
      - title: Send text or the clipboard
        body: Type on the phone or send its clipboard. The computer inserts it like a dictation result.
      - title: On any network
        body: On the same network the devices connect directly. Elsewhere, an optional relay forwards the encrypted data without being able to read it.

  platforms:
    title: Platforms
    intro: Voltip is the same app on all three desktop systems. The platform notes list what differs between them.
    columns: [Platform, Packages, Shortcut and single key, Local recognition, GPU]
    rows:
      - name: Windows 10 and 11
        status: available
        cells:
          - Per-user installer or portable zip, x64
          - Both
          - Yes
          - Vulkan
      - name: macOS 11 or later
        status: available
        cells:
          - A dmg for Apple silicon and one for Intel
          - Both, with the Accessibility permission
          - Yes
          - Metal
      - name: Linux
        status: available
        cells:
          - .deb or AppImage, x64; X11 and Wayland
          - Both on X11; on Wayland, a system shortcut runs the toggle command
          - Yes
          - Vulkan
      - name: Android
        status: building
        cells:
          - Not released yet
          - Hold to talk in the app
          - Done by the paired computer
          - —
    note: iOS has not been started. Packages for Windows on ARM and Linux on ARM are not planned for now.

  privacy:
    title: What leaves your computer
    intro: It depends on the services you choose. The home page and the Speech models page always show which one is in use.
    sendsLabel: Sends
    modes:
      - name: Local recognition
        sends: Nothing
        detail: Recognition runs inside Voltip on a model file on your disk. The only network use is downloading the model, which is verified against a SHA-256 hash.
      - name: Cloud recognition
        sends: Audio, to the service you chose
        detail: The recording goes to the built-in service or to the provider you set up, with your key and, if you keep a dictionary, its terms as a hint. Keys are kept in the system keychain.
      - name: AI polish and voice edit
        sends: Text, to the AI service you chose
        detail: The recognised text, or for voice edit the selection and your instruction. The app's name is included unless you turn it off; the window title only if you turn it on. Audio is never sent.

  roadmap:
    title: What comes next
    intro: The next releases, in the expected order. No dates are promised; each feature is described on this site once it ships.
    items:
      - title: Long recordings and computer audio
        status: building
        body: Recordings of up to two hours from the microphone, the computer's own audio or both. Long recordings are recognised in segments and can be exported as SRT subtitles.
      - title: The Android app
        status: building
        body: Using the phone as a microphone and keyboard is built and tested. The app is being prepared for release.
      - title: Streaming recognition with cloud services
        status: planned
        body: Text while you speak currently uses the local live transcription model. Streaming results from cloud services are planned.
      - title: More local models
        status: planned
        body: Whisper among others, and measurements on AMD and Intel graphics cards.
      - title: An iOS app
        status: planned
        body: The phone as a microphone and keyboard on iOS as well. Development has not started.
    notPlanned:
      title: Deliberately left out
      items:
        - Listening all the time, or stopping when you fall silent. The microphone opens only while you record.
        - Speaker separation, meeting detection and batch transcription of files.
        - A browser-only version. The global shortcut, typing into other apps and local models need a native app.
        - Syncing the history, dictionary or rules to the phone. The phone is a microphone and keyboard; the computer does the work.
---

<HomeIndex />

<HomeSteps />

<SplitBlock proof="models">

## Recognition on your computer, or in the cloud

Download a model once and dictation works offline. Qwen3-ASR recognises 30 languages and writes the punctuation itself; SenseVoice and Paraformer are small and fast on any processor. When the computer has a graphics card, Qwen3-ASR runs on it through Vulkan or Metal, and on the processor otherwise.

Release packages also include a default cloud service, so dictation works before you set anything up. You can switch to OpenAI, Groq, SiliconFlow or any OpenAI-compatible endpoint with your own key at any time.

[Local recognition](/recognition/local) · [Cloud services](/recognition/cloud)

</SplitBlock>

<SplitBlock proof="polish" flip>

## Text that reads the way you meant it

Recognition gets the words; the next steps make them read right. The dictionary corrects the names and terms a recogniser tends to mishear. AI polish removes filler words and fixes the punctuation. Replacement rules rewrite phrases exactly as you specify, and scenes adjust all of this to the app you are writing in.

Presets decide what AI polish does: proofreading by default, or a prompt, intent, chat, translation or notes, chosen from the home page, the title bar or the tray, and presets you write yourself.

[AI polish](/recognition/polish) · [Dictionary and rules](/recognition/dictionary) · [Scenes](/recognition/scenes)

</SplitBlock>

<SplitBlock proof="phone">

## Your phone as a microphone and keyboard

<StatusTag status="building" />

Pair an Android phone with the computer and hold to talk on the phone. The computer recognises the speech with its own settings and inserts the text at its cursor. The phone can also send typed text or its clipboard.

The Android app is built and tested but not released yet. It will be on the releases page once it is. [How the phone works](/phone/)

</SplitBlock>

<HomePlatforms />

<HomePrivacy />

## Install

One command picks the right package for the computer, checks it against the release's `SHA256SUMS` and installs it.

::: code-group

```powershell [Windows]
irm https://raw.githubusercontent.com/sunerpy/voltip/main/scripts/install.ps1 | iex
```

```bash [Linux and macOS]
curl -fsSL https://raw.githubusercontent.com/sunerpy/voltip/main/scripts/install.sh | sh
```

:::

Packages for every platform, checksums and build attestations are on the [releases page](https://github.com/sunerpy/voltip/releases). The [install guide](/guide/install) covers each platform, including the first start on a Mac.

<HomeRoadmap />
