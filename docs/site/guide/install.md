---
description: Install Voltip on Windows, macOS or Linux with one command or from a package, and check what you downloaded.
---

# Install

Release packages include a default recognition and AI polish service, so dictation works as soon as Voltip is installed. You can switch to another provider or to a local model at any time.

## One command

Windows 10 or 11, in PowerShell:

```powershell
irm https://raw.githubusercontent.com/sunerpy/voltip/main/scripts/install.ps1 | iex
```

Linux (x86_64) and macOS (Apple silicon or Intel), in a terminal:

```bash
curl -fsSL https://raw.githubusercontent.com/sunerpy/voltip/main/scripts/install.sh | sh
```

The script picks the package for the computer and downloads it together with the release's `SHA256SUMS`. Nothing is installed unless the checksum matches.

- **Windows**: the per-user installer runs silently, without an administrator prompt, and starts Voltip when it is done.
- **Linux**: the `.deb` is installed through apt where apt is available, which asks for your password; otherwise the AppImage goes to `~/.local/bin` with an entry in the applications menu.
- **macOS**: the dmg for the Mac's processor is chosen automatically, also from a Rosetta terminal. The app goes to `/Applications`, or to `~/Applications` when that folder is not writable. Installed this way, it opens without the Gatekeeper prompt.

### Options

Options are environment variables for `sh`, written after the pipe:

| Variable | Effect |
| --- | --- |
| `VOLTIP_VERSION=0.0.7` | Installs this release instead of the latest one |
| `VOLTIP_PACKAGE=appimage` | Installs the AppImage on a system that has apt |
| `VOLTIP_INSTALL_DIR=<folder>` | Puts the AppImage or the Mac app in another folder |

For example `curl -fsSL … | VOLTIP_VERSION=0.0.7 sh`. In PowerShell, run `$env:VOLTIP_VERSION = "0.0.7"` before the install command.

The commands read the install scripts from the `main` branch. To use the copy that shipped with a release, replace `main` with its tag, for example `v0.0.7`.

## Packages

Every [release](https://github.com/sunerpy/voltip/releases) carries the packages, `SHA256SUMS` and build attestations.

| Platform | Package | Notes |
| --- | --- | --- |
| Windows 10 and 11, x64 | `*_x64-setup.exe`, a per-user installer; `*_x64-portable.zip`, which you unpack and run as `voltip-desktop.exe` with its DLLs beside it | Not code-signed yet, so SmartScreen warns on the first start |
| Linux, x64 | `.deb`, `.AppImage` | Built on Ubuntu 22.04 and needs glibc 2.34 or newer. The `.deb` depends on `libwebkit2gtk-4.1-0`, `libvulkan1` and `libblas3`; the AppImage needs FUSE 2 (`libfuse2`). Works on X11 and Wayland |
| macOS 11 or later, Apple silicon | `*_aarch64.dmg` | M1 or newer |
| macOS 11 or later, Intel | `*_x64.dmg` | Intel Macs |
| Android | Not released yet | See [the phone page](/phone/) |

### Windows

The installer is not code-signed yet. On the first start, SmartScreen may show "Windows protected your PC": choose **More info**, then **Run anyway**.

### macOS

The dmg packages are signed with the project's own certificate but are not notarized by Apple. Open the dmg and drag Voltip onto Applications. Because the app is not notarized, macOS blocks the first start:

- On macOS 15 and later, open System Settings → Privacy & Security and choose **Open Anyway**.
- On macOS 11 to 14, Control-click Voltip in Applications and choose **Open**.
- In a terminal, `xattr -dr com.apple.quarantine /Applications/Voltip.app` has the same effect.

Updates install from inside the app. Updating from version 0.0.6 or earlier asks for the permissions once more: choose **Always Allow** in the keychain prompts, and if dictation does not work afterwards, turn Voltip off and on again under Accessibility and under Microphone in System Settings.

### Linux

- `.deb`: `sudo apt install ./Voltip_0.0.7_amd64.deb` installs it together with its dependencies.
- AppImage: make it executable with `chmod +x`, then run it. It needs `libfuse2`.
- On Wayland, pasting needs a helper tool, and the global shortcut is set up in the system settings. See [Platform notes](/reference/platforms#linux).

## Check a download

Compare the file with its line in `SHA256SUMS` (`sha256sum`, or `shasum -a 256` on a Mac), and confirm that it was built by this repository's release workflow:

```bash
gh attestation verify Voltip_0.0.7_amd64.deb --repo sunerpy/voltip \
  --signer-workflow sunerpy/voltip/.github/workflows/release-candidate.yml
```

## Build from source

See [Developers](/developers#build-from-source). A build from source has no default service; choose a provider or a local model in the app.
