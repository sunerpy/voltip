---
description: How Voltip inserts the text, the clipboard after a paste, what happens when you switch windows while it processes, the three output modes, what happens when pasting is not possible, how Voltip checks that the text arrived, the result cards, and how to copy or paste a result again.
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

Voltip inserts the text into the window you started in. If you switch to another window while it recognises or polishes, Voltip does not paste by default: it only copies the text, so that it does not land in the wrong place. The overlay shows **Window changed · text copied** as a [result card](#result-cards): click into the field you want and press **Paste**, or paste with <kbd>Ctrl</kbd> <kbd>V</kbd> (<kbd>⌘</kbd> <kbd>V</kbd> on a Mac). With **Type as you speak**, the sentences after the switch are no longer typed one by one; they are copied together at the end.

To paste into the window that is in front instead, set **If the window changes while processing**, under **Settings → Dictation**, to **Paste into current window**. Voice edit does not follow this setting: after a switch it always copies the rewrite only. Where Voltip cannot tell which window is in front when you start (on a pure Wayland session on Linux, for example), it pastes as usual. With **Copy to clipboard only**, the text is only copied anyway: switching windows changes nothing, and the overlay does not show this notice.

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

## Checking that the text arrived

After a paste, Voltip reads the input field that has the focus again, on this computer, and compares it with what it read right before the paste. When it is certain that the text did not arrive, because the same field still shows exactly what it showed before, the overlay says **Not written** and keeps the result on screen as a [result card](#result-cards), so that you can paste it into the right field. Voltip never pastes a second time by itself, so the text cannot end up there twice.

When Voltip cannot tell, it treats the text as inserted and shows nothing more. That is the case when the field cannot be read, another field has taken the focus, the field changed in another way (an app completed a word, for example), or the app is a terminal, a remote desktop or a virtual machine. When Voltip cannot tell which app the text went to (on a pure Wayland session on Linux, for example), it does not read the field at all. Password fields are never read. The history entry's **Technical details** then say **Not confirmed as written**.

- **Windows**: Voltip reads the field through UI Automation. The first reading makes apps built on Chromium (browsers such as Chrome, and many Electron apps) turn on their accessibility support until they are closed, which can slow down very large pages.
- **Mac**: through the Accessibility permission that pasting already needs.
- **Linux**: through the desktop's accessibility service (AT-SPI), only while accessibility is turned on in your session; Voltip does not turn it on. Apps built on Chromium can usually not be read.

To turn the check off, switch off **Check that the text was written** under **Settings → Dictation**. What Voltip reads stays in memory for the moment of the paste; it is not saved or uploaded.

## Result cards

When the text was not written, the window changed while it processed, or recognition failed, the overlay keeps the result as a card instead of letting it disappear:

<ScreenFigure src="/screens/overlay-card-en-light.webp" dark="/screens/overlay-card-en-dark.webp" width="1376" height="150" alt="The overlay's result card: Not written, with the Paste, Copy and Close buttons" />

| Situation | The card says | Buttons |
| --- | --- | --- |
| The text did not arrive | **Not written** | **Paste**, **Copy** |
| You switched windows while it processed | **Window changed · text copied** | **Paste**, **Copy** |
| A failure after the text was recognised | The reason | **Paste**, **Copy** |
| A failure before any text, with the recording kept | The reason · **Recording kept** | **Recognise again**, **Open history** |

- **Paste** pastes the text into the window in front. Click into the right field first: the overlay never takes the keyboard focus, so the window you were using stays in front.
- **Copy** puts the text on the clipboard, and the card says **Copied**.
- **Recognise again** recognises the kept recording again with the current speech model, and copies the result. **Open history** opens the History page.
- **×** closes the card. Starting a new dictation replaces it.

A card stays until you close it or start the next dictation. To let cards disappear after their usual time instead (6 seconds, or 2.5 seconds for a failure without text), switch off **Keep result cards on screen** under **Settings → Dictation**; the buttons stay the same. Other results disappear as before: text pasted, text left on the clipboard because pasting was not possible, no speech, and a cancelled dictation.

## Copy or paste a result again

On the home page's latest results (the buttons appear when you point at a row or move to it with the keyboard) and on the History page, every row has two buttons:

- **Copy this result** puts the text on the clipboard.
- **Paste into the previous window** hides Voltip, waits for the window you used before to come back to the front, and pastes the text there. If a different window comes to the front, the text is only copied.

For a voice edit, both buttons use the rewritten text.

**Copy last result** puts the latest result on the clipboard without opening Voltip, and the overlay says **Copied the last result**. It is in the tray menu on Windows and macOS (greyed until there is a result), on a shortcut you can set under **Settings → Shortcuts** (none by default), and on the command `voltip-desktop --copy-last`. Voltip keeps the latest result in memory until it quits, also when the history is turned off.
