---
description: The voltip-desktop command-line options, for controlling the running app and for recognising files without a window.
---

# Command line

The Voltip program, `voltip-desktop` (`voltip-desktop.exe` on Windows), accepts options for two purposes: controlling the Voltip that is already running, and recognising audio files without opening a window.

## Controlling the running app

These options are passed to the Voltip that is already running. Without a running Voltip, the program starts normally.

| Option | Effect |
| --- | --- |
| `--toggle` | Starts dictation, or stops it and inserts the text |
| `--edit-toggle` | Starts a [voice edit](/dictation/voice-edit) of the selected text, or finishes it |
| `--cancel` | Cancels the current recording or result |
| `--start-hidden` | Starts without showing the main window, with the tray and the shortcut only |
| `--quit` | Quits the running Voltip, as **Quit Voltip** in the tray menu does |

On Linux with Wayland, bind system shortcuts to `--toggle`, `--edit-toggle` and `--cancel`; see [Shortcuts and recording modes](/dictation/shortcuts#linux-with-wayland).

## Without a window

These options print their result and exit. They open no window, tray icon, shortcut or microphone, so they work in scripts and on servers.

| Option | Output |
| --- | --- |
| `--list-devices` | The input devices, one per line: identifier, tab, name; the default one first |
| `--list-models` | The local models and whether each is installed |
| `--list-compute` | The processor threads, and each graphics card this build can use |
| `--download-model <id>` | Downloads a model, resuming an earlier attempt, and checks its hash. Progress goes to standard error |
| `--transcribe-file <file.wav>` | Recognises a WAV file with a local model and prints the text |

`--transcribe-file` accepts these further options:

| Option | Effect |
| --- | --- |
| `--model <id>` | The model to use. Default: the one selected in the app, or Qwen3-ASR 0.6B |
| `--json` | Prints `{ "text", "model", "latency_ms", "backend" }` instead of the text alone |
| `--device auto\|cpu\|gpu` | Where the model runs. Default: the app's setting |
| `--gpu <name>` | Which graphics card, as listed by `--list-compute` |
| `--threads <n>` | How many processor threads to use |

The file should be 16 kHz mono; other sample rates are converted. The model identifiers are `qwen3-asr-0.6b`, `qwen3-asr-1.7b`, `sense-voice-small` and `paraformer-zh`.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Success |
| 1 | The model is not installed, or recognition failed; the reason is on standard error |
| 2 | The audio file could not be read |

## Example

```bash
voltip-desktop --download-model sense-voice-small
voltip-desktop --transcribe-file meeting-note.wav --model sense-voice-small --json
```

On Windows the program is built as a desktop app, so PowerShell does not show its output directly. Redirect it to a file instead: `voltip-desktop.exe --transcribe-file a.wav --json > out.json`.
