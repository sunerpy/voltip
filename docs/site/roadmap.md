---
description: What is being built next for Voltip, what is planned, and what is deliberately left out.
---

# Roadmap

The next releases, in the expected order. No dates are promised. Each feature is described in the guide once it ships, and the [changelog](https://github.com/sunerpy/voltip/blob/main/CHANGELOG.md) lists every release.

## In development

### Google Play <StatusTag status="building" />

The Android app is on the releases page; see [the phone page](/phone/). A listing on Google Play is being prepared. Google requires a closed test before an app can be published there.

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
