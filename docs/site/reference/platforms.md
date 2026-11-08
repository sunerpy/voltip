---
description: What differs between Windows, macOS and Linux, including permissions, recording computer audio, pasting on Wayland and windows that run as administrator.
---

# Platform notes

Voltip is the same app on every desktop system. This page lists what differs between them.

## Windows

- **Versions**: Windows 10 and 11, x64.
- **Packages**: a per-user installer that needs no administrator rights, or a portable zip.
- **SmartScreen**: the packages are not code-signed yet, so SmartScreen warns on the first start. Choose **More info**, then **Run anyway**.
- **Windows that run as administrator**: Windows does not let an ordinary app type into them. Voltip notices this before pasting and leaves the text on the clipboard instead.
- **Secure desktop**: the sign-in screen and elevation prompts refuse typed input from apps.
- **Voice edit** copies the selection with <kbd>Ctrl</kbd> <kbd>Insert</kbd>. On keyboard layouts where AltGr types characters, <kbd>Ctrl</kbd> <kbd>Alt</kbd> <kbd>E</kbd> can clash with AltGr+E; choose another shortcut there.
- **Tray**: a click or a double click opens Voltip, a right click opens the menu. Closing the window keeps Voltip running in the tray.
- **GPU**: Vulkan, with the graphics driver's Vulkan support. The Vulkan loader ships with Voltip.
- **Computer audio** is recorded from the output device you choose, with nothing to set up.

## macOS

- **Versions**: macOS 11 or later, as one package for Apple silicon and one for Intel.
- **First start**: the app is signed with the project's own certificate but is not notarized by Apple. See [Install](/guide/install#macos) for opening it the first time.
- **Permissions**: the microphone, and Accessibility, which Voltip needs to paste and to use a single key as the trigger. Voltip does not need Input Monitoring. From version 0.0.7 on, releases share one signing certificate so that updates keep the permissions; the first such update has not been checked on a real Mac yet.
- **Fn as the trigger**: set **When pressing the 🌐 key** to **Do Nothing** under System Settings → Keyboard.
- **Scenes** recognise the app but not the window title.
- **Tray**: a click on the menu bar icon opens the menu, and a double click opens the Voltip window. The menu appears once the double-click interval set in System Settings has passed, so that a double click does not open it. Closing the window keeps Voltip running; it can be reopened from the Dock or the menu bar.
- **GPU**: Metal.
- **Computer audio** needs macOS 14.6 or later; on earlier versions the option is dimmed. The first time you record it, macOS asks whether Voltip may record the audio of other apps; the choice can be changed later under System Settings → Privacy & Security.

## Linux

- **Packages**: `.deb` and AppImage, for x64. They are built on Ubuntu 22.04 and need glibc 2.34 or newer.
- **Sessions**: X11 and Wayland. Closing the window quits Voltip, because there is no tray icon on Linux.
- **GPU**: Vulkan, with a driver that supports it (`libvulkan1`).
- **Computer audio** is recorded from the default output's monitor through PulseAudio; PipeWire's PulseAudio service (`pipewire-pulse`) works too. Without a sound server the option is dimmed. The microphone is recorded as before, whatever the sound server.

### Pasting

On X11, pasting works without further setup; the `.deb` recommends `xdotool` as a fallback. On Wayland, Voltip needs a helper tool that can press keys, and it tries them in this order:

| Desktop | Tools |
| --- | --- |
| KDE Plasma | `wtype`, `dotool`, `ydotool`, `kwtype` |
| GNOME | `dotool`, `ydotool`; when neither works, GNOME's [remote interaction permission](#remote-interaction-on-gnome) |
| Sway, Hyprland and other wlroots desktops | `wtype`, `dotool`, `ydotool` |

Install one of them from your distribution; on GNOME nothing needs to be installed, see the next section. `dotool` and `ydotool` need access to `/dev/uinput`; see their documentation. Without a tool, the text stays on the clipboard.

### Remote interaction on GNOME

GNOME does not let apps press keys directly. On GNOME, including when Voltip runs through XWayland, Voltip asks for GNOME's remote interaction permission to press the paste keys when neither `dotool` nor `ydotool` works. Voltip asks for the keyboard only, not for the screen or the mouse, and only when it first needs to paste, never when it starts.

At the first paste, GNOME shows the **Remote Desktop** window:

1. Turn on **Allow Remote Interaction**.
2. Leave **Remember This Selection** ticked.
3. Click **Share**.

Later pastes, and pastes after you reopen Voltip, do not ask again. If you untick **Remember This Selection**, the next paste asks again. If you click **Cancel**, the text of this dictation stays on the clipboard, the overlay says **Remote interaction not allowed · text copied**, and the next paste asks again.

If GNOME does not answer or reports an error, the text stays on the clipboard too and the overlay says **Remote interaction unavailable · text copied**; the next paste tries again. If it keeps happening, log out and back in.

- **To do without the permission**, set **Insert method** under **Settings → Dictation** to **Copy to clipboard only**. Voltip then presses no paste keys and never asks for the permission; you paste the text yourself.
- **To revoke the permission**: GNOME's settings have no page for it. Open **Passwords and Keys**, search for `voltip.paste.remote-desktop` and delete the item; the next paste asks again.
- GNOME tells apps apart by how they were opened: when you open Voltip once from the app grid and once from a terminal, GNOME asks once for each.

### Pure Wayland sessions

A Wayland session without X11 support places some limits on all apps:

- No global shortcut and no single-key trigger. Bind system shortcuts to `voltip-desktop --toggle`, `--edit-toggle` and `--cancel` instead; see [Shortcuts and recording modes](/dictation/shortcuts#linux-with-wayland).
- Esc does not cancel; use the `--cancel` shortcut.
- The app in front cannot be detected, so [scenes](/recognition/scenes) do not apply.

In a session that runs X11 apps through XWayland, the shortcut works only while an X11 window has the focus.

### NVIDIA graphics cards on Wayland

On a Wayland session, WebKitGTK, which draws Voltip's window, can make Voltip quit right after it starts on computers with an NVIDIA graphics card, with NVIDIA's driver or the open-source nouveau driver alike. A terminal then shows `Error 71 (Protocol error) dispatching to Wayland display`. When Voltip finds an NVIDIA graphics card on a Wayland session, it therefore turns off WebKitGTK's DMA-BUF rendering before it opens its window. X11 sessions and computers without an NVIDIA graphics card are not affected.

- To keep DMA-BUF rendering, start Voltip with `WEBKIT_DISABLE_DMABUF_RENDERER=0`, for example `WEBKIT_DISABLE_DMABUF_RENDERER=0 voltip-desktop`. Voltip does not change a value you set yourself.
- If Voltip still quits right after it starts, run it through XWayland: `GDK_BACKEND=x11 voltip-desktop`. Voltip then runs as an X11 app, and the shortcut works while an X11 window has the focus.

## Android

The Android app turns a phone into a microphone and keyboard for the computer; see [the phone page](/phone/). It needs Android 8.0 or later and a 64-bit Arm processor, and installs from the APK on the releases page ([Install](/guide/install#android)). Recognition, AI polish and inserting the text happen on the paired computer.

## Not available

- An iOS app is planned for a later release.
- Packages for Windows on ARM and Linux on ARM are not planned for now.
- The Android app is not built for phones with a 32-bit Arm or an x86 processor.
