---
description: Answers to common questions and fixes for common problems with Voltip.
---

# Questions and troubleshooting

When dictation does not behave as expected, look for the symptom below; each answer says what causes it and what to change.

## Dictation

### Nothing is typed after I release the shortcut

Check the overlay and the latest entry in the history; they say what happened.

- **The text is on the clipboard**: Voltip could not paste, and the entry says why. Paste with <kbd>Ctrl</kbd> <kbd>V</kbd>, or <kbd>⌘</kbd> <kbd>V</kbd> on a Mac. On a Mac, check that Voltip has the Accessibility permission. On Linux with Wayland, install a paste tool; see [Platform notes](/reference/platforms#pasting).
- **"No speech detected"**: the recording was silent or shorter than about a third of a second. Press **Test microphone** on the home page and check the strength bar, and the input device: click the microphone name on the home page, or open **Settings → Recording source**.
- **An error**: the recognition service could not be reached, or a key or model is missing. The home page shows which service is in use and what it needs.

### The shortcut does nothing

Another app may hold the same key combination; **Settings → Shortcuts** shows whether it is registered. On a pure Wayland session, apps cannot register global shortcuts: bind a system shortcut to `voltip-desktop --toggle`.

### Can I use Voltip without an internet connection?

Yes, with a [local model](/recognition/local). Download it once; recognition then works offline. AI polish needs a service, unless you run Ollama on your computer.

### Does Voltip listen all the time?

No. The microphone opens when you press the shortcut and closes when the recording ends. The microphone test on the home page runs for at most 15 seconds.

### The text comes out in Traditional characters

Some models write Traditional characters. Under **Speech models → Recognition → Chinese script**, choose **Simplified**, which is the default.

### Esc does not cancel

Esc works while a dictation is recording or being processed. On a pure Wayland session, use a shortcut bound to `voltip-desktop --cancel` instead.

## Local models

### The first dictation on the graphics card is slow

The first run on a graphics card prepares the model for it, which took about 13 seconds on an NVIDIA L40S and about two minutes on a Tesla T4 in our measurements. Later runs are fast.

### The model download is slow

Voltip switches to another download source when one is slow, and resumes interrupted downloads. You can also download from the command line: `voltip-desktop --download-model qwen3-asr-0.6b`.

### Qwen3-ASR does not load on an older computer

It needs a processor with AVX2. Use SenseVoice Small, Paraformer or a cloud service instead.

## Installing and updating

### Windows says "Windows protected your PC"

The packages are not code-signed yet. Choose **More info**, then **Run anyway**.

### macOS blocks Voltip or asks for permissions again

The first start of an app that is not notarized needs one confirmation; see [Install](/guide/install#macos). Updating from version 0.0.6 or earlier asks for the permissions once more. Releases share one signing certificate, so updates keep the microphone and Accessibility permissions. The keychain asks whenever one version reads entries that another version created. The post-install hand-over attempted by 0.0.15 and 0.0.16 did not work because the updater had already removed the old bundle. The first release with the fix may therefore ask one last time when installed by 0.0.16. In-app updates between two fixed releases avoid the prompt; installing by hand still asks once for each entry.

### Where are my files, and how do I remove Voltip?

See [Updates, uninstall and your data](/guide/updates).

## The phone

### How do I install the phone app?

On the phone, download `Voltip_<version>_android_arm64.apk` from the [releases page](https://github.com/sunerpy/voltip/releases) and open it. See [Install](/guide/install#android).

## Something else

Use **Feedback** at the bottom of the sidebar, or [open an issue on GitHub](https://github.com/sunerpy/voltip/issues). The feedback dialog lists everything it sends before you send it.
