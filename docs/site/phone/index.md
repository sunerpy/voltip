---
description: Use an Android phone as a microphone and keyboard for your computer or on its own, see the history and settings your computer syncs to it, how pairing works, and how the connection is protected.
---

# Your phone as a microphone and keyboard

<StatusTag status="available" />

With the Voltip app for Android, a phone becomes a microphone and a keyboard for your computer. Hold to talk on the phone, and the text appears at the computer's cursor. With no computer online, the phone transcribes on its own.

## Install the app

Every [release](https://github.com/sunerpy/voltip/releases) carries the app as `Voltip_<version>_android_arm64.apk`, for phones with Android 8.0 or later and a 64-bit Arm processor. Open the file on the phone to install it. [Install](/guide/install#android) describes the steps and how to check the download, and [Updates](/guide/updates#on-an-android-phone) how to update it later.

From 0.0.50 the app's screens use Android's own controls. Install it over an earlier version to update; the pairing, settings and history stay. See [Install](/guide/install#android).

In the app, the system back gesture or back button goes up one level. From **History** and **Settings** it goes to **Talk**, and two backs in a row on **Talk** leave the app. An open dialog closes first.

## Using the phone on its own

Before any computer is paired, and whenever no paired computer is online, **Hold to talk** works on the phone alone, and the card says *Let go and the phone transcribes and polishes it, then copies the result*. The built-in service transcribes and polishes what you said, and the text is copied to the phone's clipboard, ready to paste into any app.

The phone has settings of its own, under **Settings** at the bottom:

- **Speech models** and **AI models and presets**: the recognition service and the AI polish service. Use the built-in services, or enter your own provider, endpoint, model and API key. Keys are stored only in the phone's system keystore. The phone does not run local models.
- Whether AI polish runs, which preset it uses, the recognition language and the Chinese script.
- **Appearance and language** and **Recording**: the interface language, the theme and the longest take.
- **Dictionary**, **Replacement rules** and **Scenes**: correct the spellings recognition tends to get wrong, make fixed replacements in the text, and adjust AI polish, the preset, the language, the Chinese script and the extra instruction for the AI by scene. Rules can be imported by pasting a TOML file, and the exported TOML can be copied or shared.

The phone cannot tell which app you are typing in, so scenes are not matched by app. Choose one under **Scene** above **Hold to talk**, and the recordings the phone recognises follow it; choose **No scene** to stop.

These settings apply to recordings the phone recognises itself. A recording sent to a computer follows the computer's settings.

With the built-in service, the bottom of the **Hold to talk** card shows today's use, and **Settings** › **Usage** has both services in detail. The built-in service counts each device's use by day: recordings the phone recognises itself count toward the phone, recordings sent to a computer toward the computer; see [Built-in usage](/recognition/cloud#built-in-usage). Once today's built-in recognition is used up, **Hold to talk** is unavailable for the day: send the recording to a computer that is online instead, or choose another provider under **Speech models**.

**Recent results** shows the newest three of what the phone transcribed and the takes it sent to a computer, each with its time and where it went: inserted on a computer, or transcribed on the phone. Open one to see its details and share it, or copy it with the button at its end. **History** at the bottom lists everything: search it, filter it by time or by star, and see how many takes you made today, this week, this month and in total, and the time they saved, takes sent to a computer included. Open an entry to read the polished text and the transcript, and to copy, share, star or delete it; a long recording can also be processed again with an AI preset, and its subtitles (SRT) or text (TXT) shared to another app as a file. **Settings** › **History** turns saving off, sets how many entries are kept, or clears them all. These results are kept on the phone; while it syncs with a paired computer, a copy of what the phone recognised itself goes to the computer, see [The computer's history and settings](#the-computer-s-history-and-settings).

**Settings** › **Feedback** reports a problem or suggests an idea, with screenshots or screen recordings from the gallery; before anything is sent, the page lists everything that goes along. See [Privacy](/privacy).

**Settings** › **About Voltip** shows the version and the license, and opens the source code, the releases and the [privacy policy](/privacy) in the browser.

When a paired computer is online, **Where to transcribe** at the top of **Talk** is set to **Send to** and the computer's name, and the recording goes to the computer as described below; choose **Transcribe on the phone** to have the phone transcribe it instead. A recording keeps its route until it ends, even if a computer comes online or goes offline in the meantime.

## Floating bubble

The floating bubble is a talk button over other apps: in a chat or a notes app, hold it and speak, and the result is copied to the phone's clipboard; touch and hold the text field to paste it. The phone recognises the recording itself, as **Transcribe on the phone** on the **Talk** page does, with the phone's own speech model, AI polish and dictionary.

To turn it on:

1. Open **Settings** › **Floating bubble**.
2. Allow the three permissions in turn: the microphone; **Display over other apps** (choose **Open settings** and turn it on for Voltip in the system settings); and notifications (while the bubble shows, a notification stays in the shade, from which you can close the bubble at any time).
3. Turn on **Show floating bubble**.

To use it:

- **Hold to talk** (the default): hold the bubble and speak, release to finish; to cancel, move the finger well away from where it pressed before releasing, and the bubble turns red. Drag the bubble right away to move it; when released, it moves to the nearer side of the screen and keeps that place.
- **Tap to start, tap to finish**: tap once to start recording and once more to finish; while recording, **Cancel** on the card discards it.
- When the text is ready, the bubble shows *Copied · touch and hold a text field to paste* and the text for 5 seconds: **Talk again** records again, **Don't copy** clears the clipboard. With **Show the result** off, the bubble only shows a check mark for a moment.
- **AI polish** is the same setting as on the **Talk** and AI models pages. With the built-in service, the bubble's recordings count toward this phone's [usage](/recognition/cloud#built-in-usage).

The bubble keeps working after you close Voltip's screens; choose **Close** in its notification, or turn it off under **Settings** › **Floating bubble**, to stop it. Some phones (such as Xiaomi, OPPO and vivo) need Voltip allowed to start automatically or run in the background in the system settings, or the system may close the bubble; where that is depends on the phone, usually under the app management or battery settings.

## Pairing

Pairing connects one phone and one computer, once:

1. On the computer, open the **Phone** page and start pairing. It shows a QR code and a 6-digit code, valid for 120 seconds.
2. On the phone, scan the QR code or enter the 6-digit code.
3. When you scan, the QR code has already checked both devices: press **Confirm and trust** on the phone, and there is nothing to do on the computer. When you enter the code, both screens show the same safety code. Check that they match, then confirm on both devices. If they do not match, reject the pairing.

A phone that scans the QR code pairs without a confirmation on the computer, so do not screenshot the pairing QR code or show it on a shared screen. A paired phone appears in the computer's device list, and you can forget a device you do not recognise at any time.

**Always-on pairing** on the computer keeps a pairing open for the next phone, renewing the QR code before it expires, until you turn it off. A phone that scans still needs only its own confirmation; one that enters the code needs the safety code confirmed on the computer as well.

The phone and the computer meet through a relay, the built-in one unless the computer is set to another, so pairing and the connection work on any network. Turning the relay off on the computer disconnects its phones. After a network drop, or a switch between Wi-Fi and mobile data, the two reconnect on their own, usually within seconds.

The pairing page on the phone shows the phone's connection to the relay, which pairing needs first. When it cannot connect, the page says why the last attempt failed (for example, it timed out, or the relay's address could not be looked up) and offers the **Connection check**, so you know whether to look at the network or the proxy settings.

## Talking on the phone

Hold **Hold to talk** on the phone, speak, and release. The audio streams to the computer while you speak. The computer recognises it with its own settings — its recognition service, dictionary, AI polish, rules and scenes — and inserts the text at its own cursor. While you hold it, the card shows how long you have been recording and the sound level, and says which computer the text goes to when you let go; then the phone shows each step and the result. To cancel, slide your finger away from the button and let go once it reads **Release to cancel**; a finger that moves a little while holding does not cancel.

If you would rather not hold the button, set **Talk by** under it to **Tap**: tap once to start recording and again to finish and send it, and tap **Cancel recording** under the button to discard it. The choice cannot change while a recording runs, and the phone remembers it.

When the computer recognises the text but cannot insert it at the cursor — the computer is locked, say, or the window in front runs as administrator — the text stays on the computer's clipboard and the phone says *Left on the computer's clipboard*. When something else keeps it from inserting, the phone says the text is in the computer's history. Either way the computer's history has the text.

Once the computer has inserted the text, the phone also keeps an entry in its own **History**, marked **Sent to** and the computer's name: the text the computer reported (up to 2,000 characters) and the length of the recording. The recognition and polish details stay in the computer's history. This entry is never uploaded to the computer. With saving turned off under **Settings** › **History** on the phone, the phone keeps none.

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

- Pairing creates an encrypted connection. When you scan, the QR code checks both devices: the phone knows it reached the computer on the screen, and the computer knows the phone read its QR code, so neither the relay nor someone who only knows the 6-digit code can pretend to be one of them. When you enter the code, you compare the safety code on both screens.
- Afterwards, the devices connect through the relay, which forwards their traffic. Everything is encrypted end to end, so the relay cannot read audio or text.
- Audio is compressed with Opus before it is sent.
- **Forget device** removes a pairing on both sides when the other device is online. The two can be paired again straight away.
- **Connection check** tests the connection to the relay and the encrypted round trip to each paired device.

The phone's own dictionary, rules and scenes apply only to what the phone recognises itself; the computer's settings synced to the phone are for reading and are never used to recognise on the phone.
