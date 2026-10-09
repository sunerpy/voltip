---
description: 用一条命令或安装包在 Windows、macOS 和 Linux 上安装 Voltip，安装 Android 应用，并核对下载的文件。
---

# 安装

正式版安装包内置了默认的识别和 AI 润色服务，安装完成即可听写。之后可以随时换成其他服务商或本地模型。

## 一条命令安装

Windows 10 或 11，在 PowerShell 中执行：

```powershell
irm https://raw.githubusercontent.com/sunerpy/voltip/main/scripts/install.ps1 | iex
```

Linux（x86_64）和 macOS（Apple 芯片或 Intel 芯片），在终端中执行：

```bash
curl -fsSL https://raw.githubusercontent.com/sunerpy/voltip/main/scripts/install.sh | sh
```

脚本会为这台电脑选择合适的安装包，并连同发布附带的 `SHA256SUMS` 一起下载。校验不通过时不会安装任何内容。

- **Windows**：静默运行按用户安装的安装器，不需要管理员授权，安装完成后自动启动 Voltip。
- **Linux**：有 apt 的系统通过 apt 安装 `.deb`，会要求输入密码；否则把 AppImage 放到 `~/.local/bin`，并添加到应用菜单。
- **macOS**：自动识别处理器类型（在 Rosetta 终端中同样适用），下载对应的 dmg，把应用放进 `/Applications`；没有写入权限时放进 `~/Applications`。用这种方式安装的应用，打开时不会被 Gatekeeper 拦下。

### 选项

选项是传给 `sh` 的环境变量，写在管道之后：

| 变量 | 作用 |
| --- | --- |
| `VOLTIP_VERSION=0.0.7` | 安装指定版本，而不是最新版本 |
| `VOLTIP_PACKAGE=appimage` | 在有 apt 的系统上改为安装 AppImage |
| `VOLTIP_INSTALL_DIR=<目录>` | 把 AppImage、Mac 应用或 `voltip-server` 放到其他目录 |

例如 `curl -fsSL … | VOLTIP_VERSION=0.0.7 sh`。在 PowerShell 中，先执行 `$env:VOLTIP_VERSION = "0.0.7"`，再执行安装命令。

上面的命令从 `main` 分支读取安装脚本。如果要使用某个版本随附的脚本，把 `main` 换成它的标签，例如 `v0.0.7`。

### 在服务器上安装本机服务

如果要在没有桌面的 Linux x64 电脑上安装[本机服务](/zh/recognition/service) `voltip-server`，加上 `--server`：

```bash
curl -fsSL https://raw.githubusercontent.com/sunerpy/voltip/main/scripts/install.sh | sh -s -- --server
```

它安装到 `~/.local/share/voltip-server`，不需要管理员权限，命令为 `~/.local/bin/voltip-server`。加上 `--systemd-user` 时另外写入 systemd 用户服务，再加上 `--enable` 时启动它。

## 安装包

每个[发布版本](https://github.com/sunerpy/voltip/releases)都附带安装包、`SHA256SUMS` 和构建证明。

| 平台 | 安装包 | 说明 |
| --- | --- | --- |
| Windows 10 和 11，x64 | `*_x64-setup.exe`，按用户安装；`*_x64-portable.zip`，解压后运行 `voltip-desktop.exe`，旁边的 DLL 需要保留 | 尚未进行代码签名，首次启动时 SmartScreen 会提示 |
| Linux，x64 | `.deb`、`.AppImage` | 在 Ubuntu 22.04 上构建，需要 glibc 2.34 或更高版本。`.deb` 依赖 `libwebkit2gtk-4.1-0`、`libvulkan1` 和 `libblas3`；AppImage 需要 FUSE 2（`libfuse2`）。支持 X11 和 Wayland |
| Linux 服务器，x64 | `voltip-server-*-linux-x64.tar.gz` | 用于没有桌面的电脑的[本机服务](/zh/recognition/service)。与 Linux 安装包一样在 Ubuntu 22.04 上构建；自带运行所需的库，缺少系统库时 `install.sh --server` 会列出 |
| macOS 11 及以上，Apple 芯片 | `*_aarch64.dmg` | M1 及更新的芯片 |
| macOS 11 及以上，Intel 芯片 | `*_x64.dmg` | Intel 芯片的 Mac |
| Android 8.0 及以上，64 位 Arm | `Voltip_*_android_arm64.apk` | 见下面的 [Android](#android) 和[手机端](/zh/phone/) |

### Windows

安装包尚未进行代码签名。首次启动时，SmartScreen 可能显示「Windows 已保护你的电脑」：选择「更多信息」，再选择「仍要运行」。

### macOS

dmg 安装包使用项目自己的证书签名，但未经 Apple 公证。打开 dmg，把 Voltip 拖到「应用程序」。由于应用未经公证，macOS 会拦下首次启动：

- macOS 15 及更新版本：打开「系统设置 → 隐私与安全性」，选择「仍要打开」。
- macOS 11 至 14：在「应用程序」中按住 Control 点按 Voltip，选择「打开」。
- 也可以在终端中执行 `xattr -dr com.apple.quarantine /Applications/Voltip.app`，效果相同。

之后的更新在应用内完成。从 0.0.6 及更早版本更新时，各项权限还会再询问一次。0.0.15 和 0.0.16 曾尝试在安装后交接钥匙串，但 updater 当时已经删除旧应用 bundle，所以这条路径实际无效。0.0.16 安装首个包含修复的版本时，仍可能最后询问一次；再下一次应用内更新时，两边都已有修复，新版本会在安装前写入并校验自己的钥匙串条目，此后不再弹窗。使用 dmg 手动安装仍会为每个条目询问一次，选「允许」或「始终允许」都可以。如果之后听写无效，在系统设置的「辅助功能」和「麦克风」中把 Voltip 关闭后再打开。

### Linux

- `.deb`：执行 `sudo apt install ./Voltip_0.0.7_amd64.deb`，apt 会一并安装依赖。
- AppImage：用 `chmod +x` 设为可执行后运行，需要 `libfuse2`。
- 在 Wayland 下，粘贴需要辅助工具，全局快捷键需要在系统设置中配置。见[各平台说明](/zh/reference/platforms#linux)。

### Android

在手机上下载 `Voltip_<版本>_android_arm64.apk` 并打开。Android 会询问是否允许浏览器或文件管理器安装应用，为这次安装选择允许即可。应用需要 Android 8.0 或更高版本，以及 64 位 Arm 处理器。

新版本可以直接覆盖安装，配对关系会保留，因为每个版本都使用同一把密钥签名。发布中的 `.aab` 文件是提交给 Google Play 的安装包，手机上安装的是 `.apk`。

从 0.0.50 起，Android 应用换成了界面使用 Android 原生控件的新版本（0.0.46 至 0.0.49 以 Voltip RN 的名字单独发布）。在 0.0.49 及更早的 Voltip 上直接覆盖安装即可升级，配对、设置和记录都会保留。之前单独安装的 Voltip RN 是另一个应用，它的配对、设置和记录不会转到新版本，确认新版本可用后可以把它卸载。

## 核对下载的文件

把文件与 `SHA256SUMS` 中对应的一行比对（`sha256sum`，Mac 上用 `shasum -a 256`），再确认它出自这个仓库的发布流程：

```bash
gh attestation verify Voltip_0.0.7_amd64.deb --repo sunerpy/voltip \
  --signer-workflow sunerpy/voltip/.github/workflows/release-candidate.yml
```

## 从源码构建

见[开发者](/zh/developers#从源码构建)。从源码构建的版本没有默认服务，需要在应用中选择服务商或本地模型。
