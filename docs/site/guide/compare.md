---
title: Open-source Wispr Flow alternative for Windows, macOS, Linux and Android
description: How Voltip compares with Wispr Flow, Superwhisper, Typeless, Handy, OpenWhispr and VoiceInk on platforms, speech recognition on the device, source code and price.
---

# Voltip and other voice typing apps

This page helps you compare Voltip with other voice typing and dictation apps on facts you can check: the platforms each app runs on, whether speech recognition can run on your own device, whether the source code is published, and what it costs.

## At a glance

| App | Platforms | Recognition on the device | Source code | Price |
| --- | --- | --- | --- | --- |
| **Voltip** | Windows, macOS, Linux, Android | Yes, on the computer with a local model; the Android app uses a cloud service | Published, AGPL-3.0 | Free |
| Wispr Flow | macOS, Windows, iOS, Android | No, transcription runs in the cloud | Not published | Free plan with a weekly word limit; paid Pro |
| Superwhisper | macOS, Windows, Linux, iOS, Android | Yes, with offline models | Not published | Free plan; paid Pro |
| Typeless | macOS, Windows, Linux, iOS, Android | No, audio is processed on Typeless's cloud servers | Not published | Free plan with a weekly word limit; paid Pro |
| Handy | Windows, macOS, Linux | Yes, it works offline | Published, MIT | Free |
| OpenWhispr | macOS, Windows, Linux; iPhone and iPad with Pro | Yes, with local models | Published, MIT | Free plan; paid Pro |
| VoiceInk | macOS (Apple silicon), iOS | Yes, transcription runs on the device | Published, GPL-3.0 | One-time purchase, free trial |

"Not published" means the app's website links to no source code for the app. Every row other than Voltip's comes from the pages under [Sources](#sources), read on 12 November 2026; features and prices change, and the vendors' own pages are the reference.

## Plans and prices

Prices are in US dollars as each vendor's website listed them; taxes and regional prices may differ.

- **Voltip**: free. The built-in service recognises and polishes for free, shared by all users; a cloud provider of your own charges its own prices, and local models cost nothing.
- **Wispr Flow**: the free plan has a weekly word limit, on Mac and Windows an allowance of 2,000 words and at most 5,000. Pro is US$15 a month, or US$144 a year.
- **Superwhisper**: free plan; Pro is US$8.49 a month, US$84.99 a year, or US$249.99 for a lifetime licence.
- **Typeless**: the free plan has a limited number of words a week. Pro is US$12 a month billed yearly, or US$30 billed monthly.
- **Handy**: free.
- **OpenWhispr**: local dictation is free and unlimited, and the free plan includes 2,000 cloud words a week. Pro is US$80 per user a year.
- **VoiceInk**: a one-time purchase, from US$25 for one Mac, with a free trial.

## What Voltip offers

- **One app on four systems**: Windows, macOS, Linux (X11 and Wayland) and Android, from the same [releases page](https://github.com/sunerpy/voltip/releases). An iOS app is planned.
- **Offline speech to text on your computer**: Qwen3-ASR, SenseVoice and Paraformer run on Windows, macOS and Linux, on the graphics card where there is one; the audio stays on your computer. See [Local recognition](/recognition/local).
- **No key needed to start**: the built-in service recognises and polishes for free, and you can switch to a provider of your own at any time. See [Cloud services](/recognition/cloud).
- **Your phone as a microphone and keyboard**: an Android phone sends what you say to the computer's cursor over an end-to-end encrypted connection, or transcribes on its own. See [Phone](/phone/).
- **Open source under AGPL-3.0**: the source code is on [GitHub](https://github.com/sunerpy/voltip).

[What is Voltip](/guide/what-is-voltip) describes how a dictation works, and [Install](/guide/install) has the packages for each system.

## Sources

Read on 12 November 2026. If a fact here is out of date, [report it](https://github.com/sunerpy/voltip/issues).

- Wispr Flow: [pricing](https://wisprflow.ai/pricing), [Flow plans and what's included](https://docs.wisprflow.ai/articles/9559327591-flow-plans-and-what-s-included), [data controls](https://wisprflow.ai/data-controls) ("Transcription always occurs on the cloud").
- Superwhisper: [home page and pricing](https://superwhisper.com/), [Superwhisper for Linux](https://superwhisper.com/linux).
- Typeless: [pricing](https://www.typeless.com/pricing), [data controls](https://www.typeless.com/data-controls) ("processed in real time on our cloud servers").
- Handy: [handy.computer](https://handy.computer/), [repository and licence](https://github.com/cjpais/Handy).
- OpenWhispr: [pricing](https://openwhispr.com/pricing), [repository and licence](https://github.com/OpenWhispr/openwhispr).
- VoiceInk: [pricing](https://tryvoiceink.com/pricing), [repository and licence](https://github.com/Beingpax/VoiceInk).
- Voltip: [repository](https://github.com/sunerpy/voltip), [licence](https://github.com/sunerpy/voltip/blob/main/LICENSE) and this website.
