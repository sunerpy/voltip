---
description: The dictation shortcut, the three recording modes, starting with a single key or a mouse button, the overlay, cancelling with Esc, the shortcut that copies the last result, and what and how long a dictation records.
---

# Shortcuts and recording modes

Dictation starts with a global shortcut that works in any app: <kbd>Ctrl</kbd> <kbd>Alt</kbd> <kbd>Space</kbd> by default. To change it, open **Settings → Shortcuts** and record a new key combination.

Which combinations can be registered depends on the operating system. Settings → Shortcuts shows whether the combination you chose is registered, and why not if another app holds it.

## Recording modes

Settings → Shortcuts offers three ways for the shortcut to control a recording:

| Mode | How it works |
| --- | --- |
| **Hold to talk** (default) | Hold the shortcut while you speak; releasing it ends the recording and starts recognition. |
| **Press to start, again to stop** | Press once to start and once more to stop. Releasing the keys does nothing. |
| **Hold or press** | Hold to talk as above; a short press instead locks the recording so you can speak hands-free, and the next press ends it. |

For the third mode, **Short-press threshold** sets how long a press may last and still count as short (300 ms by default). In every mode, **Keep recording after release** keeps the microphone open for up to five seconds after you stop, so the last word is not cut off. New installs start at 150 ms; 0 stops at once. Measured on a Linux computer with the local Paraformer model (20 Chinese and mixed Chinese-English sentences, three times each): stopping 0.1 seconds before the last word ended cut the last character off about 5% of the sentences, and stopping 0.2 seconds before, about 15%; at 150 ms none lost it. An earlier install keeps its setting, and you can change it to 150 ms here.

When you hold to talk and the release never reaches Voltip (for example because another program caught the key at that moment), Voltip notices within a second that the key is up and ends the recording as usual. This works on Windows, on a Mac and in an X11 session on Linux.

## A single key or mouse button

A single key can start dictation on its own, in addition to the shortcut: Right Ctrl, Right Alt, Right Shift, the right Windows, Command or Super key, the Fn key on a Mac, or the middle, back or forward mouse button. It follows the same recording mode as the shortcut.

- Pressing another key or clicking while you hold it cancels the recording that the press started, so shortcuts such as Right Ctrl + C keep working.
- On many European keyboard layouts Right Alt is AltGr, and Right Shift is used for capital letters; the setting warns when you choose one of these.
- On a Mac, set **When pressing the 🌐 key** to **Do Nothing** under System Settings → Keyboard before using Fn.
- On a Mac the single key needs the Accessibility permission, which Voltip already asks for to paste.
- A pure Wayland session does not allow it.

Windows sometimes stops Voltip from watching the single key, for example when the computer is very busy for a moment. Voltip notices and restores it without a restart, and it starts watching again after the computer wakes from sleep or is unlocked. **Settings → Shortcuts** shows when it last recovered. On a Mac, if the Accessibility permission is withdrawn, the setting says so and recovers once the permission is granted again; on Linux, Voltip reconnects when the connection to the X server is lost.

## The overlay

While you dictate, a small overlay at the bottom of the screen shows what is happening:

- **Recording**: the input level, the elapsed time, what is being recorded (Microphone, Computer audio or Mixed) and the [words recognised while you speak](#live-text-while-you-speak). A padlock appears when a recording is locked. After the first two minutes of a long recording, it also shows how many segments have been recognised.
- **Processing**: the step in progress — recognition, AI polish or insertion — with the time it has taken. For a long recording, recognition shows how many of its segments are done.
- **Done**: how many characters were inserted, or why the text stayed on the clipboard. When the text was not written, the window changed or recognition failed, the overlay keeps a [result card](/dictation/output#result-cards) with buttons to paste or copy the text.

The overlay stays above other windows, including windows set to stay on top and programs in full screen. It comes back to the top whenever it appears or changes state, and on Windows and Mac it also stays on top while it shows, when another window comes to the front or is set to stay on top. The overlay never takes the keyboard focus, so the text still goes to the app you were using.

These can still cover the overlay:

- **Windows**: a game running in exclusive full screen, and system screens such as the Start menu or the notification centre.
- **Mac**: an app in full screen; the overlay does not appear in its full-screen space.
- **Linux**: the window manager decides the order of windows. A full-screen window in front stays above the overlay; some desktops, such as GNOME, do not move the overlay back to the top while it shows; on a pure Wayland session, the order is entirely up to the desktop.

### Live text while you speak

The words the overlay shows while you record come from the provider's real-time recognition or from the local model on this computer, and a tag names where they come from, such as **Preview · Doubao Speech**, **Preview · Built-in** or **Preview · Local model**. They are a preview only; the final text follows the [output mode](/dictation/output#output-modes).

<ScreenFigure src="/screens/overlay-live-en-light.webp" dark="/screens/overlay-live-en-dark.webp" width="1376" height="178"
  alt="The overlay while recording: the top line shows the words being spoken, Pull this logic out into a helper, followed by the tag Preview · Built-in; the bottom line shows the waveform, the timer, Microphone, Cloud and Esc Cancel."
  caption="With the built-in service, the overlay's tag reads Preview · Built-in." />

**Live preview** on the Recognition tab of the **Speech models** page has two switches, both on by default:

- **Provider's live text**: shows the built-in service's preview and a realtime model's words. With it off, the overlay shows none of them: a realtime model still recognises while you speak, with the same result and speed, and the built-in service sends no preview requests.
- **Local model preview**: when the provider gives no live text, or **Provider's live text** is off, the live transcription model on this computer shows the words; download it first (169 MB, see [Local recognition](/recognition/local)).

With both on, the provider's words are shown. The status line of **Live preview** names the source in use.

<ScreenFigure src="/screens/live-preview-en-light.webp" width="1440" height="900"
  alt="Live preview on the Recognition tab of the Speech models page: the status line reads In use · Built-in service, with the Provider's live text and Local model preview switches and the live transcription model's card below."
  caption="Live preview under Speech models → Recognition, with the built-in service." />

## Cancelling

Press <kbd>Esc</kbd> while recording or processing to cancel; the overlay shows the key in red for as long as it works. Nothing is inserted and nothing is saved in the history, except that with **Type as you speak** the sentences already typed stay where they are. Once the paste has begun, Esc no longer cancels: the dictation finishes, and the text is inserted and saved in the history as usual. Voltip listens for Esc only during a dictation, so it does not interfere with other apps the rest of the time.

On a pure Wayland session, bind a system shortcut to `voltip-desktop --cancel` instead.

## Copy the last result

Under **Settings → Shortcuts**, **Copy last result** takes a shortcut of its own, none by default. Pressing it puts the latest result on the clipboard without opening Voltip, and the overlay says **Copied the last result**. It must differ from the dictation and voice edit shortcuts. The tray menu on Windows and macOS has the same command, see [Copy or paste a result again](/dictation/output#copy-or-paste-a-result-again).

## Recording source

Under **Settings → Recording source**, or from the microphone menu under the page title (on every page but the home page), choose what a dictation records:

| Source | Records |
| --- | --- |
| **Microphone** (default) | The chosen microphone, or the system's default input. Choose the microphone from its name in the **Recognition and polish** card on the home page or under the page title, or under Settings → Recording source. |
| **Computer audio** | The sound your computer plays, such as a meeting, a call or a video, from the chosen output device or the system's default output. |
| **Mixed** | Both together. |

When mixing, Voltip removes the sound of the speakers from what the microphone records, so the computer's sound is not recorded twice. This is **Cancel speaker echo** under Settings → Recording source, on by default. At high speaker volume, headphones are still recommended. A voice edit always records the microphone.

Recording computer audio needs macOS 14.6 or later on a Mac, and a PulseAudio or PipeWire sound server on Linux; see [Platform notes](/reference/platforms). Where it is not available, the two options are dimmed and the reason is shown.

**“Ready to speak” sound**, on the same page, is off by default. When it is on, a short sound of about 0.1 seconds plays when the microphone starts recording, so you know you can speak. The sound is not transcribed: Voltip silences the short stretch of the recording while it plays, which silences anything said during it too, so start speaking after the sound. It plays only when the recording source is **Microphone**, since a recording of the computer's sound would contain it. With Bluetooth headphones, the system may switch to its call mode when the sound plays.

## Recording length

A recording stops by itself when it reaches **Longest recording** under Settings → Dictation: 1, 2, 5, 10 (the default) or 30 minutes, 1 hour or 2 hours.

A recording longer than 2 minutes is recognised in segments while you speak. Voltip cuts it where you pause, about every 20 to 30 seconds, and the overlay counts the segments; when you stop, only the last ones are left, so the text is ready soon after. A part that could not be recognised is marked in the text with its time, for example `[未识别 00:12:30–00:13:00]`.

- AI polish runs only when the text is 2,000 characters or shorter. A longer text is inserted as recognised; in the [history](/dictation/history#long-recordings) you can process it with an AI preset in parts.
- A text longer than 5,000 characters is not pasted. It stays on the clipboard.
- While a long recording runs, its audio is written to a file in Voltip's data folder. The file is deleted when the recording ends.

## Linux with Wayland

A pure Wayland session does not let apps register global shortcuts or watch single keys. Bind shortcuts in your desktop's keyboard settings to these commands instead:

| Command | Effect |
| --- | --- |
| `voltip-desktop --toggle` | Starts dictation, or stops it and inserts the text |
| `voltip-desktop --edit-toggle` | Starts or finishes a [voice edit](/dictation/voice-edit) |
| `voltip-desktop --cancel` | Cancels the current dictation |
| `voltip-desktop --copy-last` | Puts the latest result on the clipboard |

When Voltip runs as an AppImage, use the AppImage's path instead of `voltip-desktop`; Settings → Shortcuts shows the exact command. In a session that runs X11 apps through XWayland, the shortcut works only while an X11 window has the focus, so a system shortcut is the reliable choice there as well.
