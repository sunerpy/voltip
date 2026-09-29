---
description: The dictation shortcut, the three recording modes, starting with a single key or a mouse button, the overlay, and cancelling with Esc.
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

For the third mode, **Short-press threshold** sets how long a press may last and still count as short (300 ms by default). In every mode, **Keep recording after release** keeps the microphone open for up to five seconds after you stop, so the last word is not cut off.

## A single key or mouse button

A single key can start dictation on its own, in addition to the shortcut: Right Ctrl, Right Alt, Right Shift, the right Windows, Command or Super key, the Fn key on a Mac, or the middle, back or forward mouse button. It follows the same recording mode as the shortcut.

- Pressing another key or clicking while you hold it cancels the recording that the press started, so shortcuts such as Right Ctrl + C keep working.
- On many European keyboard layouts Right Alt is AltGr, and Right Shift is used for capital letters; the setting warns when you choose one of these.
- On a Mac, set **When pressing the 🌐 key** to **Do Nothing** under System Settings → Keyboard before using Fn.
- On a Mac the single key needs the Accessibility permission, which Voltip already asks for to paste.
- A pure Wayland session does not allow it.

## The overlay

While you dictate, a small overlay at the bottom of the screen shows what is happening:

- **Recording**: the input level, the elapsed time and, with the live transcription model installed, the words recognised so far. A padlock appears when a recording is locked.
- **Processing**: the step in progress — recognition, AI polish or insertion — with the time it has taken.
- **Done**: how many characters were inserted, or why the text stayed on the clipboard.

The overlay never takes the keyboard focus, so the text still goes to the app you were using.

## Cancelling

Press <kbd>Esc</kbd> while recording or processing to cancel; the overlay shows the key in red for as long as it works. Nothing is inserted and nothing is saved in the history, except that with **Type as you speak** the sentences already typed stay where they are. Voltip listens for Esc only during a dictation, so it does not interfere with other apps the rest of the time.

On a pure Wayland session, bind a system shortcut to `voltip-desktop --cancel` instead.

## Recording length

A recording stops by itself after 2 minutes with the **All at once** output mode, and after 10 minutes with the two modes that transcribe while you speak. Longer recordings, of up to two hours, are in development; see the [roadmap](/roadmap).

## Linux with Wayland

A pure Wayland session does not let apps register global shortcuts or watch single keys. Bind shortcuts in your desktop's keyboard settings to these commands instead:

| Command | Effect |
| --- | --- |
| `voltip-desktop --toggle` | Starts dictation, or stops it and inserts the text |
| `voltip-desktop --edit-toggle` | Starts or finishes a [voice edit](/dictation/voice-edit) |
| `voltip-desktop --cancel` | Cancels the current dictation |

When Voltip runs as an AppImage, use the AppImage's path instead of `voltip-desktop`; Settings → Shortcuts shows the exact command. In a session that runs X11 apps through XWayland, the shortcut works only while an X11 window has the focus, so a system shortcut is the reliable choice there as well.
