---
description: The built-in service, the cloud providers Voltip supports for recognition and AI polish (Alibaba Cloud Model Studio's realtime models among them), and how their keys are stored.
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
| Alibaba Cloud Model Studio | Yes, realtime models included | Yes | Required |
| DeepSeek | — | Yes | Required |
| Ollama, on this computer | — | Yes | Not needed |
| Custom, any OpenAI-compatible endpoint | Yes | Yes | Optional; the endpoint address is required |

A provider that offers both services uses one key for both.

## Setting up a provider

1. On the Speech models or AI models page, expand the provider's card.
2. Choose a model, and for a custom endpoint enter its address.
3. Enter your key. **Test connection** checks the address and the key and lists the models the provider offers.
4. Choose **Use** to switch to the provider.

## Alibaba Cloud Model Studio

Model Studio's speech models use Model Studio's own interfaces, and Voltip picks the right one from the model's name:

- **Realtime models** (the default qwen-audio-3.1-asr-flash-streaming, and qwen-audio-3.1-asr-flash-message, fun-asr-realtime, paraformer-realtime-v2 and others): the recording goes to Model Studio while you speak and is recognised as it arrives. The overlay shows the words, release only waits for the last sentence, and the whole recording is not recognised a second time. This needs **Live preview** (on by default); with it off, the recording is sent when you release the shortcut, and the wait can come close to the length of the recording. With the output mode **All at once**, takes run as **While you speak**, and the text is still inserted once on release.
- **Whole-recording models** (qwen-audio-3.1-asr-flash, qwen3-asr-flash, fun-asr-flash): the whole recording is recognised after you release the shortcut, up to about 4 minutes per recording.
- Models with filetrans in their names, fun-asr and paraformer-v2 only transcribe recorded files in the background and cannot be used for dictation; qwen3-asr-flash-realtime is not supported yet. With one of these selected, dictation says to pick another model.

With the endpoint left empty, Voltip uses the public address `https://dashscope.aliyuncs.com/compatible-mode/v1`. For your workspace's own address, enter it under **Endpoint**, for example `https://<workspace ID>.cn-beijing.maas.aliyuncs.com/compatible-mode/v1`; the Singapore region uses `dashscope-intl.aliyuncs.com`. A Model Studio address entered for a **Custom endpoint** follows the same rules.

The correct spellings in your [dictionary](/recognition/dictionary) go to the qwen-audio models as hot words. For AI polish with Model Studio's Qwen or DeepSeek models, Voltip turns off their thinking mode, which is on by default and makes polish slow.

Model Studio gives newly activated models a free quota. To use the free quota only, turn on **免费额度用完即停** (stop when the free quota is used up) on the console's free quota page; once the quota is used up, dictation says so.

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

- **Recognition**: the recording, the model name and your key. If your [dictionary](/recognition/dictionary) has entries, their correct spellings go along as a hint. With a realtime model, the recording is sent while you speak.
- **AI polish**: the recognised text, never audio, together with the app's name unless you turn that off. See [Privacy](/privacy) for the details.
