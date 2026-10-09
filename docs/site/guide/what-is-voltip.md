---
description: What Voltip is, how a dictation works from the shortcut to the cursor, and what it deliberately does not do.
---

# What is Voltip

Voltip is a dictation app for Windows, macOS and Linux. You hold a shortcut, speak and release it, and the text appears at your cursor in whatever app you are using.

## Watch the tutorial

<VideoFigure src="/media/voltip-tutorial-en.mp4" poster="/media/voltip-tutorial-en.webp" width="1920" height="1080" title="Voltip tutorial" caption="2 min 52 s, recorded with Voltip 0.0.14. The Chinese version is on the Chinese page." />

| Time | Chapter |
| --- | --- |
| 0:00 | Introduction |
| 0:07 | Install |
| 0:22 | The home page and the microphone test |
| 0:41 | The first dictation |
| 1:01 | AI polish |
| 1:27 | Voice edit |
| 1:47 | Recognition on this computer |
| 2:19 | Dictionary and rules |
| 2:30 | History |
| 2:40 | Where to find more |

## How a dictation works

1. **Hold the shortcut**, Ctrl+Alt+Space by default. Voltip opens the microphone and shows a small overlay at the bottom of the screen with the input level.
2. **Speak.** With the live transcription model installed, the words appear in the overlay as you say them.
3. **Release the keys.** The recording is recognised, either on your computer with a downloaded model or by a cloud service. Voltip then applies your dictionary, AI polish if it is on, and your replacement rules.
4. **The text is pasted at your cursor** in the app you were using, and a copy is saved in the history on your computer.

Press Esc at any point during a dictation to cancel it.

## What you can adjust

- **How recording starts and stops**: hold to talk, press to start and again to stop, or lock a long take with a short press. A single key or a mouse button can also start dictation. See [Shortcuts and recording modes](/dictation/shortcuts).
- **Where recognition happens**: on this computer with a local model, with the built-in service, or with a cloud provider you choose. See [Local recognition](/recognition/local) and [Cloud services](/recognition/cloud).
- **How the text is written**: AI polish, a personal dictionary, replacement rules and scenes for particular apps. See [AI polish](/recognition/polish), [Dictionary and rules](/recognition/dictionary) and [Scenes](/recognition/scenes).
- **How the text is inserted**: pasted at the cursor or only copied to the clipboard, all at once or while you speak. See [Where the text goes](/dictation/output).

## What Voltip does not do

Voltip turns speech into text and does nothing else with it: there is no wake word, and it does not carry out what you say. Voice edit rewrites a selection only when you ask it to. The microphone is open only while you record, or while you test it on the home page.

Your keyboard and input method keep working as before, because Voltip inserts text by pasting it, as any app can. A few places refuse pasted text; [Where the text goes](/dictation/output#when-the-text-cannot-be-pasted) explains them.

## Next steps

- [Install Voltip](/guide/install)
- [Quick start](/guide/quick-start)
