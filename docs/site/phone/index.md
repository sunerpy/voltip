---
description: Use an Android phone as a microphone and keyboard for your computer, how pairing works, and how the connection is protected.
---

# Your phone as a microphone and keyboard

<StatusTag status="available" />

With the Voltip app for Android, a phone becomes a microphone and a keyboard for your computer. Hold to talk on the phone, and the text appears at the computer's cursor.

## Install the app

Every [release](https://github.com/sunerpy/voltip/releases) carries the app as `Voltip_<version>_android_arm64.apk`, for phones with Android 8.0 or later and a 64-bit Arm processor. Open the file on the phone to install it. [Install](/guide/install#android) describes the steps and how to check the download.

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

## How the connection is protected

- Pairing creates an encrypted connection and asks you to compare the safety code, so that nobody in between can pretend to be one of the devices.
- Afterwards, the devices connect directly on the same network. On different networks, an optional relay forwards their traffic. Everything is encrypted end to end, so the relay cannot read audio or text.
- Audio is compressed with Opus before it is sent.
- **Forget device** removes a pairing on both sides when the other device is online.
- **Connection check** tests the network path to each paired device and reports where it fails.

The history, dictionary and rules stay on the computer and are not synced to the phone; the phone only lists what it sent.
