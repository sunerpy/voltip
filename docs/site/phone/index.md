---
description: Use an Android phone as a microphone and keyboard for your computer or on its own, see the history and settings your computer syncs to it, how pairing works, and how the connection is protected.
---

# Your phone as a microphone and keyboard

<StatusTag status="available" />

With the Voltip app for Android, a phone becomes a microphone and a keyboard for your computer. Hold to talk on the phone, and the text appears at the computer's cursor. With no computer online, the phone transcribes on its own.

## Install the app

Every [release](https://github.com/sunerpy/voltip/releases) carries the app as `Voltip_<version>_android_arm64.apk`, for phones with Android 8.0 or later and a 64-bit Arm processor. Open the file on the phone to install it. [Install](/guide/install#android) describes the steps and how to check the download.

In the app, the system back gesture or back button goes up one level. From **History** and **Settings** it goes to **Talk**, and two backs in a row on **Talk** leave the app. An open dialog closes first.

## Using the phone on its own

Before any computer is paired, and whenever no paired computer is online, **Hold to talk** works on the phone alone, and the button reads **Transcribe on the phone**. The built-in service transcribes and polishes what you said, and the text is copied to the phone's clipboard, ready to paste into any app.

The phone has settings of its own, under **Settings** at the bottom:

- **Speech models** and **AI models and presets**: the recognition service and the AI polish service. Use the built-in services, or enter your own provider, endpoint, model and API key. Keys are stored only in the phone's system keystore. The phone does not run local models.
- Whether AI polish runs, which preset it uses, the recognition language and the Chinese script.
- **Appearance and language** and **Recording**: the interface language, the theme and the longest take.
- **Dictionary**, **Replacement rules** and **Scenes**: correct the spellings recognition tends to get wrong, make fixed replacements in the text, and adjust AI polish, the preset, the language, the Chinese script and the extra instruction for the AI by scene. Rules can be imported by pasting a TOML file, and the exported TOML can be copied or shared.

The phone cannot tell which app you are typing in, so scenes are not matched by app. Choose one under **Scene** above **Hold to talk**, and the recordings the phone recognises follow it; choose **No scene** to stop.

These settings apply to recordings the phone recognises itself. A recording sent to a computer follows the computer's settings.

**Recent results** lists what the phone transcribed, newest first, with **Copy** and **Share**. **History** at the bottom lists everything: search it, filter it by time or by star, and see how many takes you made today, this week, this month and in total, and the time they saved. Open an entry to read the polished text and the transcript, and to copy, share, star or delete it; a long recording can also be processed again with an AI preset, and its subtitles (SRT) or text (TXT) shared to another app as a file. **Settings** › **History** turns saving off, sets how many entries are kept, or clears them all. These results are kept on the phone; while it syncs with a paired computer, a copy goes to the computer, see [The computer's history and settings](#the-computer-s-history-and-settings).

**Settings** › **Feedback** reports a problem or suggests an idea, with screenshots or screen recordings from the gallery; before anything is sent, the page lists everything that goes along. See [Privacy](/privacy).

When a paired computer is online, the button reads **Send to** and the computer's name instead, and the recording goes to the computer as described below. A recording keeps its route until it ends, even if a computer comes online or goes offline in the meantime.

## Pairing

Pairing connects one phone and one computer, once:

1. On the computer, open the **Phone** page and start pairing. It shows a QR code and a 6-digit code, valid for 120 seconds.
2. On the phone, scan the QR code, enter the 6-digit code, or tap the computer under **Computers nearby** when both are on the same network.
3. Both screens show the same safety code. Check that they match, then confirm on both devices. If they do not match, reject the pairing.

**Always-on pairing** on the computer keeps a pairing open for the next phone, renewing the QR code before it expires, until you turn it off. Every pairing still needs the safety code confirmed on the computer.

**LAN discovery**, on by default, lets phones on the same network find the computer by name and lets paired devices find each other again after an address change. Turned off, pairing works by QR code or 6-digit code only.

## Talking on the phone

Hold **Hold to talk** on the phone, speak, and release. The audio streams to the computer while you speak. The computer recognises it with its own settings — its recognition service, dictionary, AI polish, rules and scenes — and inserts the text at its own cursor. The phone shows each step and the result. Slide your finger off the button before releasing to cancel.

If several computers are paired and online, choose the one to send to.

## Sending text

The phone can also send text without speaking: type it, or press **Send clipboard** to send what is on the phone's clipboard. The computer inserts it at its cursor like a dictation result, without recognition or polish. When the computer is busy with a dictation, the text waits in a queue and is inserted afterwards.

## The computer's history and settings

A paired computer syncs its history and settings to the phone, where you can read them but not change them.

- Choose the computer's name at the top of **History** to see its history: search and filter it, open an entry to read the polished text, the transcript and where it came from, and copy or share it. The computer's entries cannot be starred or deleted on the phone. An entry longer than 50,000 characters cannot be copied or shared in one piece; a very long entry shows only part of its text on the phone, the computer has it all.
- **Settings** › **Computers** › **Computer name's settings** shows the computer's language and theme, speech model, AI polish, presets, dictionary, rules and scenes. The phone's own look and settings do not change with them.
- What the phone transcribes on its own is copied to a paired computer when the phone connects to it; with several computers paired, to the one that connected first. On the computer the copy is marked **Phone · phone name**. The phone keeps its own entries, and each side deletes its entries independently.
- A computer syncs with up to 5 phones, and a phone with up to 5 computers.
- On the computer's **Phone** page each phone has a **Sync** switch, on by default. Turned off, this computer's history and settings are deleted from the phone, and the phone stops uploading to this computer.
- This computer's history and settings are deleted from the phone when you forget the computer on the phone, when the computer stops syncing with the phone, or when the computer forgets the phone while the phone is online. If the computer forgets the phone while it is offline, they stay on the phone until you forget the computer there.
- Voltip on the computer must be a version that syncs; otherwise the phone shows **Voltip on the computer needs an update to sync**.

## How the connection is protected

- Pairing creates an encrypted connection and asks you to compare the safety code, so that nobody in between can pretend to be one of the devices.
- Afterwards, the devices connect directly on the same network. On different networks, an optional relay forwards their traffic. Everything is encrypted end to end, so the relay cannot read audio or text.
- Audio is compressed with Opus before it is sent.
- **Forget device** removes a pairing on both sides when the other device is online. The two can be paired again straight away.
- **Connection check** tests the network path to each paired device and reports where it fails.

The phone's own dictionary, rules and scenes apply only to what the phone recognises itself; the computer's settings synced to the phone are for reading and are never used to recognise on the phone.
