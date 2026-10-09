---
description: Scenes adjust AI polish, the output mode, the language and extra instructions to the app you dictate into.
---

# Scenes

A scene adjusts one dictation to the app you are speaking into. When you start recording, Voltip checks which app has the focus and applies the first scene that matches it, for that dictation only. When no scene matches, the global settings apply.

Scenes are set up under **Settings → Scenes**.

## What a scene contains

- **Apps**: the apps it applies to, picked from the apps in your history or entered by name. Optionally, **window title keywords** narrow it to windows whose title contains one of them.
- **Overrides**, each of which can also follow the global setting:
  - AI polish on or off, and its [preset](/recognition/polish#presets);
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

Up to 50 scenes of your own, each with up to 20 apps and 10 title keywords. The built-in scenes do not count towards the 50.

## Built-in scenes

Voltip comes with ready-made scenes for common kinds of writing. Each has an [AI preset](/recognition/polish#presets), extra instructions, a list of specialist terms and a list of apps for Windows, macOS and Linux.

| Scene | For | Preset |
| --- | --- | --- |
| Coding | Editors and terminals such as VS Code, Cursor, Kiro and the JetBrains IDEs. Keeps code, commands, paths and identifiers as they are | Proofread |
| Office writing | Mail and documents in apps such as Outlook, Word, WPS, OneNote and Notion | Formal |
| Chat | Messaging apps such as WeChat, QQ, Slack, Teams, DingTalk, Feishu, Telegram and Discord | Casual chat |
| Legal, Medical, Finance, Academic | Strict proofreading that keeps specialist terms as they are and writes numbers, units and dates correctly. They apply once you add the apps you use | Proofread |

Built-in scenes are off until you turn them on. They are listed after your own scenes and marked **Built-in**, so your own scenes take precedence. You can change a built-in scene's apps, preset and instructions, but not its name, and it cannot be deleted. **Restore default** puts back its apps, preset and instructions and leaves it on or off.

When a built-in scene applies, its terms are given to recognition and AI polish together with your dictionary, and they do not count towards the dictionary's limits. The scene shows how many terms it has; Chat has none.
