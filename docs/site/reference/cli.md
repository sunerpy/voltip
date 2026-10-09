---
description: The voltip-desktop command-line options, for controlling the running app and for recognising files without a window, and the options of voltip-server, the local service for computers without a desktop.
---

# Command line

The Voltip program, `voltip-desktop` (`voltip-desktop.exe` on Windows), accepts options for two purposes: controlling the Voltip that is already running, and recognising audio files without opening a window.

## Controlling the running app

These options are passed to the Voltip that is already running. Without a running Voltip, `--quit` exits at once and the others start the program normally.

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

## voltip-server

`voltip-server` runs the [local service](/recognition/service) on a computer without a desktop. It is built for Linux x64 and installed with `install.sh --server`. Without options it listens on `127.0.0.1:47840` until it receives Ctrl+C or `SIGTERM`.

| Option | Effect |
| --- | --- |
| `--listen <address:port>` | Where to listen. Default: `127.0.0.1:47840`. An address other computers can reach also needs `--allow-remote` |
| `--allow-remote` | Allows such an address. The connection is not encrypted |
| `--token-file <path>` | The access token's file, created the first time. Default: `~/.local/share/voltip/serve/token` |
| `--scene <scene>` | The scene of every `voltip` request: a built-in one (`coding`, `office`, `chat`, `legal`, `medical`, `finance`, `academic`) or a scene's name |
| `--app <app id>` | Instead of `--scene`: the scene that lists this app, as if it were the app in front |
| `--preset <preset>` | The preset for AI polish: a built-in one (`proofread`, `prompt`, `intent`, `chat`, `translate`, `notes`, `punctuation`, `formal`) or a preset's name |
| `--refine on\|off` | AI polish for every `voltip` request. Default: the settings, or the scene's |
| `--language <code>` | The recognition language of every request (`zh`, `en`, …; `auto` leaves it to the model) |
| `--script simplified\|traditional\|as_is` | The Chinese script |
| `--asr <provider>`, `--llm <provider>` | Another recognition or AI polish provider, for example `builtin`, `local` or `aliyun` |
| `--local-model <id>`, `--device`, `--gpu`, `--threads` | Recognition with a local model, as with `--transcribe-file` |
| `--max-minutes <1–120>` | The longest audio of one request. Default: 120 |
| `--concurrency <1–8>` | Requests processed at a time. Default: 2 |
| `--no-preload` | Does not load the local model at start; the first request waits for it |

These options print their result and exit:

| Option | Output |
| --- | --- |
| `--check` | The services, the default processing, the limits and any warnings |
| `--print-token` | The access token, created the first time |
| `--list-models`, `--download-model <id>`, `--list-compute` | The same as for `voltip-desktop` |

| Code | Meaning |
| --- | --- |
| 0 | The service stopped normally, or `--check` found that recognition can run |
| 1 | The service could not start, or recognition is not available |
| 2 | Options that do not go together, or an unknown option |

Without a system keychain, provider keys can come from the environment, which wins over the keychain: `VOLTIP_KEY_OPENAI`, `VOLTIP_KEY_GROQ`, `VOLTIP_KEY_SILICONFLOW`, `VOLTIP_KEY_ALIYUN`, `VOLTIP_KEY_DEEPSEEK`, `VOLTIP_KEY_CUSTOM_ASR`, `VOLTIP_KEY_CUSTOM_LLM`. See [Local service](/recognition/service#on-a-server-without-a-desktop).
