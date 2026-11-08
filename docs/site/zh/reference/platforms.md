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
- **托盘**：单击或双击图标打开 Voltip，右键打开菜单。关闭窗口后，Voltip 会继续在托盘中运行。
- **GPU**：Vulkan，需要显卡驱动支持 Vulkan。Vulkan 加载器随 Voltip 一起提供。
- **电脑声音**：录制所选输出设备的声音，无需额外设置。

## macOS

- **版本**：macOS 11 及以上，Apple 芯片和 Intel 芯片各有一个安装包。
- **首次启动**：应用使用项目自己的证书签名，但未经 Apple 公证。首次打开的方法见[安装](/zh/guide/install#macos)。
- **权限**：麦克风，以及辅助功能。Voltip 粘贴文字和使用单键触发都需要辅助功能权限，不需要「输入监控」权限。从 0.0.7 起，各版本使用同一张签名证书，以便更新后保留这些权限；第一次这样的更新尚未在真机上验证。
- **用 Fn 键触发**：在「系统设置 → 键盘」中把「按下 🌐 键时」设为「不执行任何操作」。
- **场景**可以识别应用，但不读取窗口标题。
- **托盘**：单击菜单栏图标打开菜单，双击打开 Voltip 窗口。为了与双击区分，菜单在「系统设置」中设定的双击间隔过后才出现。关闭窗口后 Voltip 继续运行，可以从程序坞或菜单栏重新打开。
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
| GNOME | `dotool`、`ydotool`；都不可用时，使用 GNOME 的[远程交互授权](#gnome-的远程交互授权) |
| Sway、Hyprland 等 wlroots 桌面 | `wtype`、`dotool`、`ydotool` |

从发行版的软件源安装其中一个即可；在 GNOME 上无需安装，见下一节。`dotool` 和 `ydotool` 需要访问 `/dev/uinput`，请参阅它们的文档。没有可用的工具时，文字会留在剪贴板中。

### GNOME 的远程交互授权

GNOME 不允许应用直接模拟按键。在 GNOME 上（包括通过 XWayland 运行 Voltip 时），如果 `dotool` 和 `ydotool` 都不可用，Voltip 会请求 GNOME 的「远程交互」授权来按下粘贴键。Voltip 只请求键盘，不请求屏幕和鼠标；只在第一次需要粘贴时请求，启动时不会请求。

第一次粘贴时，GNOME 显示「远程桌面」窗口：

1. 打开「允许远程交互」。
2. 保持勾选「记住该选择」。
3. 点按「分享」。

此后的粘贴，以及重新打开 Voltip 之后，都不会再询问。如果取消勾选「记住该选择」，下次粘贴时会再次询问。如果点按「取消」，这次的文字留在剪贴板中，悬浮窗显示「未获准远程交互，文字已复制」，下次粘贴时会再次询问。

如果 GNOME 没有响应或出错，文字同样留在剪贴板中，悬浮窗显示「远程交互不可用，文字已复制」，下次粘贴时会再试一次。如果一直如此，请注销后重新登录。

- **不使用这项授权**：把「设置 → 听写」中的「插入方式」改为「仅复制到剪贴板」。Voltip 不再按粘贴键，也不会请求授权，文字由你自己粘贴。
- **撤销授权**：GNOME 的设置中没有管理这项授权的页面。请打开「密码和密钥」，搜索 `voltip.paste.remote-desktop` 并删除这一项，下次粘贴时 GNOME 会再次询问。
- GNOME 按打开方式区分应用：例如一次从应用列表、一次从终端打开 Voltip 时，GNOME 会分别询问一次。

### 纯 Wayland 会话

不支持 X11 的 Wayland 会话对所有应用都有以下限制：

- 没有全局快捷键，也不能用单个按键触发。请改为给 `voltip-desktop --toggle`、`--edit-toggle` 和 `--cancel` 绑定系统快捷键，见[快捷键与录音方式](/zh/dictation/shortcuts#linux-与-wayland)。
- 按 Esc 不能取消，请使用绑定到 `--cancel` 的快捷键。
- 无法检测前台应用，因此[场景](/zh/recognition/scenes)不生效。

在通过 XWayland 运行 X11 应用的会话中，组合键只在 X11 窗口获得焦点时有效。

### Wayland 与 NVIDIA 显卡

在 Wayland 会话中，如果电脑装有 NVIDIA 显卡，无论使用 NVIDIA 驱动还是开源的 nouveau 驱动，负责绘制 Voltip 窗口的 WebKitGTK 都可能使 Voltip 在启动后随即退出，终端中显示 `Error 71 (Protocol error) dispatching to Wayland display`。因此，Voltip 在 Wayland 会话中发现 NVIDIA 显卡时，会在打开窗口前关闭 WebKitGTK 的 DMA-BUF 渲染。X11 会话和没有 NVIDIA 显卡的电脑不受影响。

- 如需保留 DMA-BUF 渲染，请用 `WEBKIT_DISABLE_DMABUF_RENDERER=0` 启动 Voltip，例如 `WEBKIT_DISABLE_DMABUF_RENDERER=0 voltip-desktop`。你自己设置的值，Voltip 不会更改。
- 如果 Voltip 仍在启动后随即退出，请改为通过 XWayland 运行：`GDK_BACKEND=x11 voltip-desktop`。此时 Voltip 作为 X11 应用运行，快捷键在 X11 窗口获得焦点时有效。

## Android

Android 应用把手机当作电脑的麦克风和键盘，见[手机端](/zh/phone/)。应用需要 Android 8.0 或更高版本，以及 64 位 Arm 处理器，通过发布页面的 APK 安装（见[安装](/zh/guide/install#android)）。识别、AI 润色和插入文字都在配对的电脑上完成。

## 暂不提供

- iOS 版计划在后续发布。
- 暂不提供 Windows ARM 和 Linux ARM 的安装包。
- 暂不提供适用于 32 位 Arm 或 x86 处理器手机的 Android 应用。
