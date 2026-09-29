---
description: What differs between Windows, macOS and Linux, including permissions, pasting on Wayland and windows that run as administrator.
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
- **Tray**: left-click opens Voltip, right-click opens the menu. Closing the window keeps Voltip running in the tray.
- **GPU**: Vulkan, with the graphics driver's Vulkan support. The Vulkan loader ships with Voltip.

## macOS

- **Versions**: macOS 11 or later, as one package for Apple silicon and one for Intel.
- **First start**: the app is signed with the project's own certificate but is not notarized by Apple. See [Install](/guide/install#macos) for opening it the first time.
- **Permissions**: the microphone, and Accessibility, which Voltip needs to paste and to use a single key as the trigger. Voltip does not need Input Monitoring. From version 0.0.7 on, the permissions stay granted across updates.
- **Fn as the trigger**: set **When pressing the 🌐 key** to **Do Nothing** under System Settings → Keyboard.
- **Scenes** recognise the app but not the window title.
- **Tray**: clicking the menu bar icon opens the menu. Closing the window keeps Voltip running; it can be reopened from the Dock or the menu bar.
- **GPU**: Metal.

## Linux

- **Packages**: `.deb` and AppImage, for x64. They are built on Ubuntu 22.04 and need glibc 2.34 or newer.
- **Sessions**: X11 and Wayland. Closing the window quits Voltip, because there is no tray icon on Linux.
- **GPU**: Vulkan, with a driver that supports it (`libvulkan1`).

### Pasting

On X11, pasting works without further setup; the `.deb` recommends `xdotool` as a fallback. On Wayland, Voltip needs a helper tool that can press keys, and it tries them in this order:

| Desktop | Tools |
| --- | --- |
| KDE Plasma | `wtype`, `dotool`, `ydotool`, `kwtype` |
| GNOME | `dotool`, `ydotool` |
| Sway, Hyprland and other wlroots desktops | `wtype`, `dotool`, `ydotool` |

Install one of them from your distribution. `dotool` and `ydotool` need access to `/dev/uinput`; see their documentation. Without a tool, the text stays on the clipboard.

### Pure Wayland sessions

A Wayland session without X11 support places some limits on all apps:

- No global shortcut and no single-key trigger. Bind system shortcuts to `voltip-desktop --toggle`, `--edit-toggle` and `--cancel` instead; see [Shortcuts and recording modes](/dictation/shortcuts#linux-with-wayland).
- Esc does not cancel; use the `--cancel` shortcut.
- The app in front cannot be detected, so [scenes](/recognition/scenes) do not apply.

In a session that runs X11 apps through XWayland, the shortcut works only while an X11 window has the focus.

## Android

The Android app, which turns a phone into a microphone and keyboard for the computer, is in development and not released yet. See [the phone page](/phone/).

## Not available

- iOS has not been started.
- Packages for Windows on ARM and Linux on ARM are not planned for now.
