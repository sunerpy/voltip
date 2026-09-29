---
description: Scenes adjust AI polish, the output mode, the language and extra instructions to the app you dictate into.
---

# Scenes

A scene adjusts one dictation to the app you are speaking into. When you start recording, Voltip checks which app has the focus and applies the first scene that matches it, for that dictation only. When no scene matches, the global settings apply.

Scenes are set up under **Settings → Scenes**.

## What a scene contains

- **Apps**: the apps it applies to, picked from the apps in your history or entered by name. Optionally, **window title keywords** narrow it to windows whose title contains one of them.
- **Overrides**, each of which can also follow the global setting:
  - AI polish on or off, and its [style](/recognition/polish#styles);
  - the [output mode](/dictation/output#output-modes);
  - the language;
  - the Chinese script.
- **Extra instructions** for the AI service, up to 500 characters, for example "This is a chat message: keep it casual and leave out the closing full stop."

Scenes are matched in the order of the list; move a more specific scene above a general one. For example, a scene for Chrome with the keyword "GitHub" placed above a scene for Chrome.

The overlay shows the name of the scene that applies, and each history entry records the app and the scene.

## How apps are recognised

| Platform | App name used |
| --- | --- |
| Windows | The program's file name without `.exe`, for example `slack` |
| macOS | The app's bundle identifier, for example `com.tinyspeck.slackmacgap` |
| Linux on X11 | The window class, for example `code` |

On a Mac, window titles are not read, so title keywords have no effect. A pure Wayland session does not tell apps which window has the focus, so scenes do not apply there.

## Limits

Up to 50 scenes, each with up to 20 apps and 10 title keywords.

## Built-in scenes

<StatusTag status="building" />

In development: ready-made scenes that you can switch on, adjust and reset, each with an [AI preset](/recognition/polish#presets), extra instructions, a list of specialist terms and a list of apps.

| Scene | For |
| --- | --- |
| Coding | Editors and terminals such as VS Code, Cursor and the JetBrains IDEs. Keeps code, commands, paths and identifiers as they are |
| Office writing | Mail and documents in apps such as Outlook, Word and WPS |
| Chat | Messaging apps such as WeChat, QQ, Slack, Teams, DingTalk, Feishu, Telegram and Discord |
| Legal, medical, finance, academic | Strict proofreading that keeps specialist terms, numbers, units and dates exact. They apply once you add the apps you use |

Built-in scenes are off until you turn them on, and your own scenes take precedence over them.
