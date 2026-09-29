---
description: Rewrite selected text by voice in any app with Ctrl+Alt+E, what it needs, and where it does not work.
---

# Voice edit

Voice edit rewrites text you have selected. Select a passage in any app, hold <kbd>Ctrl</kbd> <kbd>Alt</kbd> <kbd>E</kbd>, say what to change, and release. The selection is replaced with the result.

Instructions can be anything you would ask an editor, for example:

- "Make it more formal."
- "Translate it into English."
- "Shorten it by half."
- "Turn it into a bulleted list."

The overlay shows **Edit** while you speak, and the history keeps the instruction, the original selection and the result.

## What it needs

- **An AI service.** Voice edit uses the same service as [AI polish](/recognition/polish), whether that is the built-in service or your own provider. It works even when the AI polish switch is off.
- **Selected text of up to 2,000 characters.** Without a selection, the voice edit stops and nothing is sent.

To change the shortcut, or turn it off, open **Settings → Shortcuts**. On Linux with Wayland, bind a system shortcut to `voltip-desktop --edit-toggle`.

## How the selection is read

Voltip copies the selection with a copy shortcut, reads it, and puts your clipboard back as it was:

- On Windows and Linux it uses <kbd>Ctrl</kbd> <kbd>Insert</kbd>, because <kbd>Ctrl</kbd> <kbd>C</kbd> interrupts the running program in a terminal.
- On a Mac it uses <kbd>⌘</kbd> <kbd>C</kbd>.

The result is pasted over the selection, which is still highlighted because the focus never leaves the app.

## Where it does not work

- **Terminals on Windows and Linux.** Text in a terminal cannot be replaced, so voice edit refuses to start when a terminal has the focus. It cannot recognise the terminal panel inside an editor such as VS Code; there, avoid starting a voice edit.
- **Controls that ignore the copy shortcut**, such as some custom-drawn fields: Voltip finds no selection and leaves everything unchanged.
- **Editors that copy the whole line when nothing is selected**, such as VS Code: the result is pasted at the cursor instead of replacing a selection.
- On some European keyboard layouts on Windows, <kbd>Ctrl</kbd> <kbd>Alt</kbd> <kbd>E</kbd> is the same as AltGr+E, which types €. Choose a different shortcut there.

On X11 with **Hold to talk**, the selection can only be read after you release the keys, so a missing selection is reported after you have spoken. The **Press to start, again to stop** mode reports it straight away.

## Privacy

The selection and your instruction are sent to the AI service, the same destination as AI polish, and only when you start a voice edit with a selection. The history on your computer keeps the instruction, the selection and the result; you can delete them there.
