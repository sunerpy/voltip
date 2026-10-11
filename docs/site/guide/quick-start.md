---
description: Your first dictation with Voltip, the permissions it needs, and how to switch to recognition on your own computer.
---

# Quick start

These steps take you from a fresh install to text at your cursor, and then to recognition that keeps the audio on your computer.

## Your first dictation

1. **Start Voltip.** It opens on the home page, ready to dictate with the default service.
2. **Grant the permissions it asks for.** A missing permission — the microphone, and on macOS also Accessibility, which Voltip needs to paste — appears on the home page with a button that opens the right setting.
3. **Put the cursor in any text field**, hold <kbd>Ctrl</kbd> <kbd>Alt</kbd> <kbd>Space</kbd>, speak, and release.

The text appears at the cursor, and the result is listed under the latest results on the home page.

If nothing arrives, press **Test microphone** in the **Recognition and polish** card on the home page and say a few words: the ring around **Start dictation** should grow and shrink with your voice. To record from another microphone, click its name in that card, or under the page title on the other pages, and choose the device. [Questions and troubleshooting](/reference/faq) covers the usual causes.

The setup guide under **Settings → General** walks through the shortcut, the recognition service and a test dictation.

## Recognise on your own computer

To keep the audio on your computer:

1. Open **Speech models** in the sidebar and choose **This computer**.
2. Download a model. **Qwen3-ASR 0.6B** (690 MB) is the recommended one; **SenseVoice Small** (240 MB) is the smallest.
3. Select the model when the download is finished.

From then on, no audio leaves the computer. To switch between the downloaded models, or back to the built-in service, click the speech model name on the home page, or under the page title on the other pages. [Local recognition](/recognition/local) compares the models and explains when the graphics card is used.

## On Linux with Wayland

A pure Wayland session does not let apps register global shortcuts. Bind a shortcut in your desktop's keyboard settings to the command `voltip-desktop --toggle`, and to `voltip-desktop --edit-toggle` for voice edit. Settings → Shortcuts shows the exact command for your installation, including the path of an AppImage.

## Next steps

- Choose how recording starts and stops: [Shortcuts and recording modes](/dictation/shortcuts)
- Turn AI polish on or off with the **AI Polish** switch at the top right of the window: [AI polish](/recognition/polish)
- Teach Voltip your names and terms: [Dictionary and rules](/recognition/dictionary)
