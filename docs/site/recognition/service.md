---
description: Let other programs on this computer, such as Paseo's dictation from a phone, use Voltip's recognition, dictionary and AI presets through an OpenAI-compatible API, from the app or from a server without a desktop.
---

# Local service

The local service lets other programs on this computer use Voltip's speech recognition, dictionary, scenes and AI presets through an OpenAI-compatible API. A program sends a recording and gets the text back, processed the same way as a dictation in the app: recognition, the dictionary, AI polish with a preset, then the rules. Nothing is inserted at the cursor, and nothing is added to the history.

There are two ways to run it, with the same API:

- **In the app**: turn it on in the settings. It runs inside Voltip and shares the loaded model, the settings and the keys with dictation.
- **On a server without a desktop**: `voltip-server` is a separate program for Linux x64 that needs no window, graphics or sound system.

## In the app

Open **Settings → Local service**:

- **Turn on the local service** starts it; **Status** shows **Running** and the **API address**, for example `http://127.0.0.1:47840/v1`. Enter that address as the OpenAI base URL in the other program.
- **Port** changes the port (1024–65535); the service restarts on the new one. When the port is taken, the status says **Could not start** with the reason.
- **Processing** sets how requests are handled by default: the **Preset** for AI polish (**Same as the global setting**, or a preset) and the **Scene** (**No scene**, or a scene, whose preset, terms and extra instructions then apply as well). A request can still choose another preset or scene, see [Choosing the processing](#choosing-the-processing).
- **Copy token** puts the access token on the clipboard; the window never shows it. **Generate new** replaces it after a confirmation: the old token stops working at once, and every program set up with it needs the new one.

The service stops when you turn it off or quit Voltip. It accepts connections from this computer only.

## On a server without a desktop

Install `voltip-server` with the install script; it needs no administrator rights:

```bash
curl -fsSL https://raw.githubusercontent.com/sunerpy/voltip/main/scripts/install.sh | sh -s -- --server
```

The script checks the download against the release's `SHA256SUMS`, unpacks it to `~/.local/share/voltip-server/<version>` and links the command as `~/.local/bin/voltip-server`. When a system library is missing, it names the package to install, for example `sudo apt-get install libblas3`, and installs nothing itself. To upgrade, run the script again.

Then check the configuration, and start the service:

```bash
voltip-server --check          # the services, the defaults and the limits; exit code 0 when recognition can run
voltip-server --print-token    # the access token, created the first time
voltip-server --scene coding --preset prompt --language zh
```

The service runs in the foreground and stops on Ctrl+C or `SIGTERM`, after the requests in progress have finished (at most 30 seconds). All options are listed under [Command line](/reference/cli#voltip-server).

- **Settings**: `voltip-server` reads the same data folder as the app, `~/.local/share/voltip`. To use your presets, scenes, dictionary and rules, copy the app's `settings.json`, `presets.json`, `scenes.json`, `dictionary.json` and `rules.json` into it. The server only reads these files and picks up changes before the next request.
- **Services**: release builds include the built-in recognition and AI polish service, which needs no key. A local model works too: `voltip-server --download-model qwen3-asr-0.6b`, then `voltip-server --local-model qwen3-asr-0.6b`.
- **Certificates**: cloud recognition and AI polish, the built-in service included, check the server's certificate with the system's CA certificates. Minimal systems such as container images may not have them; then `--check` reports recognition as unavailable and the service does not start until `ca-certificates` is installed.
- **Keys**: keys for other providers are read from the system keychain, without changing it. A server without a desktop session usually has no keychain; there, providers that need a key are shown as not configured by `--check`.
- **Running in the background**: install with `sh -s -- --server --systemd-user --enable` to also write a systemd user service and start it. To keep it running after you log out, run `loginctl enable-linger`.

## Connecting a program

| Setting in the other program | Value |
| --- | --- |
| Base URL | `http://127.0.0.1:47840/v1` (the API address in the app) |
| API key | The access token |
| Model | `voltip` |

For example, with `curl`:

```bash
curl http://127.0.0.1:47840/v1/audio/transcriptions \
  -H "Authorization: Bearer $(voltip-server --print-token)" \
  -F file=@note.wav -F model=voltip -F language=zh
```

The answer is `{"text": "…"}`. With `-F response_format=text` it is the text alone; with `verbose_json` it also names the preset, the scene and the models that were used. A recording in which nothing was said gives an empty text.

### Choosing the processing

The `model` field chooses how a request is processed:

| `model` | Processing |
| --- | --- |
| `voltip`, or any other name such as `whisper-1` | The defaults: the preset and scene set in the app, or the options `voltip-server` was started with |
| `voltip:raw` | No AI polish |
| `voltip:preset=prompt` | AI polish with this preset |
| `voltip:scene=coding` | This scene, with its preset, terms and extra instructions |
| `voltip:scene=coding;preset=notes` | This scene, with another preset |

A request that names its own processing uses nothing of the defaults. Presets and scenes are named by their identifier, or by the name they have in the app. `GET /v1/models` lists every choice, each with a `model` value that can be sent as it is.

The `language` field gives the recognition language, for example `zh` or `en`; `auto` leaves the choice to the model. A scene's language, or the `--language` option of `voltip-server`, takes precedence.

## Paseo

Paseo can use the local service for dictation from its phone app. In Paseo's `~/.paseo/config.json`:

```json
{
  "providers": { "openai": { "stt": { "baseUrl": "http://127.0.0.1:47840/v1", "apiKey": "<access token>" } } },
  "features": { "dictation": { "stt": { "provider": "openai", "model": "voltip", "language": "zh" } } }
}
```

- Set `language`: without it, Paseo asks for English.
- Paseo reads these settings when its daemon starts; `paseo daemon reload` does not apply them. Restart the daemon after editing the file: `paseo daemon restart`. The restart also ends the agents the daemon is running.
- Paseo cuts a dictation into a new piece after every 15 seconds of audio, wherever the speaker is, and joins the pieces with spaces. Setting `PASEO_DICTATION_AUTO_COMMIT_SECONDS=0` in the environment of the Paseo daemon sends the whole dictation at the end instead; the daemon takes the environment of the command that starts it, for example `PASEO_DICTATION_AUTO_COMMIT_SECONDS=0 paseo daemon restart`. With a slow local model and long dictations, `110` lets each piece be recognised while you are still speaking.
- `model` may be any value from `GET /v1/models`, for example `voltip:scene=coding`.

## Limits

- **Audio**: WAV only, at 8 to 192 kHz, with 1 to 8 channels. One request takes up to 2 hours of audio (`voltip-server --max-minutes` lowers this). Recordings longer than 2 minutes are recognised in segments, as long recordings in the app are; AI polish is skipped when the text is longer than 2,000 characters.
- **Requests**: 2 are processed at a time and up to 4 more wait; when every place is taken, the service answers `503` and the program should try again a few seconds later. `voltip-server --concurrency` allows up to 8 at a time, with twice as many waiting.
- **Answers**: the text comes in one piece when processing has finished. There are no timestamps or subtitle formats.

## Security

- The service accepts connections from this computer only. `voltip-server --listen` can open it to other computers only together with `--allow-remote`; the connection is then unencrypted, so prefer an SSH tunnel or a TLS proxy.
- Every request must carry the access token. The token is stored in `~/.local/share/voltip/serve/token`, readable by your user only, and shared by the app and `voltip-server`.
- What a request sends onwards is the same as for a dictation with the same settings: the recording goes to the recognition service in use, the text to the AI service. See [Privacy](/privacy#the-local-service).
