---
description: The built-in service, the cloud providers Voltip supports for recognition and AI polish, and how their keys are stored.
---

# Cloud services

Instead of a local model, recognition and AI polish can use a service on the internet. Each has its own choice: recognition on the **Speech models** page, AI polish on the **AI models** page.

## The built-in service

Release packages come with a built-in service, so dictation and AI polish work before you set anything up. It is run by the Voltip project and needs no key. Builds from source have no built-in service.

With the built-in service, the overlay shows the words while you speak (live preview), with no live transcription model to download: the sentence you are saying is sent about every second, and the preview updates about every 2 seconds. The final text is still the transcript of the whole recording after you release the shortcut. **Live preview** can be turned off under **Speech models → Recognition**.

## Providers

| Provider | Recognition | AI polish | Key |
| --- | --- | --- | --- |
| Built-in service | Yes | Yes | Not needed |
| This computer | Yes, with a [local model](/recognition/local) | — | Not needed |
| OpenAI | Yes | Yes | Required |
| Groq | Yes | Yes | Required |
| SiliconFlow | Yes | Yes | Required |
| DeepSeek | — | Yes | Required |
| Ollama, on this computer | — | Yes | Not needed |
| Custom, any OpenAI-compatible endpoint | Yes | Yes | Optional; the endpoint address is required |

A provider that offers both services uses one key for both.

## Setting up a provider

1. On the Speech models or AI models page, expand the provider's card.
2. Choose a model, and for a custom endpoint enter its address.
3. Enter your key. **Test connection** checks the address and the key and lists the models the provider offers.
4. Choose **Use** to switch to the provider.

## Switching models

The title bar and the home page show the speech model and the AI polish model in use, and whether they are ready. Click either name to switch without opening its page:

- The speech model menu lists the built-in service, each provider you have set up, and the local models you have downloaded.
- The AI polish model menu, to the right of the **AI Polish** switch, lists the models of the built-in service and of each provider you have set up.

Providers that are not set up do not appear in these menus. **Manage speech models…** and **Manage AI models…** at the end of the menus open the pages where you set them up.

## Keys

- Keys are stored in the system keychain: Windows Credential Manager, the macOS keychain, or the Secret Service on Linux. They are never shown again after you save them.
- Each key is sent only to the provider it belongs to. When you switch providers or change a custom address, the old key does not follow.
- On Linux, Voltip needs a running Secret Service, such as GNOME Keyring or KWallet. It does not fall back to storing keys in a file.

## What is sent

- **Recognition**: the recording, the model name and your key. If your [dictionary](/recognition/dictionary) has entries, their correct spellings go along as a hint.
- **AI polish**: the recognised text, never audio, together with the app's name unless you turn that off. See [Privacy](/privacy) for the details.
