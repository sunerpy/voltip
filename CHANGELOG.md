# Changelog

## [0.1.0](https://github.com/sunerpy/voltip/compare/v0.0.52...v0.1.0) (2026-10-10)


### ⚠ BREAKING CHANGES

* 只走中继，断线后自动重连 ([#140](https://github.com/sunerpy/voltip/issues/140))
* 去掉局域网连接与「附近的电脑」，手机与电脑只通过中继连接；不带中继地址的旧二维码需要先更新电脑上的 Voltip。

### Features

* 今天、本周、本月、总计四张统计卡加高，突出节省的时间 ([463c1a0](https://github.com/sunerpy/voltip/commit/463c1a0e0ce2565a209d4f685dc5878d23672027))
* 只走中继，断线后自动重连 ([#140](https://github.com/sunerpy/voltip/issues/140)) ([463c1a0](https://github.com/sunerpy/voltip/commit/463c1a0e0ce2565a209d4f685dc5878d23672027))
* 检查更新先连接 voltip.firlab.app，连不上再连接 GitHub ([463c1a0](https://github.com/sunerpy/voltip/commit/463c1a0e0ce2565a209d4f685dc5878d23672027))

## [0.0.52](https://github.com/sunerpy/voltip/compare/v0.0.51...v0.0.52) (2026-10-09)


### Features

* 手机选择框改为底部面板，识别与润色在连接断开时重试 ([#137](https://github.com/sunerpy/voltip/issues/137)) ([34384d5](https://github.com/sunerpy/voltip/commit/34384d5d1a7ff16b81c1a8a71eb19a18d745bf2b))

## [0.0.51](https://github.com/sunerpy/voltip/compare/v0.0.50...v0.0.51) (2026-10-09)


### Features

* **mobile:** Android 应用可以检查更新 ([#135](https://github.com/sunerpy/voltip/issues/135)) ([bb80a53](https://github.com/sunerpy/voltip/commit/bb80a53a454a020462cfa78a934b0f099aa89350))

## [0.0.50](https://github.com/sunerpy/voltip/compare/v0.0.49...v0.0.50) (2026-10-09)


### Features

* **mobile:** Android 应用改为 React Native 版 ([#133](https://github.com/sunerpy/voltip/issues/133)) ([a567551](https://github.com/sunerpy/voltip/commit/a5675518809bcfbe10f941125501cbcf0ece1b2b))

## [0.0.49](https://github.com/sunerpy/voltip/compare/v0.0.48...v0.0.49) (2026-10-08)


### Features

* **mobile:** 关于页可以打开隐私政策 ([#131](https://github.com/sunerpy/voltip/issues/131)) ([72415fe](https://github.com/sunerpy/voltip/commit/72415fe44907388352e9cef223f47251a8b48212))

## [0.0.48](https://github.com/sunerpy/voltip/compare/v0.0.47...v0.0.48) (2026-10-08)


### Features

* **engines:** 内置服务可选多个润色模型，新增 Google AI Studio ([#130](https://github.com/sunerpy/voltip/issues/130)) ([cc04a9f](https://github.com/sunerpy/voltip/commit/cc04a9fef39f5c8d81f0f6983ad6b69e6bb085d2))
* 换用新标志「声波光标」 ([#128](https://github.com/sunerpy/voltip/issues/128)) ([a9b4687](https://github.com/sunerpy/voltip/commit/a9b468722f0a5207721e2cd4d70da29041a6c307))

## [0.0.47](https://github.com/sunerpy/voltip/compare/v0.0.46...v0.0.47) (2026-10-08)


### Bug Fixes

* **pairing:** 刚启动就开始的配对不再立即失败 ([#125](https://github.com/sunerpy/voltip/issues/125)) ([3371b04](https://github.com/sunerpy/voltip/commit/3371b0416763ca970bdb55152916637e7601454e))

## [0.0.46](https://github.com/sunerpy/voltip/compare/v0.0.45...v0.0.46) (2026-10-07)


### Features

* **release:** 发布包加入 React Native 版手机端 APK ([#123](https://github.com/sunerpy/voltip/issues/123)) ([3f9fe2e](https://github.com/sunerpy/voltip/commit/3f9fe2e3a4b01f48d6167d838cf68bc34e9ead28))

## [0.0.45](https://github.com/sunerpy/voltip/compare/v0.0.44...v0.0.45) (2026-10-07)


### Features

* **mobile-rn:** 新增 React Native 版手机端（验收版） ([#121](https://github.com/sunerpy/voltip/issues/121)) ([fa9b7d0](https://github.com/sunerpy/voltip/commit/fa9b7d096fd3dcc26c708d1a5c0047bd5230b993))

## [0.0.44](https://github.com/sunerpy/voltip/compare/v0.0.43...v0.0.44) (2026-10-05)


### Features

* **refine:** 自定义服务商的 AI 润色支持 Responses 接口 ([#112](https://github.com/sunerpy/voltip/issues/112)) ([6d542cf](https://github.com/sunerpy/voltip/commit/6d542cf270dea5c7c3f83f8e200800689e63551a))

## [0.0.43](https://github.com/sunerpy/voltip/compare/v0.0.42...v0.0.43) (2026-10-05)


### Features

* **overlay:** 插入后注明未润色的原因 ([#110](https://github.com/sunerpy/voltip/issues/110)) ([d25db09](https://github.com/sunerpy/voltip/commit/d25db090baf5bb70b2f9e6c406dfd01824b78398))

## [0.0.42](https://github.com/sunerpy/voltip/compare/v0.0.41...v0.0.42) (2026-10-05)


### Features

* **refine:** 提示词优化改用通用写法 ([#108](https://github.com/sunerpy/voltip/issues/108)) ([46e6227](https://github.com/sunerpy/voltip/commit/46e622732b13fca140f7abb60b927901e6c4107d))

## [0.0.41](https://github.com/sunerpy/voltip/compare/v0.0.40...v0.0.41) (2026-10-05)


### Features

* **refine:** 内置润色服务繁忙时提示改用自己的服务商 ([#106](https://github.com/sunerpy/voltip/issues/106)) ([2059356](https://github.com/sunerpy/voltip/commit/2059356e48125f2456f87afe398c65c2556c6414))

## [0.0.40](https://github.com/sunerpy/voltip/compare/v0.0.39...v0.0.40) (2026-10-04)


### Bug Fixes

* **mobile:** 手机不注册本机服务的三条命令 ([#103](https://github.com/sunerpy/voltip/issues/103)) ([32f266f](https://github.com/sunerpy/voltip/commit/32f266f99242d6ea881a6c0790a98170b2b7601e))

## [0.0.39](https://github.com/sunerpy/voltip/compare/v0.0.38...v0.0.39) (2026-10-04)


### Bug Fixes

* **install:** 升级 voltip-server 时重启正在运行的用户服务 ([#101](https://github.com/sunerpy/voltip/issues/101)) ([7df894d](https://github.com/sunerpy/voltip/commit/7df894d1d8be7731745a1b9bf3ef18f95e005a34))

## [0.0.38](https://github.com/sunerpy/voltip/compare/v0.0.37...v0.0.38) (2026-10-04)


### Bug Fixes

* **serve:** 关闭服务时中断上传，补齐 starting 状态、使用指南与英文预设名 ([#99](https://github.com/sunerpy/voltip/issues/99)) ([ea9afb5](https://github.com/sunerpy/voltip/commit/ea9afb5d80f5af8490af7b43a111ee3fc1bfeb99))

## [0.0.37](https://github.com/sunerpy/voltip/compare/v0.0.36...v0.0.37) (2026-10-04)


### Features

* **serve:** 本机语音服务：OpenAI 兼容接口、无头 voltip-server 与 App 开关 ([#96](https://github.com/sunerpy/voltip/issues/96)) ([31c1784](https://github.com/sunerpy/voltip/commit/31c17844de74cc0cec88b41de533314a488e254b))

## [0.0.36](https://github.com/sunerpy/voltip/compare/v0.0.35...v0.0.36) (2026-10-04)


### Bug Fixes

* **engines:** 候补模型顶替时的实时预览状态与隐私说明 ([#94](https://github.com/sunerpy/voltip/issues/94)) ([f09776d](https://github.com/sunerpy/voltip/commit/f09776dd0807e24d7c170a702f8bb40aa701c420))

## [0.0.35](https://github.com/sunerpy/voltip/compare/v0.0.34...v0.0.35) (2026-10-04)


### Features

* **engines:** 额度用完后自动改用候补模型 ([#89](https://github.com/sunerpy/voltip/issues/89)) ([c76599f](https://github.com/sunerpy/voltip/commit/c76599fa94ad562781d013754f11e1e9d048c4f6))

## [0.0.34](https://github.com/sunerpy/voltip/compare/v0.0.33...v0.0.34) (2026-10-03)


### Features

* **asr:** 支持阿里云百炼的语音识别，实时模型边说边识别 ([#86](https://github.com/sunerpy/voltip/issues/86)) ([52dea8e](https://github.com/sunerpy/voltip/commit/52dea8eab53b2a5e898344785ad42a9752145bb5))

## [0.0.33](https://github.com/sunerpy/voltip/compare/v0.0.32...v0.0.33) (2026-10-03)


### Bug Fixes

* **mobile:** the update check asks the phone's own repository ([#83](https://github.com/sunerpy/voltip/issues/83)) ([e7acaa8](https://github.com/sunerpy/voltip/commit/e7acaa82e336dd5e4c01d8313516f9cc0e2c29f4))

## [0.0.32](https://github.com/sunerpy/voltip/compare/v0.0.31...v0.0.32) (2026-10-03)


### Features

* **mobile:** the phone in the desktop's look, touch dropdowns, entries from 说话, and counts that move ([#80](https://github.com/sunerpy/voltip/issues/80)) ([8c2080e](https://github.com/sunerpy/voltip/commit/8c2080e5c284d369bdd75179553391b6aedf3bb4))

## [0.0.31](https://github.com/sunerpy/voltip/compare/v0.0.30...v0.0.31) (2026-10-03)


### Features

* **dictation:** the built-in service previews while you speak ([#77](https://github.com/sunerpy/voltip/issues/77)) ([8b26a1a](https://github.com/sunerpy/voltip/commit/8b26a1a54c7f7d4f2eb76c6de3ac06ddcd4e45a8))


### Bug Fixes

* the phone's update check and back on Android, and the live preview after a cancel or the last audio ([#79](https://github.com/sunerpy/voltip/issues/79)) ([2cebe5b](https://github.com/sunerpy/voltip/commit/2cebe5b26f5a7ef3aed91f74a360aa73f5f2d709))

## [0.0.30](https://github.com/sunerpy/voltip/compare/v0.0.29...v0.0.30) (2026-10-02)


### Features

* **mobile:** updates on Android, from Google Play or from GitHub ([#75](https://github.com/sunerpy/voltip/issues/75)) ([e540c39](https://github.com/sunerpy/voltip/commit/e540c39690fa2bf4a5496f3c873b4bd49739e1b6))

## [0.0.29](https://github.com/sunerpy/voltip/compare/v0.0.28...v0.0.29) (2026-10-02)


### Bug Fixes

* **pairing:** a device forgotten and paired again at once comes online ([#73](https://github.com/sunerpy/voltip/issues/73)) ([aa69121](https://github.com/sunerpy/voltip/commit/aa6912154cfd56dc5ec27dcc374734b38e4608ee))

## [0.0.28](https://github.com/sunerpy/voltip/compare/v0.0.27...v0.0.28) (2026-10-02)


### Features

* **models:** download a model by hand and import it ([#71](https://github.com/sunerpy/voltip/issues/71)) ([5d48f8f](https://github.com/sunerpy/voltip/commit/5d48f8f7dbbd1b61374f0dd8f028f289941afde1))

## [0.0.27](https://github.com/sunerpy/voltip/compare/v0.0.26...v0.0.27) (2026-10-02)


### Features

* **mobile:** Android's back goes up a level, and the launcher icon is the Voltip mark ([#68](https://github.com/sunerpy/voltip/issues/68)) ([1d8c756](https://github.com/sunerpy/voltip/commit/1d8c756757575fd83b2b2386bbd59959e7e46721))

## [0.0.26](https://github.com/sunerpy/voltip/compare/v0.0.25...v0.0.26) (2026-10-02)


### Features

* **sync:** the computer's history and settings on the phone, the phone's records on the computer ([#65](https://github.com/sunerpy/voltip/issues/65)) ([c1b5382](https://github.com/sunerpy/voltip/commit/c1b5382b95fa215fb1af812843fcd4ea6f6f05c6))

## [0.0.25](https://github.com/sunerpy/voltip/compare/v0.0.24...v0.0.25) (2026-10-02)


### Features

* **mobile:** the phone sends feedback of its own ([#63](https://github.com/sunerpy/voltip/issues/63)) ([9f85157](https://github.com/sunerpy/voltip/commit/9f85157ebfa6e07f823433f67082c989973ea7d9))

## [0.0.24](https://github.com/sunerpy/voltip/compare/v0.0.23...v0.0.24) (2026-10-01)


### Features

* **mobile:** the phone has the desktop's history ([#60](https://github.com/sunerpy/voltip/issues/60)) ([6385f85](https://github.com/sunerpy/voltip/commit/6385f851e9dbb5f6bf8d167896e4ccba80dd37c0))

## [0.0.23](https://github.com/sunerpy/voltip/compare/v0.0.22...v0.0.23) (2026-10-01)


### Features

* **mobile:** the phone has the desktop's dictionary, rules and scenes ([#58](https://github.com/sunerpy/voltip/issues/58)) ([833f24e](https://github.com/sunerpy/voltip/commit/833f24eac250d95732c08086bb40b9d4390106ef))

## [0.0.22](https://github.com/sunerpy/voltip/compare/v0.0.21...v0.0.22) (2026-10-01)


### Bug Fixes

* **desktop:** a history paste on Windows no longer lands in Voltip's own web view ([#55](https://github.com/sunerpy/voltip/issues/55)) ([03a9f4f](https://github.com/sunerpy/voltip/commit/03a9f4fa98bab1ac8a3932ef6aff6e6a8a4fa32c))

## [0.0.21](https://github.com/sunerpy/voltip/compare/v0.0.20...v0.0.21) (2026-10-01)


### Features

* **mobile:** the phone has its own speech models, AI models and presets ([#53](https://github.com/sunerpy/voltip/issues/53)) ([98b4930](https://github.com/sunerpy/voltip/commit/98b49307b9d43374c1812c831431bc1fc309f100))

## [0.0.20](https://github.com/sunerpy/voltip/compare/v0.0.19...v0.0.20) (2026-10-01)


### Features

* **mobile:** the phone transcribes on its own when no paired computer is online ([#50](https://github.com/sunerpy/voltip/issues/50)) ([633f1d6](https://github.com/sunerpy/voltip/commit/633f1d68f5aeb43ff0c8269a2385792d13c62768))


### Bug Fixes

* **mobile:** Android no longer closes at once on start: the app hands the Android Keystore its context before Rust starts (0.0.18 and 0.0.19 closed on a Xiaomi HyperOS 3) ([633f1d6](https://github.com/sunerpy/voltip/commit/633f1d68f5aeb43ff0c8269a2385792d13c62768))

## [0.0.19](https://github.com/sunerpy/voltip/compare/v0.0.18...v0.0.19) (2026-10-01)


### Bug Fixes

* **identity:** 在安装前交接 macOS 钥匙串条目 ([#48](https://github.com/sunerpy/voltip/issues/48)) ([a34e436](https://github.com/sunerpy/voltip/commit/a34e436ecc862d1a937054792588f0b0feb154be))

## [0.0.18](https://github.com/sunerpy/voltip/compare/v0.0.17...v0.0.18) (2026-10-01)


### Features

* **android:** ship the phone app's APK and AAB with every release ([#45](https://github.com/sunerpy/voltip/issues/45)) ([e5f4c06](https://github.com/sunerpy/voltip/commit/e5f4c06654a5e940fd8130329d455eaaedf14ac1))

## [0.0.17](https://github.com/sunerpy/voltip/compare/v0.0.16...v0.0.17) (2026-10-01)


### Features

* **desktop:** cancel the speakers' echo in a mixed recording ([#43](https://github.com/sunerpy/voltip/issues/43)) ([97e3550](https://github.com/sunerpy/voltip/commit/97e3550e95d0ff7da56016615cd9d0c473caff6f))

## [0.0.16](https://github.com/sunerpy/voltip/compare/v0.0.15...v0.0.16) (2026-10-01)


### Features

* **desktop:** switch the speech model, AI model and microphone in place; a double click on the tray icon opens the window ([#41](https://github.com/sunerpy/voltip/issues/41)) ([0d60085](https://github.com/sunerpy/voltip/commit/0d60085446e918ea8d0e398802675c93cbf6c0b5))

## [0.0.15](https://github.com/sunerpy/voltip/compare/v0.0.14...v0.0.15) (2026-09-30)


### Bug Fixes

* **identity:** an in-app update on macOS hands the keychain over instead of asking again ([#38](https://github.com/sunerpy/voltip/issues/38)) ([21e6ff7](https://github.com/sunerpy/voltip/commit/21e6ff7e939035cc65bb618ca3495063ea4eec56))

## [0.0.14](https://github.com/sunerpy/voltip/compare/v0.0.13...v0.0.14) (2026-09-30)


### Bug Fixes

* **audio:** a mixed take mixes nothing pushed before a drop ([#36](https://github.com/sunerpy/voltip/issues/36)) ([1979f13](https://github.com/sunerpy/voltip/commit/1979f138dd87498943482f04229424c009c0e948))

## [0.0.13](https://github.com/sunerpy/voltip/compare/v0.0.12...v0.0.13) (2026-09-30)


### Bug Fixes

* **audio:** a mixed take empties a full queue before its overflow is reported ([#34](https://github.com/sunerpy/voltip/issues/34)) ([f1c2553](https://github.com/sunerpy/voltip/commit/f1c25539c510b74022604d56d8a539da7dc96c95))

## [0.0.12](https://github.com/sunerpy/voltip/compare/v0.0.11...v0.0.12) (2026-09-30)


### Bug Fixes

* **audio:** a mixed take drops the computer's sound left from before a queue overflow ([4abaae1](https://github.com/sunerpy/voltip/commit/4abaae1463a4d0fbfba157abc2b73e9f8783c874))
* **audio:** a mixed take drops the computer's sound left from before a queue overflow ([#31](https://github.com/sunerpy/voltip/issues/31)) ([4abaae1](https://github.com/sunerpy/voltip/commit/4abaae1463a4d0fbfba157abc2b73e9f8783c874))
* **identity:** Mac releases stop asking for keychain items from ad-hoc Voltip on every update ([#33](https://github.com/sunerpy/voltip/issues/33)) ([c6bdc04](https://github.com/sunerpy/voltip/commit/c6bdc04df1da72d0599f4f4137aac19191c4a391))

## [0.0.11](https://github.com/sunerpy/voltip/compare/v0.0.10...v0.0.11) (2026-09-30)


### Bug Fixes

* **audio:** M4 corrections to mixing and long-take gaps, with tests that hold on slow runners ([#27](https://github.com/sunerpy/voltip/issues/27)) ([9ab69fd](https://github.com/sunerpy/voltip/commit/9ab69fd03d5a72e2e1e547d28d562f33456dcd1f))
* **audio:** mix the first microphone chunk with recent computer sound ([9ab69fd](https://github.com/sunerpy/voltip/commit/9ab69fd03d5a72e2e1e547d28d562f33456dcd1f))
* **audio:** report samples dropped right before a long take stops ([9ab69fd](https://github.com/sunerpy/voltip/commit/9ab69fd03d5a72e2e1e547d28d562f33456dcd1f))

## [0.0.10](https://github.com/sunerpy/voltip/compare/v0.0.9...v0.0.10) (2026-09-30)


### Features

* **history:** process long entries with an AI preset in parts, export SRT subtitles or text ([5e3397a](https://github.com/sunerpy/voltip/commit/5e3397ad04468cede23437281b7f3dc35715b4bb))
* long recordings, computer audio and mixing, AI processing and exports ([#23](https://github.com/sunerpy/voltip/issues/23)) ([5e3397a](https://github.com/sunerpy/voltip/commit/5e3397ad04468cede23437281b7f3dc35715b4bb))
* **recording:** recognise long recordings in segments while they record, cut where the speech pauses ([5e3397a](https://github.com/sunerpy/voltip/commit/5e3397ad04468cede23437281b7f3dc35715b4bb))
* **recording:** record the microphone, the computer's sound or both, for up to two hours ([5e3397a](https://github.com/sunerpy/voltip/commit/5e3397ad04468cede23437281b7f3dc35715b4bb))

## [0.0.9](https://github.com/sunerpy/voltip/compare/v0.0.8...v0.0.9) (2026-09-29)


### Features

* history of 20,000 entries and statistics ([#21](https://github.com/sunerpy/voltip/issues/21)) ([0a6762c](https://github.com/sunerpy/voltip/commit/0a6762c34f20e1d3e2b272da582e045d2ff1074f))
* **history:** keep up to 20,000 entries in a database, imported once from the old file ([0a6762c](https://github.com/sunerpy/voltip/commit/0a6762c34f20e1d3e2b272da582e045d2ff1074f))
* **history:** the history page loads 100 entries at a time and searches the whole history ([0a6762c](https://github.com/sunerpy/voltip/commit/0a6762c34f20e1d3e2b272da582e045d2ff1074f))
* **home:** characters transcribed and corrected, speaking time and time saved, with the basis of the estimate ([0a6762c](https://github.com/sunerpy/voltip/commit/0a6762c34f20e1d3e2b272da582e045d2ff1074f))

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
