---
description: How Voltip updates itself, where it keeps its data on each platform, and how to uninstall it.
---

# Updates, uninstall and your data

## Updates

Voltip updates from inside the app. Under **Settings → General**:

- **Check for updates** looks for a new release once, when you press it.
- **Automatic updates**, off by default, checks when Voltip starts and downloads a new version in the background. It is installed when you choose **Restart and update**, or the next time you start Voltip.

With automatic updates off, Voltip contacts the update server only when you press **Check for updates**. On Windows and macOS the tray menu has the same command.

On Windows the installer finishes the update and restarts the app. On macOS and Linux the app is replaced in place and restarts.

On a Mac, the microphone and Accessibility permissions stay granted across updates from version 0.0.7 on. See [Install](/guide/install#macos) for the one extra prompt when updating from 0.0.6 or earlier.

## Where your data is kept

Settings, the history, the dictionary, the rules, the scenes and the downloaded models are stored in one folder per user:

| Platform | Folder |
| --- | --- |
| Windows | `%APPDATA%\voltip\Voltip\data` |
| macOS | `~/Library/Application Support/dev.voltip.Voltip` |
| Linux | `~/.local/share/voltip` |

The models are in the `models` folder inside it and account for almost all of its size. Deleting a model from the Speech models page removes its files.

Keys for cloud providers and this computer's device identity are kept in the system keychain — Windows Credential Manager, the macOS keychain, or the Secret Service on Linux — under entries whose names begin with `dev.voltip.desktop`.

## Uninstall

1. Quit Voltip. On Windows and macOS, choose **Quit Voltip** from the tray menu; closing the window only hides it there.
2. Remove the app:
   - **Windows**: Settings → Apps → Installed apps → Voltip → Uninstall. For the portable version, delete its folder.
   - **macOS**: move Voltip from Applications to the Trash.
   - **Linux, .deb**: `sudo apt remove voltip`.
   - **Linux, AppImage**: delete `~/.local/bin/Voltip.AppImage` and `~/.local/share/applications/voltip.desktop`.
3. To remove your data as well, delete the folder listed above, and the keychain entries whose names begin with `dev.voltip.desktop`.
