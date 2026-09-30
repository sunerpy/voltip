---
description: What Voltip sends where in each mode, where it keeps keys and history, and what the feedback dialog sends.
---

# Privacy

What leaves your computer depends on the services you choose. The home page and the Speech models page always show which recognition service is in use.

## Recognition

| Mode | What is sent | Where to |
| --- | --- | --- |
| Local model | Nothing | — |
| Built-in service | The recording, and your dictionary's correct spellings as a hint | The built-in service run by the Voltip project |
| Cloud provider | The recording, the model name, your key, and your dictionary's correct spellings as a hint | The provider you chose, and no one else |

With a local model, the only network use is downloading the model file, which is verified against a SHA-256 hash.

## AI polish and voice edit

When AI polish is on, the recognised text is sent to the AI service you chose. For a voice edit, the selected text and your instruction are sent. Audio is never sent to the AI service. The request can also include:

- the name of the app you are dictating into — on by default, can be turned off;
- the window title — off by default;
- the extra instructions of the scene that applies;
- your dictionary's correct spellings.

The switches are under **Settings → Scenes**, in **Context sent to AI polish**. With AI polish off, or in the **Type as you speak** output mode, no text is sent for polishing.

## The microphone and computer audio

The microphone is open only while you record, or for the 15 seconds of a microphone test. Computer audio is recorded only during a dictation with it chosen as the source. Voltip does not listen in the background.

While a recording longer than 2 minutes runs, its audio is written to a file in Voltip's data folder so that it can be recognised in segments. The file is deleted when the recording ends, and at the next start if Voltip was closed during a recording.

## What stays on your computer

- **History**: every dictation, stored in Voltip's data folder. It can be limited, turned off or cleared under **Settings → Privacy and history**.
- **Dictionary, rules, scenes and settings**: in the same folder.
- **Keys and the device identity**: in the system keychain — Windows Credential Manager, the macOS keychain, or the Secret Service on Linux. Keys are never shown again after you save them, and each key is sent only to its provider. Voltip does not fall back to storing keys in a file.

[Updates, uninstall and your data](/guide/updates#where-your-data-is-kept) lists the folders.

## The phone link

Paired devices talk over an end-to-end encrypted connection; you confirm a safety code when pairing. An optional relay forwards traffic between different networks without being able to read it. See [Your phone as a microphone and keyboard](/phone/#how-the-connection-is-protected), and the [threat model](/dev/threat-model) for the technical detail.

## Feedback

The **Feedback** dialog sends only what it lists before you send: your message, an optional contact, the attachments you add, and non-sensitive details such as the app version, the operating system and which kind of service is in use. It never sends dictation text, history, keys, the dictionary or the rules.

## Updates

Voltip checks for updates when you press **Check for updates**, or at start-up when automatic updates are on. The check reads the release information published on GitHub.
