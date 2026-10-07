---
description: Voltip 如何更新，各平台上的数据保存位置，以及如何卸载。
---

# 更新、卸载与数据位置

本页帮助你让 Voltip 保持最新、找到它保存在电脑上的文件，并在需要时将它完全移除。

## 更新

Voltip 在应用内更新。在「设置 → 通用」中：

- 「检查更新」在你点击时检查一次是否有新版本。
- 「自动更新」默认关闭。开启后，Voltip 启动时检查更新，并在后台下载新版本；选择「重启并更新」时安装，或在下次启动 Voltip 时安装。

关闭自动更新时，Voltip 只在你点击「检查更新」时连接更新服务器。在 Windows 和 macOS 上，托盘菜单中也有同样的命令。

在 Windows 上，由安装器完成更新并重新启动应用；在 macOS 和 Linux 上，应用会原地替换后重新启动。

在 Mac 上，从 0.0.6 及更早版本更新时，各项权限还会再询问一次，见[安装](/zh/guide/install#macos)。从 0.0.7 起，每个版本都使用同一张证书签名，以便之后的更新保留麦克风和辅助功能权限；第一次这样的更新（0.0.7 到 0.0.8）尚未在真机上验证。

### Android 手机

手机上的更新在「设置 → 关于」的「软件更新」中，方式取决于应用从哪里安装：

- 从 Google Play 安装的，由 Google Play 自动更新。「在 Google Play 中打开」打开它在 Google Play 上的页面。
- 从 GitHub 发布页安装的，「检查更新」会查询 GitHub 上的最新发布。有新版本时，「下载新版本」在浏览器中下载它的安装包，下载完成后打开即可更新，设置和记录都会保留。「自动检查更新」默认关闭；开启后，Voltip 每次启动时检查一次。

Android 只接受与已安装版本签名相同的安装包。GitHub 对每个网络地址每小时的查询次数有限，看到「每小时的查询次数有限」时，稍后再试即可。

Voltip RN 不检查更新，它的「关于」中没有「软件更新」。从新版本的发布中下载 `Voltip-RN_<版本>_android_arm64.apk` 并打开，即可覆盖安装，设置和记录都会保留。

### 检查更新失败时

「更新失败」后面的文字会说明原因，例如连接超时、连接被拒绝或无法建立安全连接。与更新服务器建立连接超过 15 秒，或服务器 30 秒内没有发送任何数据时，Voltip 停止这次检查。

更新从 GitHub 下载。Voltip 不使用 Windows 或 macOS 设置中的代理。如果你通过代理工具访问 GitHub，请使用它的 TUN 模式，或设置环境变量 `HTTPS_PROXY` 后重新启动 Voltip。

## 数据保存位置

设置、历史记录、词典、规则、场景和已下载的模型，都保存在每个用户各自的一个目录中：

| 平台 | 目录 |
| --- | --- |
| Windows | `%APPDATA%\voltip\Voltip\data` |
| macOS | `~/Library/Application Support/dev.voltip.Voltip` |
| Linux | `~/.local/share/voltip` |

模型位于其中的 `models` 目录，占用了几乎全部空间。在「语音模型」页删除模型会同时删除它的文件。

云端服务商的密钥和这台电脑的设备身份保存在系统钥匙串中（Windows 凭据管理器、macOS 钥匙串或 Linux 的 Secret Service），条目名称以 `dev.voltip.desktop` 开头。

## 卸载

1. 退出 Voltip。在 Windows 和 macOS 上，从托盘菜单选择「退出 Voltip」；关闭窗口只会把它隐藏到托盘。
2. 删除应用：
   - **Windows**：「设置 → 应用 → 已安装的应用 → Voltip → 卸载」。便携版直接删除其所在目录。
   - **macOS**：把「应用程序」中的 Voltip 移到废纸篓。
   - **Linux，.deb**：执行 `sudo apt remove voltip`。
   - **Linux，AppImage**：删除 `~/.local/bin/Voltip.AppImage` 和 `~/.local/share/applications/voltip.desktop`。
3. 如果也要删除数据，删除上面列出的目录，以及钥匙串中名称以 `dev.voltip.desktop` 开头的条目。
