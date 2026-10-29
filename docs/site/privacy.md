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

With the built-in service and the live preview on (the default), the sentence you are saying is sent about every second while you speak, to show the words in the overlay; if you then cancel the dictation, what was sent has already reached the built-in service. With a provider's realtime model, such as Alibaba Cloud Model Studio's, and the live preview on, the recording goes to that provider continuously while you speak; nothing more is sent once you cancel. With **Live preview** off (**Speech models → Recognition**), the recording is sent only when you release the shortcut.

With **When a model's quota runs out** turned on, the recording goes to the provider of the next model in the list once the selected model's quota is used up, and so does the text for AI polish. While it is on, the note on what is sent, on the Speech models and AI models pages of the computer and the phone, names these providers.

With a local model, the only network use is downloading the model file, which is verified against a SHA-256 hash.

## AI polish and voice edit

When AI polish is on, the recognised text is sent to the AI service you chose. For a voice edit, the selected text and your instruction are sent. Audio is never sent to the AI service. The request can also include:

- the name of the app you are dictating into — on by default, can be turned off;
- the window title — off by default;
- the extra instructions of the scene that applies;
- your dictionary's correct spellings.

The switches are under **Settings → Scenes**, in **Context sent to AI polish**. With AI polish off, or in the **Type as you speak** output mode, no text is sent for polishing.

With Google AI Studio and a key on its free tier, Google may use the text it receives to improve its products, as its terms for unpaid services say; a key from a Google Cloud project with billing turned on is not used that way.

## The built-in service

The built-in service's server keeps a record of each request: the time, your IP address, whether it was recognition or AI polish (for AI polish, which of its models the text was sent to), whether it succeeded, how large it was and how long it took, and the app name and version the app sends with it. It does not keep the recording or the text.

The records are kept for 30 days. They are used to limit how many requests one address can make, so that no single address uses up the shared service, and to count how much the service is used; after that only daily totals without IP addresses are kept. Over the limit, the built-in service answers that it is busy, and AI polish then suggests your own provider. People who share one internet connection, for example in an office, share the limit.

The built-in service also counts each device's use by day (see [Built-in usage](/recognition/cloud#built-in-usage)). For that, Voltip generates a device number at random when a device first uses the service and sends it with its requests to the built-in service, never to other providers. The device number has nothing to do with the device identity used for pairing; it is not included in backups and not synced to other devices. The built-in service keeps, by device number and day, how long the device's recognition ran and how many AI polishes it made that day, for the daily limits and for counting usage; its statistics use one number per device, without the device number or the IP address. Versions 0.1.9 and earlier send no device number and are counted by IP address. Voltip keeps its own record of the last 35 days on the device (how long each provider recognised and how many polishes it made, without recordings or text) for the **Usage** page.

## Try it online

The [try page](/guide/try) on this website sends what you record or upload to the built-in service for recognition, and the text you polish to the built-in service for AI polish. The built-in service keeps the same record of these requests as of the app's, with your IP address, and does not keep the recording or the text. To keep the page's limits, the website counts your requests under a one-way hash of your IP address; the counts are removed after the hour they cover. Before you start, Cloudflare Turnstile checks that a person is using the page, under Cloudflare's privacy policy. After the check, your browser keeps a cookie for 30 minutes so that the check is not repeated; the page keeps nothing else.

## The microphone and computer audio

The microphone is open only while you record, or for the 15 seconds of a microphone test. Computer audio is recorded only during a dictation with it chosen as the source. Voltip does not listen in the background.

With the [floating bubble](/phone/#floating-bubble) on, Voltip shows a talk button over other apps as a foreground service, with a notification that closes it always in the shade. The microphone opens only while you hold or tap the bubble to record; the result is copied to the phone's clipboard, and the bubble does not read the clipboard.

[One-gesture paste](/phone/#one-gesture-paste) is optional: it needs your agreement in the app and Voltip's paste service turned on in Android's Accessibility settings. With it on, Voltip looks up which app and window the focused text field belongs to only when you hold the bubble to record and slide toward the middle of the screen. When the text is ready and the same field still has focus, Voltip asks that field to paste the text once. The service does not follow what you do in other apps, does not read the text in fields and skips password fields. What it looks up stays in the phone's memory for that one recording; it is not saved and not uploaded. You can turn the service off in Android's Accessibility settings at any time.

While a recording longer than 2 minutes runs, its audio is written to a file in Voltip's data folder so that it can be recognised in segments; the file is deleted when the recording ends. With **Keep recordings on this computer** on, a compressed copy is written as well and moves into the recording cache when the recording ends; if Voltip was closed during the recording, the next start completes the copy from what was recorded and moves it into the recording cache. With it off, the files a closed recording left are deleted at the next start.

With **Keep recordings on this computer** on (the default), the recordings of dictations are kept in Voltip's data folder, those longer than 2 minutes compressed, so that they can be [recognised again](/dictation/history#recognise-or-polish-again) when recognition or polish fails. Voltip keeps as many as the settings allow (20 recordings and 200 MB by default) and deletes the oldest past that. They stay on this computer: they are not synced to phones, not included in backups, and [AI tools](/recognition/mcp) cannot read them. Turn them off or delete them under **Settings → Privacy and history**, in **Recording cache**; turning them off deletes the kept recordings, and deleting an entry or clearing the history deletes its recordings too.

## The input field after a paste

To check that a paste arrived, Voltip reads the input field that has the focus on this computer right before and right after the paste, and compares its length, the caret and the selection, the selected text and up to 64 characters before the caret. To get them it may read more: on Windows, and in fields that only give their whole content, Voltip reads all of the field's text (up to about a million characters; longer text is not read) and works these out from it. What it reads stays in memory for the moment of the paste and is then discarded; it is never saved, uploaded or written to the log, which records only the outcome, how long the check took and the app's identifier. Password fields, terminals and remote desktops are not read, nor is a field when Voltip cannot tell which app the text went to. Switch the check off with **Check that the text was written** under **Settings → Dictation**; see [Checking that the text arrived](/dictation/output#checking-that-the-text-arrived).

## The local service

When the [local service](/recognition/service) is on, programs on this computer that have its access token can send recordings to Voltip. Each recording is processed like a dictation with the same settings: it goes to the recognition service in use, and its text to the AI service when AI polish applies. While a request is processed, its audio is kept in a temporary file in Voltip's data folder, which is deleted when the request ends. Requests are not added to the history, and the service's log records durations and the models used, never the text or the audio.

## AI tools

With [AI tools](/recognition/mcp) turned on, AI tools on this computer that hold its access token can read what you allowed: by default only the dictionary, the rules and the words AI polish replaced (with 12 characters on each side of each hit); only when you also allow **Full history** can they read the original and polished text of every entry in the period you chose. Whatever an AI tool reads is sent to the model service it uses and may stay in its conversation history. Voltip records which tool read or changed what, when and how many items, never the text itself. The server is off by default and accepts connections from this computer only.

## What stays on your computer

- **History**: every dictation, stored in Voltip's data folder. It can be limited, turned off or cleared under **Settings → Privacy and history**.
- **Recordings**: with **Keep recordings on this computer** on, the recordings of your latest dictations, in the same folder; see [The microphone and computer audio](#the-microphone-and-computer-audio).
- **Dictionary, rules, scenes and settings**: in the same folder.
- **Backup files**: a backup exported under **Settings → Privacy and history** is saved where you choose. It holds the history, the dictionary, the rules, the scenes and your own AI presets, but no settings or keys, and it is not encrypted; see [Backup and moving to another computer](/dictation/history#backup-and-moving-to-another-computer).
- **Keys and the device identity**: in the system keychain — Windows Credential Manager, the macOS keychain, or the Secret Service on Linux. Keys are never shown again after you save them, and each key is sent only to its provider. Voltip does not fall back to storing keys in a file.

[Updates, uninstall and your data](/guide/updates#where-your-data-is-kept) lists the folders.

## The phone link

Paired devices talk over an end-to-end encrypted connection; when you pair by scanning, the QR code checks both devices, and when you enter the code, you confirm a safety code. An optional relay forwards traffic between different networks without being able to read it. See [Your phone as a microphone and keyboard](/phone/#how-the-connection-is-protected), and the [threat model](/dev/threat-model) for the technical detail.

When no paired computer is online, the phone transcribes on its own: the recording goes to the built-in service for recognition and the recognised text to the built-in service for AI polish, as on a computer with the default settings. Its results are kept on the phone; while it syncs with a paired computer, a copy goes to that computer. See [Using the phone on its own](/phone/#using-the-phone-on-its-own).

For a take the phone sends to a computer, the computer reports the inserted text back, and the phone keeps a copy in its own history (subject to **Settings** › **History** on the phone). A paired computer syncs its history and settings to the phone for reading there. They travel end-to-end encrypted, the relay cannot read them, and they are kept in Voltip's private storage on the phone. The computer's **Phone** page turns syncing off per phone; when the phone deletes them is described in [The computer's history and settings](/phone/#the-computer-s-history-and-settings).

## Feedback

The **Feedback** dialog on the computer and the **Feedback** page on the phone send only what they list before you send: your message, an optional contact, the attachments you add, and non-sensitive details such as the app version, the operating system and which kind of service is in use. It never sends dictation text, history, keys, the dictionary or the rules.

## Updates

Voltip checks for updates when you press **Check for updates**, or, when automatic updates are on, at start-up and every 4 hours while it runs. The check reads the release information published on GitHub, from this site first and from GitHub when this site cannot be reached; an update is downloaded from the same place. This site runs on Cloudflare, which, like GitHub, sees the network address of the request; the check sends nothing about you or your recordings. On Android, a copy installed from Google Play makes no such check: Google Play updates it. Voltip 0.0.50 for Android makes none at all.

## Network proxy

When Voltip uses a proxy (the system's, when it follows the system, or one set under **Settings → Network**; see [Network proxy](/recognition/cloud#network-proxy)), its connections go through that proxy server. The proxy server sees which servers Voltip connects to, and how much data goes when. Connections to addresses that start with `https://` (the built-in service, the providers' default addresses, the update server and the feedback service) use TLS, so the proxy can neither read nor change their content; between your computer and your phone the content is end-to-end encrypted on top of that. When a custom provider's address starts with `http://`, the connection is not encrypted and the proxy can read and change the recording, the text and the key: use an `https://` address for it, or add it to **No proxy for**. Addresses on this device never go through the proxy. The proxy's password is stored only in the system keychain, never in the settings file or the logs.
