---
description: What AI polish changes in the recognised text, how to turn it on or off, which services it can use, and how presets decide what it does.
---

# AI polish and presets

AI polish tidies the recognised text before it is inserted. What it does depends on the [preset](#presets) in use. The default preset, **Proofread**, fixes punctuation and typos and removes filler words and repetitions; it does not change what you said, translate it or add to it.

Turn it on or off with the **AI Polish** switch in the title bar. The two menus next to it choose the preset and the model that polishes the text. A [scene](/recognition/scenes) can turn it on or off, and choose a preset, for a particular app.

## Services

AI polish uses the service and model chosen on the **AI models** page, or from the model menu next to the **AI Polish** switch: the built-in service, OpenAI, Groq, SiliconFlow, DeepSeek, a local Ollama, or any OpenAI-compatible endpoint. The same menu is on the home page; it lists the models of the built-in service and of each provider that is set up. See [Cloud services](/recognition/cloud) for setting one up.

If the service does not answer, the recognised text is inserted without polish, and the dictation is not lost.

## Presets

A preset tells AI polish what to do with the recognised text. Choose the one in use from the **AI preset** menu on the home page or in the title bar, from **AI Polish** in the tray menu, or under **Presets** on the **AI models** page. While AI polish runs, the overlay shows the name of the preset, and each history entry records it.

| Preset | What it does |
| --- | --- |
| **Proofread** (default) | Fixes typos, homophones and punctuation, splits paragraphs, removes filler words and stutters. When you correct yourself ("no, I mean…"), only the corrected wording is kept |
| **Prompt optimizer** | Rewrites a spoken request into a clear prompt for another AI: one precise instruction for a simple request, a numbered list when there are several points |
| **Clarify intent** | Keeps the corrections, removes repetition and turns several points into a list |
| **Casual chat** | Short, conversational sentences without a closing full stop |
| **Chinese ⇄ English** | Fixes recognition errors, then translates between Chinese and English |
| **Key points** | Key points and to-dos as two lists, without adding facts |
| **Punctuation only** | Adds punctuation and sentence breaks and changes no words |
| **Formal** | Rewrites spoken phrasing into complete, formal written language |

<ScreenFigure src="/screens/ai-en-light.webp" width="1440" height="900"
  alt="The AI models page: AI polish is on, and below it the eight built-in presets with Proofread in use, then the custom presets."
  caption="The presets on the AI models page." />

### Your own presets

Under **Presets** on the **AI models** page, **New preset** creates a preset of your own, and **Copy to custom** starts one from a built-in preset, which cannot be changed itself. A preset has a name of up to 24 characters and a prompt of up to 4,000 characters that says how to handle the text. Voltip adds the output format, so the prompt does not need to describe it. You can keep up to 30 presets of your own.

**Trial run** processes a sample text with the current AI service before you save; the result is not kept.

When you delete a preset, the settings and scenes that used it switch to **Proofread**.

## What the AI service receives

- The recognised text, after the dictionary corrections. Never audio.
- The instructions of the preset in use.
- The name of the app you are dictating into. You can turn this off.
- The window title, only if you turn it on.
- The extra instructions of the scene that applies, if any.
- The correct spellings from your dictionary, and the specialist terms of a [built-in scene](/recognition/scenes#built-in-scenes) that applies, so that the service keeps them as written.

The two switches are under **Settings → Scenes**, in **Context sent to AI polish**. They apply only while AI polish is on; the recognition service never receives this context.

With the **Type as you speak** output mode, sentences are inserted as they are recognised, so AI polish does not run.
