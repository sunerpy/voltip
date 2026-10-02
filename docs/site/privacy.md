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
| The phone on its own | The recording | The built-in service run by the Voltip project |

With the built-in service and the live preview on (the default), the sentence you are saying is sent about every second while you speak, to show the words in the overlay; if you then cancel the dictation, what was sent has already reached the built-in service. With **Live preview** off (**Speech models → Recognition**), the recording is sent only when you release the shortcut.

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

When no paired computer is online, the phone transcribes on its own: the recording goes to the built-in service for recognition and the recognised text to the built-in service for AI polish, as on a computer with the default settings. Its results are kept on the phone; while it syncs with a paired computer, a copy goes to that computer. See [Using the phone on its own](/phone/#using-the-phone-on-its-own).

A paired computer syncs its history and settings to the phone for reading there. They travel end-to-end encrypted, the relay cannot read them, and they are kept in Voltip's private storage on the phone. The computer's **Phone** page turns syncing off per phone; when the phone deletes them is described in [The computer's history and settings](/phone/#the-computer-s-history-and-settings).

## Feedback

The **Feedback** dialog on the computer and the **Feedback** page on the phone send only what they list before you send: your message, an optional contact, the attachments you add, and non-sensitive details such as the app version, the operating system and which kind of service is in use. It never sends dictation text, history, keys, the dictionary or the rules.

## Updates

Voltip checks for updates when you press **Check for updates**, or at start-up when automatic updates are on. The check reads the release information published on GitHub.
