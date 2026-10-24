---
description: The built-in service, the cloud providers Voltip supports for recognition and AI polish (Alibaba Cloud Model Studio's realtime models among them), and how their keys are stored.
---

# Cloud services

Instead of a local model, recognition and AI polish can use a service on the internet. Each has its own choice: recognition on the **Speech models** page, AI polish on the **AI models** page.

## The built-in service

Release packages come with a built-in service, so dictation and AI polish work before you set anything up. It is run by the Voltip project and needs no key. Builds from source have no built-in service.

For AI polish the built-in service offers several models: Qwen3.8 27B (the default), GPT-OSS 120B and GPT-OSS 20B. Choose one in the built-in service's settings on the **AI models** page. They are also offered as fallback models (see [When a model's quota runs out](#when-a-model-s-quota-runs-out)): each model has its own quota.

With the built-in service, the overlay shows the words while you speak (live preview), with no live transcription model to download: the sentence you are saying is sent about every second, and the preview updates about every 2 seconds. The final text is still the transcript of the whole recording after you release the shortcut. **Live preview** can be turned off under **Speech models → Recognition**.

The built-in service is shared by everyone who uses it. When it has too many requests, or its quota is used up, it may skip AI polish for a while: the dictation still inserts the recognised text, the overlay says it was not polished and why, the home page (on the phone, the talk screen) shows a notice, and the history's details say why too. For AI polish you can rely on, use a provider of your own, for example with a free Groq key, set up as described under [Setting up a provider](#setting-up-a-provider). The notice goes away once a dictation is polished again or another provider is in use; once closed, it stays away for a day.

When a connection to a recognition or polish service drops before the request is sent (for example `tls handshake eof`, which happens now and then on the network path from mainland China to the built-in service), Voltip tries again twice within about a second and a half. Only when every attempt fails does the dictation report the error, with the number of attempts at the end, for example `(3 attempts)`. This applies to the built-in service and to your own providers.

## Providers

| Provider | Recognition | AI polish | Key |
| --- | --- | --- | --- |
| Built-in service | Yes | Yes | Not needed |
| This computer | Yes, with a [local model](/recognition/local) | — | Not needed |
| OpenAI | Yes | Yes | Required |
| Groq | Yes | Yes | Required |
| Google AI Studio | — | Yes, Gemini models | Required |
| SiliconFlow | Yes | Yes | Required |
| Alibaba Cloud Model Studio | Yes, realtime models included | Yes | Required |
| DeepSeek | — | Yes | Required |
| Ollama, on this computer | — | Yes | Not needed |
| Custom, any OpenAI-compatible endpoint | Yes | Yes | Optional; the endpoint address is required |

A provider that offers both services uses one key for both. The provider cards show each provider's logo.

Recognition can also run on a GPU server of your own, used as the custom endpoint; see [Self-hosted recognition](/recognition/self-host).

## Setting up a provider

1. On the Speech models or AI models page, select the provider's card. Its settings open in a panel on the right, or from the bottom on the phone.
2. Choose a model, and for a custom endpoint enter its address. Once a key is saved, the model list shows every model the provider offers after **Other model…** (the **From the provider** group, with their number), updated each time the list opens, so there is no need to type a model ID.
3. Enter your key. **Test connection** checks the address and the key and shows how many models the provider offers, the same number as in the model list. The Speech models page lists speech recognition models only, and the AI models page leaves out speech, image and embedding models.
4. Choose **Set as current** to save what you entered and switch to the provider. **Set as current** on a card switches to the provider without opening its settings.

## Google AI Studio

Google AI Studio offers Gemini models for AI polish. Create a key at [aistudio.google.com/apikey](https://aistudio.google.com/apikey) (**Get a key** in the provider's settings opens it) and enter it in the Google AI Studio settings on the **AI models** page. The model list suggests Gemini models with a free tier; once the key is saved, it also shows every text model the key can use, so you choose from the list instead of typing a name.

On the free tier, Google may use what you send to improve its products; a key from a Google Cloud project with billing turned on is not used that way. See [Privacy](/privacy).

## The Responses interface

For AI polish, the custom provider's **Interface** can be Chat Completions (the default) or Responses. OpenAI's reasoning models and some gateways (kiro-provider, for example) speak the Responses interface.

- With Responses, **Reasoning effort** can be set (minimal, low, medium, high, xhigh); **Not set** leaves the service's default.
- The request carries no temperature and no output length limit: reasoning models and these gateways usually refuse both. It asks the service not to store the text.
- For example, a kiro-provider on this computer: endpoint `http://127.0.0.1:8787/v1`, model `claude-opus-5-5`, interface Responses, reasoning effort high. A higher effort polishes more slowly; when one polish takes longer than 30 seconds, Voltip inserts the recognised text without polish.

## Alibaba Cloud Model Studio

Model Studio's speech models use Model Studio's own interfaces, and Voltip picks the right one from the model's name:

- **Realtime models** (the default qwen-audio-3.1-asr-flash-streaming, and qwen-audio-3.1-asr-flash-message, fun-asr-realtime, paraformer-realtime-v2 and others): the recording goes to Model Studio while you speak and is recognised as it arrives. The overlay shows the words, release only waits for the last sentence, and the whole recording is not recognised a second time. This needs **Live preview** (on by default); with it off, the recording is sent when you release the shortcut, and the wait can come close to the length of the recording. With the output mode **All at once**, takes run as **While you speak**, and the text is still inserted once on release.
- **Whole-recording models** (qwen-audio-3.1-asr-flash, qwen3-asr-flash, fun-asr-flash): the whole recording is recognised after you release the shortcut, up to about 4 minutes per recording.
- Models with filetrans in their names, fun-asr, paraformer-v2 and sensevoice-v1 only transcribe recorded files in the background and cannot be used for dictation; qwen3-asr-flash-realtime is not supported yet. The model list leaves them out; with one of them typed in and selected, dictation says to pick another model.

With the endpoint left empty, Voltip uses the public address `https://dashscope.aliyuncs.com/compatible-mode/v1`. For your workspace's own address, enter it under **Endpoint**, for example `https://<workspace ID>.cn-beijing.maas.aliyuncs.com/compatible-mode/v1`; the Singapore region uses `dashscope-intl.aliyuncs.com`. A Model Studio address entered for a **Custom endpoint** follows the same rules.

The correct spellings in your [dictionary](/recognition/dictionary) go to the qwen-audio models as hot words. For AI polish with Model Studio's Qwen or DeepSeek models, Voltip turns off their thinking mode, which is on by default and makes polish slow.

Model Studio gives newly activated models a free quota, one for each model. **View quota** in the Model Studio settings opens the console's free quota page, where you see each model's remaining quota and usage: Model Studio has no API for reading the quota with an API key, so Voltip does not show it. To use the free quota only, turn on **Free Quota Only** (免费额度用完即停 in the Chinese console) on that page. Once the quota is used up, dictation says that the model's quota is used up, or Voltip moves on to another model: see the next section.

## When a model's quota runs out

Recognition and AI polish can each list fallback models: when the selected model's quota is used up, Voltip moves on to the next model in the list, and the dictation still goes through. Each of Model Studio's models has its own free quota, so you can list several models with a free quota one after another.

1. On the Speech models or AI models page, turn on **When a model's quota runs out**.
2. Below it, choose a provider and a model, and choose **Add**. A provider can be added with several of its models, up to 8 in all; each uses the address and the key set on the provider's card.
3. Use the move up and move down buttons to change the order. The first row is the selected model.

- Voltip moves on only when the provider answers that the quota is used up. Other errors, such as a network failure, a wrong key or too many requests, are reported as before and do not switch models.
- A model whose quota is used up is skipped for 24 hours and then tried again; the list shows when. **Check again** starts over with the selected model at once, and so does restarting Voltip or changing the provider's key.
- When every model's quota is used up, dictation says the model's quota is used up. Text that was already inserted stays: in **Type as you speak**, the sentences already inserted are kept, and the recognised parts of a long recording are inserted as usual.
- Fallback models are not used while the selected model is a local model, or while the selected provider cannot be used (for example, its key is missing).
- When the selected model is a realtime model, a realtime fallback model also recognises while you speak; a whole-recording fallback model recognises the recording after you release the shortcut, and meanwhile **Live preview** shows as unavailable and the output runs as **All at once**.
- The history shows the model that actually recognised and polished the text, and the **Now** line (above the providers on the computer, under the provider cards on the phone) notes when a fallback model stands in.
- Model Studio needs **Free Quota Only** turned on for these models in its console: then it refuses requests once a quota is used up and Voltip moves on to the next model; without it, Model Studio starts charging instead. Voltip cannot read the remaining quota and does not change the console's settings; **View quota** below the list opens the console's free quota page.

## Switching models

The **Recognition and polish** card on the home page shows the speech model and the AI polish model in use, and whether they are ready; on the other pages the speech model is under the page title and the polish model next to the **AI Polish** switch. Click either name to switch without opening its page:

- The speech model menu lists the built-in service, each provider you have set up, and the local models you have downloaded.
- The AI polish model menu, to the right of the **AI Polish** switch, lists the models of the built-in service and of each provider you have set up.

Providers that are not set up do not appear in these menus. **Manage speech models…** and **Manage AI models…** at the end of the menus open the pages where you set them up.

## Keys

- Keys are stored in the system keychain: Windows Credential Manager, the macOS keychain, or the Secret Service on Linux. They are never shown again after you save them.
- Each key is sent only to the provider it belongs to. When you switch providers or change a custom address, the old key does not follow.
- On Linux, Voltip needs a running Secret Service, such as GNOME Keyring or KWallet. It does not fall back to storing keys in a file.

## Network proxy

Speech recognition, AI polish, model downloads, update checks and downloads, feedback, and the connection to your phone all reach the network through the proxy set under **Settings → Network**. On the phone, the same setting is under **Settings → General → Network**.

<ScreenFigure src="/screens/network-en-light.webp" width="1440" height="900"
  alt="Network in Settings: the proxy set to Manual, the proxy server http://127.0.0.1:7890, and the local network under No proxy for."
  caption="Settings › Network, with a manual HTTP proxy." />

- **Follow the system** (the default) uses the system's proxy settings. On a computer, Voltip reads the `HTTPS_PROXY`, `HTTP_PROXY`, `ALL_PROXY` and `NO_PROXY` environment variables first, then macOS's network settings or Windows' Internet Options; on Linux it reads the environment variables only. On the phone, it uses the proxy of the current network (Wi-Fi). The settings page shows the system's current proxy; after you change the system's settings, **Read again** reads them at once.
- **No proxy** connects directly.
- **Manual** uses the proxy server you enter, such as `http://127.0.0.1:7890` or `socks5h://127.0.0.1:1080`. `http`, `socks5` and `socks5h` (the proxy server resolves host names) are supported; a host and port alone means `http`, and the port is required. If the proxy needs a sign-in, enter the user name and password; the password is stored only in the system keychain and is never shown after you save it. **No proxy for** takes a comma-separated list of hosts, domains (with their subdomains), IP addresses or networks, such as `corp.example.com, 10.0.0.0/8`; for a first manual proxy it starts with the local network (`local, 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16`), so a model service on another computer of your network is not sent to the proxy. Change it as you need.

Addresses on this device (`localhost`, `127.0.0.1`), such as Ollama running locally and the local service, are always reached directly. After a change, every new connection follows the new setting.

Voltip does not support PAC files, automatic proxy detection (WPAD) or `https://` proxies, and it does not read the proxy settings of Linux desktop environments such as GNOME or KDE; enter the proxy under **Manual**, or set the environment variables above. Pages opened in the browser, such as a provider's console or the phone's installer download, use the browser's own proxy settings.

## What is sent

- **Recognition**: the recording, the model name and your key. If your [dictionary](/recognition/dictionary) has entries, their correct spellings go along as a hint. With a realtime model, the recording is sent while you speak.
- **AI polish**: the recognised text, never audio, together with the app's name unless you turn that off. See [Privacy](/privacy) for the details.
- **Fallback models**: with **When a model's quota runs out** turned on, the recording or the text goes to the provider of the next model in the list once the selected model's quota is used up.
- **Model lists**: when a provider's settings or a model list opens, Voltip asks that provider for its models with the saved key; no recording or text goes with it.
