---
description: Windows、macOS 和 Linux 之间的差异，包括权限、录制电脑声音、Wayland 下的粘贴，以及以管理员身份运行的窗口。
---

# 各平台说明

Voltip 在各个桌面系统上是同一个应用。本页列出各平台之间的差异。

## Windows

- **版本**：Windows 10 和 11，x64。
- **安装包**：按用户安装的安装包，不需要管理员权限；或便携版 zip。
- **SmartScreen**：安装包尚未进行代码签名，首次启动时 SmartScreen 会提示。选择「更多信息」，再选择「仍要运行」。
- **以管理员身份运行的窗口**：Windows 不允许普通应用向这类窗口输入。Voltip 会在粘贴前发现这种情况，改为把文字留在剪贴板中。
- **安全桌面**：登录界面和权限提升提示会拒绝其他应用的输入。
- **语音编辑**用 <kbd>Ctrl</kbd> <kbd>Insert</kbd> 复制选中的文字。在用 AltGr 输入字符的键盘布局中，<kbd>Ctrl</kbd> <kbd>Alt</kbd> <kbd>E</kbd> 可能与 AltGr+E 冲突，请换一个快捷键。
- **托盘**：左键打开 Voltip，右键打开菜单。关闭窗口后，Voltip 会继续在托盘中运行。
- **GPU**：Vulkan，需要显卡驱动支持 Vulkan。Vulkan 加载器随 Voltip 一起提供。
- **电脑声音**：录制所选输出设备的声音，无需额外设置。

## macOS

- **版本**：macOS 11 及以上，Apple 芯片和 Intel 芯片各有一个安装包。
- **首次启动**：应用使用项目自己的证书签名，但未经 Apple 公证。首次打开的方法见[安装](/zh/guide/install#macos)。
- **权限**：麦克风，以及辅助功能。Voltip 粘贴文字和使用单键触发都需要辅助功能权限，不需要「输入监控」权限。从 0.0.7 起，各版本使用同一张签名证书，以便更新后保留这些权限；第一次这样的更新尚未在真机上验证。
- **用 Fn 键触发**：在「系统设置 → 键盘」中把「按下 🌐 键时」设为「不执行任何操作」。
- **场景**可以识别应用，但不读取窗口标题。
- **托盘**：点击菜单栏图标打开菜单。关闭窗口后 Voltip 继续运行，可以从程序坞或菜单栏重新打开。
- **GPU**：Metal。
- **电脑声音**：需要 macOS 14.6 或更高版本，更早的版本上这个选项置灰。第一次录制时，macOS 会询问是否允许 Voltip 录制其他应用的声音，之后可以在「系统设置 → 隐私与安全性」中更改。

## Linux

- **安装包**：`.deb` 和 AppImage，x64。在 Ubuntu 22.04 上构建，需要 glibc 2.34 或更高版本。
- **会话**：支持 X11 和 Wayland。Linux 上没有托盘图标，因此关闭窗口即退出 Voltip。
- **GPU**：Vulkan，需要支持 Vulkan 的驱动（`libvulkan1`）。
- **电脑声音**：通过 PulseAudio 录制默认输出设备的监听源；PipeWire 的 PulseAudio 服务（`pipewire-pulse`）同样可用。没有音频服务时这个选项置灰。麦克风的录制方式不受音频服务影响。

### 粘贴

在 X11 上，粘贴无需额外设置；`.deb` 推荐安装 `xdotool` 作为备用。在 Wayland 上，Voltip 需要一个能模拟按键的辅助工具，并按以下顺序尝试：

| 桌面环境 | 工具 |
| --- | --- |
| KDE Plasma | `wtype`、`dotool`、`ydotool`、`kwtype` |
| GNOME | `dotool`、`ydotool` |
| Sway、Hyprland 等 wlroots 桌面 | `wtype`、`dotool`、`ydotool` |

从发行版的软件源安装其中一个即可。`dotool` 和 `ydotool` 需要访问 `/dev/uinput`，请参阅它们的文档。没有可用的工具时，文字会留在剪贴板中。

### 纯 Wayland 会话

不支持 X11 的 Wayland 会话对所有应用都有以下限制：

- 没有全局快捷键，也不能用单个按键触发。请改为给 `voltip-desktop --toggle`、`--edit-toggle` 和 `--cancel` 绑定系统快捷键，见[快捷键与录音方式](/zh/dictation/shortcuts#linux-与-wayland)。
- 按 Esc 不能取消，请使用绑定到 `--cancel` 的快捷键。
- 无法检测前台应用，因此[场景](/zh/recognition/scenes)不生效。

在通过 XWayland 运行 X11 应用的会话中，组合键只在 X11 窗口获得焦点时有效。

## Android

把手机当作电脑麦克风和键盘的 Android 应用正在开发，尚未发布。见[手机端](/zh/phone/)。

## 暂不提供

- iOS 尚未开始开发。
- 暂不提供 Windows ARM 和 Linux ARM 的安装包。
