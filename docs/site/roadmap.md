---
description: What is being built next for Voltip, what is planned, and what is deliberately left out.
---

# Roadmap

The next releases, in the expected order. No dates are promised. Each feature is described in the guide once it ships, and the [changelog](https://github.com/sunerpy/voltip/blob/main/CHANGELOG.md) lists every release.

## In development

### AI presets and built-in scenes <StatusTag status="building" />

Several built-in AI presets — Proofread (the default), Prompt, Intent, Chat, Translate between Chinese and English, and Notes — switched from the home page, the title bar or the tray, and your own presets. Built-in scenes for coding, office writing, chat and professional fields, each with a preset, instructions, specialist terms and a list of apps. See [AI polish and presets](/recognition/polish#presets) and [Scenes](/recognition/scenes#built-in-scenes).

### 20,000 history entries and statistics <StatusTag status="building" />

The history limit rises from 500 to 20,000 entries, and the home page shows characters transcribed, characters corrected, speaking time and time saved for today, this week, this month and in total. See [History](/dictation/history#statistics).

### Long recordings and computer audio <StatusTag status="building" />

- Recordings of up to two hours, with the limit set under Settings.
- Recording from the microphone, from the computer's own audio, or from both mixed.
- Long recordings are recognised in segments while you speak.
- The result can be processed with an AI preset in parts, and exported as SRT subtitles or as text.

### The Android app <StatusTag status="building" />

Using the phone as a microphone and keyboard is built and tested; the app is being prepared for release. See [the phone page](/phone/).

## Planned

- Streaming recognition with cloud services. Text while you speak currently uses the local live transcription model.
- A check that the field in focus accepts text before pasting.
- More local models, such as Whisper, and measurements on AMD and Intel graphics cards.
- Exporting and importing all data, and a diagnostics bundle.
- Code signing for the Windows packages.
- An iOS app, with the phone as a microphone and keyboard as on Android. Development has not started.

## Deliberately left out

- **Listening all the time, or stopping when you fall silent.** An always-on microphone could type other people's words into the app in front, and stopping on silence cuts off dictation in a noisy room.
- **Polishing sentence by sentence in Type as you speak.** Sentences already typed cannot be taken back and rewritten.
- **Speaker separation, meeting detection and batch transcription of files.**
- **Browser addresses, the clipboard or screenshots as context for AI polish.** They can contain sensitive content unrelated to the dictation.
- **A browser-only version.** The global shortcut, typing into other apps and local models need a native app.
- **Syncing the history, dictionary, rules or settings to the phone.** The phone is a microphone and keyboard; the computer does the recognition, the corrections and the typing.
- **Packages for Windows on ARM and Linux on ARM**, for now.
