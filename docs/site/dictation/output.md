---
description: How Voltip inserts the text, the clipboard after a paste, what happens when you switch windows while it processes, the three output modes, what happens when pasting is not possible, and how to copy or paste a result again.
---

# Where the text goes

When a dictation finishes, Voltip inserts the text into the app that had the focus when you started. Two settings decide how.

## Paste or copy

Under **Settings → Dictation**, **Insert method** has two choices:

- **Paste at cursor** (default): Voltip puts the text on the clipboard, presses the paste keys for you (<kbd>Ctrl</kbd> <kbd>V</kbd>, or <kbd>⌘</kbd> <kbd>V</kbd> on a Mac) and then restores what was on the clipboard before.
- **Copy to clipboard only**: the text is left on the clipboard and you paste it yourself.

## The clipboard after a paste

To paste, Voltip borrows the clipboard for a moment. Whatever you had copied before, whether text, an image, files or formatted content, is put back once the paste is done; with **Type as you speak**, which pastes sentence by sentence, it is put back once, at the end. If you copy something else in the meantime, Voltip keeps what you copied.

Under **Settings → Dictation**, **Clipboard after inserting** decides what the clipboard holds after a paste:

- **Restore original** (default): what you had copied comes back.
- **Keep result**: the recognised text stays on the clipboard, so you can paste it again.

The text Voltip puts on the clipboard for a paste is kept out of the Windows clipboard history (<kbd>Win</kbd> <kbd>V</kbd>) and the cloud clipboard. On a Mac and on Linux, Voltip marks it with the transient-content flag that clipboard managers use; managers that honour the flag, such as KDE's Klipper, do not record it.

A few kinds of content cannot be put back in full: on Windows, formats such as metafiles that are provided as handles; on Linux, Voltip can read only files, images, formatted text and plain text, and when several are there it puts back one of them; and anything larger than 64 MiB at once.

## Switching windows while it processes

Voltip inserts the text into the window you started in. If you switch to another window while it recognises or polishes, Voltip does not paste by default: it only copies the text, so that it does not land in the wrong place. The overlay shows **Window changed · text copied**, and you can paste it where you want with <kbd>Ctrl</kbd> <kbd>V</kbd> (<kbd>⌘</kbd> <kbd>V</kbd> on a Mac). With **Type as you speak**, the sentences after the switch are no longer typed one by one; they are copied together at the end.

To paste into the window that is in front instead, set **If the window changes while processing**, under **Settings → Dictation**, to **Paste into current window**. Voice edit does not follow this setting: after a switch it always copies the rewrite only. Where Voltip cannot tell which window is in front when you start (on a pure Wayland session on Linux, for example), it pastes as usual.

## Output modes

On the **Speech models** page, the **Recognition** tab has three output modes:

| Mode | What happens |
| --- | --- |
| **All at once** (default) | After you release the shortcut, the whole recording is recognised, polished and inserted in one go. |
| **While you speak** | Sentences are settled while you speak; after you release, only the last one remains. AI polish still runs once, on the whole text. |
| **Type as you speak** | Each sentence is typed into the app as soon as it is settled. This mode does not use AI polish, and cancelling does not remove sentences already typed. |

The last two modes need text while you speak: with the built-in service or a [realtime model](/recognition/cloud#alibaba-cloud-model-studio) they work as they are; with another provider or a local model, download the live transcription model (169 MB) first. Until then they run as **All at once**, and the Recognition tab says so. With a realtime model, **All at once** runs as **While you speak**, since the words are recognised as you speak.

On the same tab, **Silence trimming** cuts the silence before and after your speech when recognising on your computer. The small model it needs is downloaded with the first local model.

## When the text cannot be pasted

If Voltip cannot paste, the text stays on the clipboard so that nothing is lost. The overlay and the history say why, in plain words, and you can paste it with <kbd>Ctrl</kbd> <kbd>V</kbd> (<kbd>⌘</kbd> <kbd>V</kbd> on a Mac). The usual reasons:

| Reason | What to do |
| --- | --- |
| On a Mac, Voltip does not have the Accessibility permission | Grant it in System Settings → Privacy & Security → Accessibility. The home page has a button that opens the setting. |
| The target window runs as administrator (Windows) | Windows does not let an ordinary app type into it. Run the target app normally, or paste by hand. |
| A password field or the Windows secure desktop has the focus | These refuse typed input from apps by design. |
| No paste tool on Linux with Wayland | Install a tool for your desktop; see [Platform notes](/reference/platforms#linux). |
| No connection to the display (Linux) | Voltip was started outside a graphical session. |
| The text of a long recording is longer than 5,000 characters | A text this long is not pasted; paste it where you want it. See [Recording length](/dictation/shortcuts#recording-length). |
| You switched to another window while it recognised or polished | Paste it where you want it. See [Switching windows while it processes](#switching-windows-while-it-processes). |

The details of the original error are available under **Technical details** in the history entry.

## Copy or paste a result again

On the home page's latest results (the buttons appear when you point at a row or move to it with the keyboard) and on the History page, every row has two buttons:

- **Copy this result** puts the text on the clipboard.
- **Paste into the previous window** hides Voltip, waits for the window you used before to come back to the front, and pastes the text there. If a different window comes to the front, the text is only copied.

For a voice edit, both buttons use the rewritten text.
