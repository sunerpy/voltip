# Changelog

## [0.0.8](https://github.com/sunerpy/voltip/compare/v0.0.7...v0.0.8) (2026-09-29)


### Features

* AI presets and built-in scenes ([#17](https://github.com/sunerpy/voltip/issues/17)) ([1411760](https://github.com/sunerpy/voltip/commit/1411760a8b8b5650975659b9a7b3581dab402485))
* **presets:** built-in and custom AI presets, chosen from the home page, the title bar and the tray ([1411760](https://github.com/sunerpy/voltip/commit/1411760a8b8b5650975659b9a7b3581dab402485))
* **scenes:** seven built-in scenes with presets, instructions and term packs ([1411760](https://github.com/sunerpy/voltip/commit/1411760a8b8b5650975659b9a7b3581dab402485))


### Bug Fixes

* **desktop:** every settings group opens at its top ([1411760](https://github.com/sunerpy/voltip/commit/1411760a8b8b5650975659b9a7b3581dab402485))
* **desktop:** the history paste never goes to a window of the WebView2 runtime ([1411760](https://github.com/sunerpy/voltip/commit/1411760a8b8b5650975659b9a7b3581dab402485))
* **desktop:** the tray menu shows the newest state after a quick choice ([1411760](https://github.com/sunerpy/voltip/commit/1411760a8b8b5650975659b9a7b3581dab402485))
* **desktop:** the updater gives up on a silent host and says why a request failed ([1411760](https://github.com/sunerpy/voltip/commit/1411760a8b8b5650975659b9a7b3581dab402485))
* **ui:** a dialog over another one keeps its name, and Esc closes only the one on top ([1411760](https://github.com/sunerpy/voltip/commit/1411760a8b8b5650975659b9a7b3581dab402485))

## [0.0.7](https://github.com/sunerpy/voltip/compare/v0.0.6...v0.0.7) (2026-09-29)


### Features

* **desktop:** choose how text is inserted under Settings › Dictation ([7369468](https://github.com/sunerpy/voltip/commit/73694684b3b42c8ad45cd2b4a89d7e71c90da7ff))
* **desktop:** copy or paste a result from the home and history pages ([7369468](https://github.com/sunerpy/voltip/commit/73694684b3b42c8ad45cd2b4a89d7e71c90da7ff))
* **desktop:** say in words why a paste stayed on the clipboard ([7369468](https://github.com/sunerpy/voltip/commit/73694684b3b42c8ad45cd2b4a89d7e71c90da7ff))
* **overlay:** time each processing step and show that Esc cancels ([7369468](https://github.com/sunerpy/voltip/commit/73694684b3b42c8ad45cd2b4a89d7e71c90da7ff))
* **release:** sign the macOS release with one fixed certificate ([7369468](https://github.com/sunerpy/voltip/commit/73694684b3b42c8ad45cd2b4a89d7e71c90da7ff))
* standard interface copy, paste from the results, accent colours ([#14](https://github.com/sunerpy/voltip/issues/14)) ([7369468](https://github.com/sunerpy/voltip/commit/73694684b3b42c8ad45cd2b4a89d7e71c90da7ff))
* **ui:** Codex's blue, and an accent colour choice under Settings › Appearance ([7369468](https://github.com/sunerpy/voltip/commit/73694684b3b42c8ad45cd2b4a89d7e71c90da7ff))


### Bug Fixes

* **desktop:** free the updater before it reports the end of a run ([#16](https://github.com/sunerpy/voltip/issues/16)) ([c4a39b8](https://github.com/sunerpy/voltip/commit/c4a39b898eef9f59e46e046f5cae06ebf9d2b319))
* **release:** never wait on the macOS keychain clean-up ([c4a39b8](https://github.com/sunerpy/voltip/commit/c4a39b898eef9f59e46e046f5cae06ebf9d2b319))
* **ui:** keep text on one line when the window has room, in both languages ([7369468](https://github.com/sunerpy/voltip/commit/73694684b3b42c8ad45cd2b4a89d7e71c90da7ff))

## [0.0.6](https://github.com/sunerpy/voltip/compare/v0.0.5...v0.0.6) (2026-09-29)


### Bug Fixes

* **desktop:** give the macOS traffic lights a slot of their own ([609f59f](https://github.com/sunerpy/voltip/commit/609f59fb1443bf6b82068354f769a6143d72bf82))
* **desktop:** granting Accessibility needs no restart, and the notice says so ([7b74d44](https://github.com/sunerpy/voltip/commit/7b74d441a30c542ddcab8525175341672c85867d))
* **install:** register the Mac app with LaunchServices after copying it ([7196951](https://github.com/sunerpy/voltip/commit/71969519d085cae44190c0923c40e4b46ad82da2))

## [0.0.5](https://github.com/sunerpy/voltip/compare/v0.0.4...v0.0.5) (2026-09-28)


### Bug Fixes

* **desktop:** the Linux packages depend on the BLAS the binary links ([06334e6](https://github.com/sunerpy/voltip/commit/06334e6e3242cde81dc842dc8d7b3843390b1e2c))
* **install:** refresh apt's package lists before installing the deb ([158503c](https://github.com/sunerpy/voltip/commit/158503c01ee6c5991554c5b59b20f4eea936f8b3))

## [0.0.4](https://github.com/sunerpy/voltip/compare/v0.0.3...v0.0.4) (2026-09-28)


### Features

* **desktop:** open on the home page, with a notice for a missing permission ([f88d229](https://github.com/sunerpy/voltip/commit/f88d229868d306c560bfeedd022a6aff4dcd02de))
* **desktop:** 反馈 is a dialog, and its draft survives closing it ([ef25d34](https://github.com/sunerpy/voltip/commit/ef25d3456aebf06af38bb96c895478b53b205eb5))
* **desktop:** 语音模型, AI 模型 and 反馈 are pages of the main layout ([1d8df32](https://github.com/sunerpy/voltip/commit/1d8df32d4acc0e1a6152101166c6457069d1e3d3))
* **feedback:** screenshots and recordings, and 设置's 反馈 is the same page ([5d2ac68](https://github.com/sunerpy/voltip/commit/5d2ac681458b4d53ca8288f7f29918157a49f3f1))
* no idle metering, a microphone test, and a device choice ([ce3c437](https://github.com/sunerpy/voltip/commit/ce3c437f4273d7308f90cb059a6bfd593649f81f))
* one-line installers for Windows, Linux and both kinds of Mac ([35d2e87](https://github.com/sunerpy/voltip/commit/35d2e870828f05c28f2e0becbeaef778805ed68c))
* **release:** a macOS leg, with the dmg and the updater archive ([52d45fb](https://github.com/sunerpy/voltip/commit/52d45fb9190f3b7698d6d5ebc8306d9806eaafac))
* **release:** Intel Macs too, next to Apple silicon ([8c6ce4c](https://github.com/sunerpy/voltip/commit/8c6ce4c79e02b70ba5e65a014935217164f9c8a7))
* **tray:** the logo with a phase badge, a menu, and closing to the tray ([bbe498b](https://github.com/sunerpy/voltip/commit/bbe498b6d6fe6b012594b247beef16e38535b458))


### Bug Fixes

* **ui:** a select in a settings row is named once, and the device name fits ([c4a8c24](https://github.com/sunerpy/voltip/commit/c4a8c248a3bbf553e3c44e3868a122ab5984e9f6))
* **ui:** the hand cursor on every control, and the theme row lines up ([ebca843](https://github.com/sunerpy/voltip/commit/ebca8431d67ded23e28bfcedbc2e81ce93144f59))

## [0.0.3](https://github.com/sunerpy/voltip/compare/v0.0.2...v0.0.3) (2026-09-28)


### Features

* **hotkey:** push-to-talk on a lone modifier or a mouse side button ([81f6aa5](https://github.com/sunerpy/voltip/commit/81f6aa55245c444a48ed40bcb2bfad8c602b7f17))
* **pairing:** find devices on the LAN and pair a nearby computer with a tap ([8836dac](https://github.com/sunerpy/voltip/commit/8836dacd1019318c49a01a343340e9f66607d8b8))
* **pairing:** keep a pairing open on the desktop until it is turned off ([1901fab](https://github.com/sunerpy/voltip/commit/1901fab8f0ec2706ef78fcb951e9eefa62ed6383))
* **phone:** send typed text or the clipboard for the computer to insert ([e43b6db](https://github.com/sunerpy/voltip/commit/e43b6dbab5ba0860053e5b0ad22ca47dce7d826a))
* **phone:** stream a phone take as Opus once the desktop decodes it ([3f6a3e8](https://github.com/sunerpy/voltip/commit/3f6a3e8b5fae8bdb54c9f1cb6c4888f6edf4acec))


### Bug Fixes

* **pairing:** start a new pairing straight from a finished one ([6b526ef](https://github.com/sunerpy/voltip/commit/6b526eff16009932bf8ff38df922adbb39f0b30b))
* **phone:** a text sent after clearing the list is inserted again ([dde27b3](https://github.com/sunerpy/voltip/commit/dde27b3aaf8fdf593498c49bfeed29701ad918b8))
* **release:** the provider-key scan passed when a key sat early in a binary ([82c2b56](https://github.com/sunerpy/voltip/commit/82c2b56c15409c416ea7d995a6fd477935b74bdd))

## [0.0.2](https://github.com/sunerpy/voltip/compare/v0.0.1...v0.0.2) (2026-09-28)


### Features

* **feedback:** in-app feedback through a Cloudflare Worker ([e31ce28](https://github.com/sunerpy/voltip/commit/e31ce2891bb1d67dd405dbe0e7ca6e321e237ac9))
* **shell:** the sidebar collapses, hides and names 语音模型 and AI 模型 ([ee9081b](https://github.com/sunerpy/voltip/commit/ee9081bbf2d38ced2fa7db78aa59aac7abbfdeda))
* **update:** an update dialog in the manner of clash-verge ([b10fe85](https://github.com/sunerpy/voltip/commit/b10fe85463518d7c5efae928c18c7e517330d15d))


### Bug Fixes

* **core:** a device that shuts down stops answering on its LAN address first ([fdfff0d](https://github.com/sunerpy/voltip/commit/fdfff0d52aed62e664ad8155e3ad88c53686b277))
* **update:** measure the download speed from the update events ([c28838b](https://github.com/sunerpy/voltip/commit/c28838b031b1dc8df5f5175cd63ed9df7498adae))

## 0.0.1 (2026-09-27)


### Features

* Voltip, push-to-talk dictation with on-device models and a phone microphone ([a2037e4](https://github.com/sunerpy/voltip/commit/a2037e415a0de7e9754b67babc8809aae9c316e7))
