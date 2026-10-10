---
description: How Voltip updates itself, where it keeps its data on each platform, and how to uninstall it.
---

# Updates, uninstall and your data

Use this page to keep Voltip up to date, to find the files it keeps on your computer, and to remove it completely.

## Updates

Voltip updates from inside the app. Under **Settings → General**:

- **Check for updates** looks for a new release once, when you press it.
- **Automatic updates**, off by default, checks when Voltip starts and downloads a new version in the background. It is installed when you choose **Restart and update**, or the next time you start Voltip.

With automatic updates off, Voltip contacts the update server only when you press **Check for updates**. On Windows and macOS the tray menu has the same command.

On Windows the installer finishes the update and restarts the app. On macOS and Linux the app is replaced in place and restarts.

On a Mac, updating from 0.0.6 or earlier asks for the permissions once more; see [Install](/guide/install#macos). From version 0.0.7 every release is signed with the same certificate so that later updates keep the microphone and Accessibility permissions. The first such update, from 0.0.7 to 0.0.8, has not been checked on a real Mac yet.

### On an Android phone

On the phone, updates are under **Settings → About**, and how they work depends on where the app came from:

- Installed from Google Play, it is updated by Google Play. **Open in Google Play**, under **Updated by Google Play**, opens its page there.
- Installed from a GitHub release, **Check for updates**, under **Updates**, looks for the latest release on this site and then on GitHub. When there is a newer one, **Download the new version** downloads its installer in the browser; open it when it is done to update. Your pairing, settings and history are kept. **Check automatically**, off by default, checks each time Voltip starts.

Android only installs an update signed like the version you have. When this site cannot be reached, Voltip asks GitHub, which answers a limited number of queries per hour from each network address; when the message says so, try again later.

Voltip 0.0.50 does not check for updates, and its **About** page has no **Updates** section. Google Play updates a copy it installed; for a copy from a GitHub release, download `Voltip_<version>_android_arm64.apk` from the new release and open it: it installs over the old version. Voltip 0.0.49 and earlier find the new version with **Check for updates** as above.

### If a check fails

The text after **Update failed** names the cause, for example that the connection timed out, was refused or could not be secured. Voltip gives up when a connection to the update server takes longer than 15 seconds to open, or when the server sends nothing for 30 seconds.

Voltip looks for updates on this site first and downloads them from here; this site serves the same files as the GitHub release, and Voltip checks every package against its signature before installing it. When this site cannot be reached, Voltip asks GitHub instead. Voltip does not use the proxy set in the Windows or macOS settings. If you reach the internet through a proxy tool, use its TUN mode, or set the `HTTPS_PROXY` environment variable and start Voltip again.

Voltip 0.0.52 and earlier ask GitHub only.

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
