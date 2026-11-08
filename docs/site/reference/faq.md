---
description: Answers to common questions and fixes for common problems with Voltip.
---

# Questions and troubleshooting

When dictation does not behave as expected, look for the symptom below; each answer says what causes it and what to change.

## Dictation

### Nothing is typed after I release the shortcut

Check the overlay and the latest entry in the history; they say what happened.

- **The text is on the clipboard**: Voltip could not paste, and the entry says why. Paste with <kbd>Ctrl</kbd> <kbd>V</kbd>, or <kbd>⌘</kbd> <kbd>V</kbd> on a Mac. On a Mac, check that Voltip has the Accessibility permission. On Linux with Wayland, install a paste tool; see [Platform notes](/reference/platforms#pasting).
- **"No speech detected"**: the recording was silent or shorter than about a third of a second. Press **Test microphone** on the home page and watch the ring around **Start dictation** move with your voice, and check the input device: click the microphone name on the home page, or open **Settings → Recording source**.
- **An error**: the recognition service could not be reached, or a key or model is missing. The home page shows which service is in use and what it needs.

### The overlay shows no text while I speak {#no-live-text}

With a provider that recognises while you speak, the overlay shows the words that service returns, with a tag naming where they come from, such as **Preview · Doubao Speech**. If none appear, first check that the model selected on the Speech models page is marked **realtime** and that **Provider's live text** is on under **Live preview** on the Recognition tab; with it off, a realtime model still recognises while you speak, and the overlay only shows no words. When the tag reads **Preview · Local model**, the words come from the live transcription model on this computer, not from the provider.

Then open the entry's details in the history. If they say "Live transcription stopped; the whole recording was transcribed instead", the connection to the service dropped while you spoke, or the service returned no text; Voltip recognised the whole recording instead, so no words are lost. The reason after it names how often the service answered, how long the audio sent was and its loudest level (this part is in Chinese):

- If it says the audio was nearly silent or silent (「几乎没有声音」, 「均为静音」), or the loudest level is very low, check the microphone and the recording source: press **Test microphone** on the home page.
- If the level is normal and the service returned no text, check in the provider's console that the selected model is activated and that the credentials match the model; see [Provider setup](/recognition/providers). Volcengine Doubao Speech also gives a log ID (日志 ID); give it to Volcengine's support when you contact them.

### The shortcut does nothing

Another app may hold the same key combination; **Settings → Shortcuts** shows whether it is registered. On a pure Wayland session, apps cannot register global shortcuts: bind a system shortcut to `voltip-desktop --toggle`.

### Can I use Voltip without an internet connection?

Yes, with a [local model](/recognition/local). Download it once; recognition then works offline. AI polish needs a service, unless you run Ollama on your computer.

### It says built-in speech recognition is used up for today

The built-in service has a daily limit per device: 60 minutes of speech recognition and 100 AI polishes, counted again every day at 00:00 Beijing time. Once it is used up, switch to an [on-device model](/recognition/local) or another provider; on a computer, recordings that were not recognised can be recognised again on another model. The **Usage** card in the sidebar shows the use and the time of the reset; see [Built-in usage](/recognition/cloud#built-in-usage).

### My network needs a proxy

Choose how Voltip connects under **Settings → Network**: it follows the system's proxy settings by default, or uses a proxy server you enter, with a user name, a password and the addresses that go direct. Recognition, polish, model downloads, updates and the connection to your phone all follow this setting; see [Network proxy](/recognition/cloud#network-proxy).

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

### Voltip says it needs the system keychain

Voltip keeps your keys and this device's identity in the system keychain and does not start without it; it does not fall back to a file. On Linux the keychain is the Secret Service: start GNOME Keyring or KWallet and unlock it. When the keychain asks for its password as Voltip starts, enter it; if that prompt is closed instead, Voltip shows this notice and quits. On a Mac, unlock the keychain and allow Voltip to use it when macOS asks. On Windows, make sure the Credential Manager service is running. Then open Voltip again.

### On Linux, Voltip quits right after it starts

If a terminal shows `Error 71 (Protocol error) dispatching to Wayland display` and the computer has an NVIDIA graphics card, the cause is WebKitGTK's DMA-BUF rendering. From version 0.3.0, Voltip turns it off by itself on Wayland sessions, unless you have set `WEBKIT_DISABLE_DMABUF_RENDERER` yourself. Earlier versions need to be started with `WEBKIT_DISABLE_DMABUF_RENDERER=1 voltip-desktop`. If Voltip still quits, start it with `GDK_BACKEND=x11 voltip-desktop`; see [Platform notes](/reference/platforms#nvidia-graphics-cards-on-wayland).

### Where are my files, and how do I remove Voltip?

See [Updates, uninstall and your data](/guide/updates).

## The phone

### How do I install the phone app?

On the phone, download `Voltip_<version>_android_arm64.apk` from the [releases page](https://github.com/sunerpy/voltip/releases) and open it. See [Install](/guide/install#android).

### The floating bubble disappears after a while

Some phones close apps that run in the background. Allow Voltip to start automatically or run in the background in the system settings, and turn off battery restrictions for Voltip; see [Floating bubble](/phone/#floating-bubble).

## Something else

Use **Feedback** at the bottom of the sidebar, or [open an issue on GitHub](https://github.com/sunerpy/voltip/issues). The feedback dialog lists everything it sends before you send it.

To ask other users, join one of the groups on the [Community](/reference/community) page.
