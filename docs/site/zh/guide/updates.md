---
description: Voltip 如何更新，各平台上的数据保存位置，以及如何卸载。
---

# 更新、卸载与数据位置

## 更新

Voltip 在应用内更新。在「设置 → 通用」中：

- 「检查更新」在你点击时检查一次是否有新版本。
- 「自动更新」默认关闭。开启后，Voltip 启动时检查更新，并在后台下载新版本；选择「重启并更新」时安装，或在下次启动 Voltip 时安装。

关闭自动更新时，Voltip 只在你点击「检查更新」时连接更新服务器。在 Windows 和 macOS 上，托盘菜单中也有同样的命令。

在 Windows 上，由安装器完成更新并重新启动应用；在 macOS 和 Linux 上，应用会原地替换后重新启动。

在 Mac 上，从 0.0.7 起，麦克风和辅助功能权限在更新后保持有效。从 0.0.6 及更早版本更新时多出的一次确认，见[安装](/zh/guide/install#macos)。

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
