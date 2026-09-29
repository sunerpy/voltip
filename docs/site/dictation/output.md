---
description: How Voltip inserts the text, the three output modes, what happens when pasting is not possible, and how to copy or paste a result again.
---

# Where the text goes

When a dictation finishes, Voltip inserts the text into the app that had the focus when you started. Two settings decide how.

## Paste or copy

Under **Settings → Dictation**, **Insert method** has two choices:

- **Paste at cursor** (default): Voltip puts the text on the clipboard, presses the paste keys for you (<kbd>Ctrl</kbd> <kbd>V</kbd>, or <kbd>⌘</kbd> <kbd>V</kbd> on a Mac) and then restores what was on the clipboard before.
- **Copy to clipboard only**: the text is left on the clipboard and you paste it yourself.

## Output modes

On the **Speech models** page, the **Recognition** tab has three output modes:

| Mode | What happens |
| --- | --- |
| **All at once** (default) | After you release the shortcut, the whole recording is recognised, polished and inserted in one go. |
| **While you speak** | Sentences are settled while you speak; after you release, only the last one remains. AI polish still runs once, on the whole text. |
| **Type as you speak** | Each sentence is typed into the app as soon as it is settled. This mode does not use AI polish, and cancelling does not remove sentences already typed. |

The last two modes need the live transcription model (169 MB). Until it is downloaded they run as **All at once**, and the Recognition tab says so.

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

The details of the original error are available under **Technical details** in the history entry.

## Copy or paste a result again

On the home page's latest results and on the History page, every row has two buttons:

- **Copy this result** puts the text on the clipboard.
- **Paste into the previous window** hides Voltip, waits for the window you used before to come back to the front, and pastes the text there. If a different window comes to the front, the text is only copied.

For a voice edit, both buttons use the rewritten text.
