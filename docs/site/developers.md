---
description: Building Voltip from source, contributing, and the design documents that describe how it works.
---

# Developers

Voltip is open source under the [GNU Affero General Public License v3.0 or later](https://github.com/sunerpy/voltip/blob/main/LICENSE); versions 0.0.20 and earlier were released under the Apache License 2.0. The code, issues and releases are on [GitHub](https://github.com/sunerpy/voltip).

## How it is built

- A Rust workspace does all the native work: global shortcuts, audio, recognition on the device and in the cloud, AI polish, text insertion, pairing and the encrypted phone link.
- Two Tauri 2 apps, one for the desktop and one for Android, share the React interface components. The interface only displays state and sends commands.
- Local recognition uses transcribe.cpp for Qwen3-ASR and sherpa-onnx for SenseVoice, Paraformer and live transcription.

## Build from source

You need Rust (the version in `rust-toolchain.toml`), Node 22 with pnpm, `cmake`, and on Linux the system packages Tauri needs; the full list is in the CI workflow.

```bash
pnpm install --frozen-lockfile
make desktop-dev      # run the desktop app with hot reload
make verify           # every check CI runs
make help             # everything else
```

Packages without a default service:

```bash
VOLTIP_ALLOW_NO_BUILTIN_ENGINES=1 make linux-x64     # .deb, .rpm and .AppImage in dist/linux-x64
VOLTIP_ALLOW_NO_BUILTIN_ENGINES=1 make windows-x64   # installer and portable zip, built on Linux
```

A build from source has no built-in service; choose a provider or a local model in the app. To build in your own defaults, copy `.env.build.example` to `.env.build` and fill it in; `.github/README-secrets.md` explains each value.

## Contributing

Issues and pull requests are welcome. [AGENTS.md](https://github.com/sunerpy/voltip/blob/main/AGENTS.md) describes the layout and the rules the code follows, [CONTRIBUTING.md](https://github.com/sunerpy/voltip/blob/main/CONTRIBUTING.md) the workflow, and [SECURITY.md](https://github.com/sunerpy/voltip/blob/main/SECURITY.md) how to report a security problem privately.

The pages of this site are written in the repository under `docs/site/`, in English and Chinese. A change to behaviour updates the matching pages in the same pull request.

## Design documents

The design documents are written in Chinese and are the contracts the code follows:

| Document | Topic |
| --- | --- |
| [Architecture](/zh/dev/architecture) | The Rust core, the two Tauri apps and how they communicate |
| [The dictation pipeline](/zh/dev/dictation) | Recording, recognition, models, shortcuts and insertion |
| [Interface and IPC contract](/zh/dev/frontend) | The front end and the messages it exchanges with Rust |
| [Pairing](/zh/dev/pairing) | Pairing, LAN discovery and the connection check |
| [Wire protocol](/zh/dev/protocol) | The messages between devices |
| [Threat model](/zh/dev/threat-model) | What the encrypted link protects against |
| [State machines](/zh/dev/state-machines) | The states of pairing and of a connection |
| [In-app feedback](/zh/dev/feedback) | What the feedback dialog sends, and the service that receives it |

Two more are on GitHub only: [acceptance.md](https://github.com/sunerpy/voltip/blob/main/docs/acceptance.md), which maps every feature to its code and tests, and [runbook.md](https://github.com/sunerpy/voltip/blob/main/docs/runbook.md), which covers building, packaging, the relay and releases.
