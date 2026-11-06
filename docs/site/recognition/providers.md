---
description: What each provider Voltip supports needs to be activated, where its credentials are and where they go, with the suggested models, notes and official documentation.
---

# Provider setup

This page shows, for each provider Voltip supports, what to activate, where to get the credentials and where to enter them. For the steps every provider shares, see [Setting up a provider](/recognition/cloud#setting-up-a-provider); for the built-in service, see [The built-in service](/recognition/cloud#the-built-in-service); for local models, see [Local recognition](/recognition/local).

When you open a provider's settings on the Speech models or AI models page, **How to set up** gives short steps and direct links: the page where the service is activated, the app is created or credit is added, **Get a key**, and **Full guide**, which opens the provider's section on this page. The page addresses below were checked against each provider's official documentation in November 2026; the provider's own website is the reference for console names, free quotas and prices.

<ScreenFigure src="/screens/providers-en-light.webp" width="1440" height="900"
  alt="Volcengine Doubao Speech settings on the Speech models page: under the model, How to set up lists three steps, followed by the links Activate the service, Get a key and Full guide."
  caption="How to set up, in a provider's settings." />

## OpenAI {#openai}

Speech recognition and AI polish, with one key for both.

1. Sign in to the [OpenAI Platform](https://platform.openai.com/) and add credit under [Billing](https://platform.openai.com/account/billing/overview). The API is billed by use.
2. Create a key on the [API keys](https://platform.openai.com/api-keys) page.
3. Enter the key in the OpenAI settings.

- Suggested models: `gpt-transcribe`, `gpt-4o-mini-transcribe` and `whisper-1` for recognition, `gpt-6-luna` and `gpt-6-sol` for polish. OpenAI shuts down `whisper-1`, `gpt-4o-transcribe` and `gpt-4o-mini-transcribe` on 26 February 2027 and recommends `gpt-transcribe`.
- Official documentation: [Quickstart](https://developers.openai.com/api/docs/quickstart), [Speech to text](https://developers.openai.com/api/docs/guides/speech-to-text).

## Groq {#groq}

Speech recognition and AI polish, with one key for both. Groq has a free plan, with rate limits on requests.

1. Sign in to the [GroqCloud console](https://console.groq.com/).
2. Create a key on the [API Keys](https://console.groq.com/keys) page.
3. Enter the key in the Groq settings.

- Suggested models: `whisper-large-v3-turbo` and `whisper-large-v3` for recognition, `qwen/qwen3.8-27b`, `openai/gpt-oss-120b` and `openai/gpt-oss-20b` for polish.
- Official documentation: [Speech to text](https://console.groq.com/docs/speech-to-text), [Rate limits](https://console.groq.com/docs/rate-limits).

## Google AI Studio {#google}

AI polish only, with Gemini models.

1. Sign in to [Google AI Studio](https://aistudio.google.com/) with a Google account and accept the terms of service.
2. Copy or create a key on the [API Keys](https://aistudio.google.com/apikey) page.
3. Enter the key in the Google AI Studio settings on the **AI models** page.

The model list suggests Gemini models with a free tier, `gemini-3.5-flash-lite` first: polish handles short texts, and Flash-Lite answers faster. Once the key is saved, the list also shows every text model the key can use, so you choose from the list instead of typing a name.

A Google model is sometimes unavailable for a while because of high demand (HTTP 503); the overlay then says **Not polished · service busy**. Try again later, or pick another model.

On the free tier, Google may use what you send to improve its products; a key from a Google Cloud project with billing turned on is not used that way. See [Privacy](/privacy).

- Official documentation: [Gemini API keys](https://aistudio.google.com/docs/api-key), [OpenAI compatibility](https://aistudio.google.com/docs/openai).

## SiliconFlow {#siliconflow}

Speech recognition and AI polish, with one key for both.

1. Sign in to [SiliconFlow](https://cloud.siliconflow.cn/) and complete identity verification on the [verification](https://cloud.siliconflow.cn/account/authentication) page (实名认证): top-ups and all the free models need it.
2. Choose **New API key** on the [API keys](https://cloud.siliconflow.cn/account/ak) page.
3. Enter the key in the SiliconFlow settings.

- Suggested models: `FunAudioLLM/SenseVoiceSmall` and `TeleAI/TeleSpeechASR` for recognition, `Qwen/Qwen3-8B` and `deepseek-ai/DeepSeek-V3` for polish.
- Official documentation: [Quickstart](https://docs.siliconflow.cn/docs/userguide/quickstart), [Identity verification](https://docs.siliconflow.cn/docs/userguide/faqs/authentication), [Speech to text API](https://docs.siliconflow.cn/docs/api/audio-transcriptions-post).

## Alibaba Cloud Model Studio {#aliyun}

Speech recognition and AI polish, with one API key for both.

1. With a verified Alibaba Cloud account, open the [Model Studio console](https://bailian.console.aliyun.com/model/market) in the Beijing region (China North 2) and accept the service agreement: Model Studio is activated and each model gets a free quota.
2. Speech recognition models need access enabled in a Model Studio workspace before they can be called and use their free quota.
3. Choose **Create API key** on the [API Key](https://bailian.console.aliyun.com/cn-beijing/model/settings/api-key) page. Each region has its own API keys and addresses, which do not work in another region.
4. Enter the API key in the Model Studio settings.

Model Studio's speech models use Model Studio's own interfaces, and Voltip picks the right one from the model's name:

- **Realtime models** (the default qwen-audio-3.1-asr-flash-streaming, and qwen-audio-3.1-asr-flash-message, fun-asr-realtime, paraformer-realtime-v2 and others): the recording goes to Model Studio while you speak and is recognised as it arrives. The overlay shows the words, release only waits for the last sentence, and the whole recording is not recognised a second time. The overlay's tag reads **Preview · Model Studio**. With **Provider's live text** off under **Live preview**, the recording is still sent and recognised while you speak, with the same result and wait, and the overlay only shows no words. With the output mode **All at once**, takes run as **While you speak**, and the text is still inserted once on release.
- **Whole-recording models** (qwen-audio-3.1-asr-flash, qwen3-asr-flash, fun-asr-flash): the whole recording is recognised after you release the shortcut, up to about 4 minutes per recording.
- Models with filetrans in their names, fun-asr, paraformer-v2 and sensevoice-v1 only transcribe recorded files in the background and cannot be used for dictation; qwen3-asr-flash-realtime is not supported yet. The model list leaves them out; with one of them typed in and selected, dictation says to pick another model.

With the endpoint left empty, Voltip uses the public address `https://dashscope.aliyuncs.com/compatible-mode/v1`. For your workspace's own address, enter it under **Endpoint**, for example `https://<workspace ID>.cn-beijing.maas.aliyuncs.com/compatible-mode/v1`; the Singapore region uses `dashscope-intl.aliyuncs.com`. A Model Studio address entered for a **Custom endpoint** follows the same rules.

The correct spellings in your [dictionary](/recognition/dictionary) go to the qwen-audio models as hot words, and to fun-asr-realtime and fun-asr-flash as context; Model Studio's other speech models take no glossary. For AI polish with Model Studio's Qwen or DeepSeek models, Voltip turns off their thinking mode, which is on by default and makes polish slow.

Model Studio gives newly activated models a free quota, one for each model; only the models of the Beijing region have one. **View quota** in the Model Studio settings opens the console's free quota page, where you see each model's remaining quota and usage: Model Studio has no API for reading the quota with an API key, so Voltip does not show it. To use the free quota only, turn on **Free Quota Only** (免费额度用完即停 in the Chinese console) on that page. Once the quota is used up, dictation says that the model's quota is used up, or Voltip moves on to another model: see [When a model's quota runs out](/recognition/cloud#when-a-model-s-quota-runs-out).

- Official documentation: [Get an API key](https://help.aliyun.com/zh/model-studio/get-api-key), [Free quota for new users](https://help.aliyun.com/zh/model-studio/new-free-quota).

## Volcengine Doubao Speech {#volcengine-speech}

Speech recognition only, while you speak. Volcengine Ark keys do not work here.

1. Create a Volcengine account and complete identity verification.
2. In the new Doubao Speech console, activate the streaming speech recognition model under [Activation management](https://console.volcengine.com/speech/new/setting/activate?projectName=default) (开通管理). A new user can activate every model for the default project at once and receives a free trial package.
3. Choose **Create API Key** under [API Key management](https://console.volcengine.com/speech/new/setting/apikeys?projectName=default).
4. Enter the API key in the Volcengine Doubao Speech settings, and choose the resource ID of the service you activated as the model.

- The model is the service's resource ID: `volc.seedasr.sauc.duration` is the hourly plan of the Doubao streaming speech recognition model 2.0 (the default), `volc.bigasr.sauc.duration` the hourly plan of 1.0, `volc.seedasr.sauc.concurrent` the concurrency plan of 2.0.
- With the old console, create an app under [App management](https://console.volcengine.com/speech/app) and note its App ID and Access Token; in the settings, choose **Old console: App ID and Access Token** under **Credential type** and enter those two. Volcengine is retiring the old console and recommends the new one.
- Official documentation: [New console quick start](https://docs.volcengine.com/docs/doubao-voice/quick-start-new-console), [Using an API key](https://docs.volcengine.com/docs/doubao-voice/api-key-usage), [Streaming speech recognition API](https://docs.volcengine.com/docs/doubao-voice/large-model-streaming-automatic-speech-recognition-api-1).

## Tencent Cloud Speech Recognition {#tencent}

Speech recognition only, while you speak. It needs three credentials, the AppId, SecretId and SecretKey; Tencent Hunyuan (TokenHub) keys do not work here.

1. Open the [speech recognition console](https://console.cloud.tencent.com/asr), complete identity and face verification, accept the user agreement and choose **Activate** (立即开通). Each service then has a free quota, shown under [Resource packages](https://console.cloud.tencent.com/asr/resourcebundle).
2. Choose **Create key** on the [API key management](https://console.cloud.tencent.com/cam/capi) page (API 密钥管理) and copy the SecretId and SecretKey.
3. Copy the APPID from [Account center](https://console.cloud.tencent.com/developer) (账号中心).
4. Enter all three in the Tencent Cloud Speech Recognition settings.

- The model is the engine type: `16k_zh_en` covers Chinese, English, Cantonese and many dialects, `16k_zh` is general Chinese, `Hy-ASR-3.0-preview` covers Chinese, English and 20 dialects and takes at most 5 minutes at a time, so a longer recording is recognised in parts.
- Official documentation: [Getting started with the server API](https://cloud.tencent.com/document/product/1093/54362), [Realtime speech recognition (WebSocket)](https://cloud.tencent.com/document/product/1093/48982), [Billing overview](https://cloud.tencent.com/document/product/1093/35686).

## iFlytek Dictation {#iflytek}

Speech recognition only, while you speak. One session takes at most 60 seconds, and a longer recording carries on in a new one.

1. Sign in to the [iFlytek Open Platform](https://www.xfyun.cn/) and create an app under [My apps](https://console.xfyun.cn/app/myapp) (我的应用). A new app's dictation takes 500 calls a day by default.
2. Choose the app on the [dictation service page](https://console.xfyun.cn/services/iat) and copy its APPID, APIKey and APISecret.
3. Enter all three in the iFlytek Dictation settings.

- Hot words can only be uploaded for the app on the service page; Voltip does not send your dictionary's entries to iFlytek.
- Official documentation: [Dictation (streaming) WebAPI](https://www.xfyun.cn/doc/asr/voicedictation/API.html), [Quick guide](https://www.xfyun.cn/doc/platform/quickguide.html).

## iFlytek Realtime Transcription {#iflytek-rtasr}

Speech recognition only, while you speak, with no length limit, which suits long recordings. Voltip uses the standard edition of realtime transcription.

1. On the [realtime transcription product page](https://www.xfyun.cn/services/rtasr), choose the standard edition (实时语音转写标准版) and claim its free trial package (5 hours for individuals, 50 hours for companies, valid for a year). The large-model edition (实时语音转写大模型) that the page shows first is another service, and its packages do not work here.
2. Open your app under [My apps](https://console.xfyun.cn/app/myapp) (the dictation app will do) and go to its realtime transcription service page.
3. Copy the APPID and this service's APIKey into the iFlytek Realtime Transcription settings. The APIKey is created when the service is added to the app and differs from dictation's.

- Official documentation: [Realtime transcription (standard edition)](https://www.xfyun.cn/doc/asr/rtasr/API.html).

## Baidu AI Cloud Speech {#baidu}

Speech recognition only, while you speak. It needs the app's AppID and API Key.

1. Create a [Baidu AI Cloud](https://cloud.baidu.com/) account, complete identity verification and open the [speech console](https://console.bce.baidu.com/ai-engine/speech/overview/index). The free test resources arrive by themselves within 5–10 minutes and show under **Resource list** (资源列表).
2. Under **App list** (应用列表) on the left, choose **Create app** (创建应用), enter a name, tick the service interfaces to call and choose **Create now**.
3. Copy the app's AppID and API Key from the app list into the Baidu AI Cloud Speech settings.

- The model is the recognition model number: `15372` is Mandarin with fuller punctuation, `1537` Mandarin, `15376` Chinese dialects (Mandarin, Cantonese, Sichuanese and Northeastern).
- Official documentation: [Free resources and app creation](https://cloud.baidu.com/doc/SPEECH/s/4l9mh6qf9), [Realtime speech recognition WebSocket API](https://cloud.baidu.com/doc/SPEECH/s/jlbxejt2i).

## StepFun {#stepfun}

Speech recognition only. Switch between the China and international sites, which keep separate accounts and keys.

1. Sign up on the StepFun platform: [platform.stepfun.com](https://platform.stepfun.com/) is the China site, [platform.stepfun.ai](https://platform.stepfun.ai/) the international one. The China site needs identity verification first, under Account management › Account information (账户管理 › 账户信息); pay-as-you-go charges come out of the account balance.
2. Create an API key on the key page: [China site](https://platform.stepfun.com/interface-key) (接口密钥), [international site](https://platform.stepfun.ai/interface-key) (Interface Keys).
3. In the StepFun settings, choose the site you signed up on and enter the API key.

- Models: `stepaudio-2.5-asr` recognises whole recordings; `stepaudio-2.5-asr-stream` recognises while you speak, and a recording made earlier (to recognise it again, for example) goes to `stepaudio-2.5-asr`.
- Official documentation: [China site quick start](https://platform.stepfun.com/docs/zh/quickstart/overview), [Streaming speech recognition](https://platform.stepfun.com/docs/zh/api-reference/audio/asr-stream), [International site quick start](https://platform.stepfun.ai/docs/en/quickstart/overview).

## DeepSeek {#deepseek}

AI polish only.

1. Sign in to the [DeepSeek platform](https://platform.deepseek.com/) and top up on the [Top up](https://platform.deepseek.com/top_up) page. Requests are refused when the balance runs out.
2. Create an API key on the [API keys](https://platform.deepseek.com/api_keys) page.
3. Enter it in the DeepSeek settings on the **AI models** page.

- Suggested models: `deepseek-flash` and `deepseek-v4-pro`.
- Official documentation: [Your first API call](https://api-docs.deepseek.com/zh-cn/), [Error codes](https://api-docs.deepseek.com/zh-cn/quick_start/error_codes).

## Kimi {#kimi}

AI polish only. Switch between the China and international sites, which keep separate accounts, balances and keys.

1. Sign up on the Kimi API platform: [platform.kimi.com](https://platform.kimi.com/) is the China site, [platform.kimi.ai](https://platform.kimi.ai/) the international one.
2. The suggested Kimi K3.1 Pro models need a top-up first: at least ¥10 on the China site, where the voucher given at sign-up does not cover K3.1 Pro, and at least $1 on the international site.
3. Create an API key on the API Keys page: [China site](https://platform.kimi.com/console/api-keys), [international site](https://platform.kimi.ai/console/api-keys).
4. In the Kimi settings, choose the site you signed up on and enter the API key.

- Voltip suggests the K3 models, `kimi-k3.1-pro-highspeed` first. Kimi's K2 models are closed to new accounts from 31 October 2026.
- Official documentation: [Get an API key](https://platform.kimi.com/docs/get-api-key), [Kimi K3.1 Pro quick start](https://platform.kimi.com/docs/guide/kimi-k3-1-pro-quickstart), [Get your API key (international)](https://platform.kimi.ai/docs/get-your-api-key).

## Zhipu GLM {#glm}

Speech recognition and AI polish, with one key for both. Switch between the China site (the Zhipu open platform) and the international site (Z.ai), which keep separate accounts and keys.

1. Sign up and sign in on the [Zhipu open platform](https://bigmodel.cn/) or [Z.ai](https://z.ai/model-api).
2. Create a standard API key: on the [API Keys](https://bigmodel.cn/usercenter/proj-mgmt/apikeys) page of the China site, or on the [API Keys](https://z.ai/manage-apikey/apikey-list) page of the international site. Since 16 October 2026, standard API keys and GLM Coding Plan keys are kept apart and do not work in place of each other.
3. In the Zhipu GLM settings, choose the site you signed up on and enter the API key.

- For AI polish, Voltip suggests `glm-5.3-flash`; `glm-4.7-flash` is free on both sites. To pay as you go on the international site, top up under [Billing](https://z.ai/manage-apikey/billing).
- Speech recognition uses `glm-asr-2512`, for whole recordings. One request takes at most 30 seconds, so a longer recording is recognised in parts cut at its pauses, each with the text recognised before it.
- Official documentation: [Quick start](https://docs.bigmodel.cn/cn/guide/start/quick-start), [API key types](https://docs.bigmodel.cn/cn/coding-plan/notice/api-key-management), [The `glm-asr-2512` model](https://docs.bigmodel.cn/cn/guide/models/sound-and-video/glm-asr-2512), [Z.ai quick start](https://docs.z.ai/guides/overview/quick-start).

## Volcengine Ark {#volcengine}

AI polish only, with Doubao and other models. Doubao Speech credentials do not work here.

1. In the Volcengine Ark console, choose **Activate** for each model you want under [Activation management](https://ark.volcengine.com/region:cn-beijing/openManagement) (开通管理). A model that is not activated refuses requests.
2. Choose **Create API Key** under [API Key management](https://ark.volcengine.com/region:cn-beijing/apikey).
3. Enter it in the Volcengine Ark settings on the **AI models** page.

- Voltip suggests the Doubao Seed models.
- Official documentation: [Quick start](https://docs.volcengine.com/docs/ark/quick-start?lang=zh), [Get an API key](https://docs.volcengine.com/docs/ark/api-key?lang=zh), [Activation management](https://docs.volcengine.com/docs/ark/activation-management?lang=zh).

## Tencent Hunyuan {#hunyuan}

AI polish only, through Tencent Cloud's model platform TokenHub, which also offers other vendors' models. Tencent Cloud Speech Recognition credentials do not work here.

1. With a verified Tencent Cloud account, sign in to the [TokenHub console](https://console.cloud.tencent.com/tokenhub) and activate the service as prompted.
2. Enable the models you want under [Enable management](https://console.cloud.tencent.com/tokenhub/open-management) (启用管理). Language models come with a free trial quota for new users; once it is used up, or for a model without one, turn on pay-as-you-go.
3. On the [API Key management](https://console.cloud.tencent.com/tokenhub/apikey) page, choose the Guangzhou region at the top, then choose **Create API Key**.
4. Enter it in the Tencent Hunyuan settings on the **AI models** page.

- Voltip suggests `hy3` and `hy4-preview`. For the Singapore site, enter its address `https://tokenhub-intl.tencentmaas.com/v1` as the endpoint address and use a key made there; neither site serves the other's requests.
- Official documentation: [Quick start](https://cloud.tencent.com/document/product/1823/130058), [Enable management](https://cloud.tencent.com/document/product/1823/137795), [Free trial for new users](https://cloud.tencent.com/document/product/1823/130053), [API key management](https://cloud.tencent.com/document/product/1823/130090).

## Xiaomi MiMo {#mimo}

Speech recognition and AI polish, with one key for both.

1. Sign in to the [Xiaomi MiMo platform](https://platform.xiaomimimo.com/) with a Xiaomi account. Accounts in mainland China need personal or company identity verification before a top-up or a plan purchase.
2. Top up under [Account balance](https://platform.xiaomimimo.com/#/console/balance). Requests are refused when the balance runs out.
3. Create an API key under [API Keys](https://platform.xiaomimimo.com/#/console/api-keys).
4. Enter it in the Xiaomi MiMo settings.

- For AI polish, Voltip suggests `mimo-v2.6-flash` and `mimo-v2.6-pro`. With a Token Plan, enter the plan's own address as the endpoint address and use the plan's key.
- Speech recognition uses `mimo-v2.5-asr`, for whole recordings; one longer than 3 minutes is recognised in parts cut at its pauses.
- Official documentation: [Your first API call](https://mimo.mi.com/docs/zh-CN/quick-start/summary/first-api-call), [Payment](https://mimo.mi.com/docs/zh-CN/quick-start/faq/payment), [Speech recognition](https://mimo.mi.com/docs/zh-CN/quick-start/usage-guide/audio/Speech-Recognition).

## OpenRouter {#openrouter}

AI polish only: many vendors' models with one key.

1. Create an [OpenRouter](https://openrouter.ai/) account and buy credits on the [Credits](https://openrouter.ai/settings/credits) page. Free models take 50 requests a day, or 1000 a day once you have bought at least 10 credits.
2. Create a key on the [Keys](https://openrouter.ai/settings/keys) page; you can give it a credit limit.
3. Enter it in the OpenRouter settings on the **AI models** page.

- Voltip suggests fast, inexpensive models such as `google/gemini-3.5-flash-lite`.
- Official documentation: [Quickstart](https://openrouter.ai/docs/quickstart), [FAQ](https://openrouter.ai/docs/faq).

## Ollama {#ollama}

AI polish that runs on this computer: no key, and the text never leaves the computer.

1. Download and install Ollama from [ollama.com](https://ollama.com/download), and keep it running. On Linux, run `ollama serve` in a terminal if Ollama is not already running in the background.
2. Run `ollama pull <model name>` in a terminal to download a model; the [Ollama library](https://ollama.com/library) lists the models.
3. In the Ollama settings on the **AI models** page, leave the endpoint empty to use `http://127.0.0.1:11434/v1`, then choose a downloaded model in the model list.

- Official documentation: [Quickstart](https://docs.ollama.com/quickstart), [OpenAI compatibility](https://docs.ollama.com/api/openai-compatibility).

## Custom endpoint {#custom}

Any OpenAI-compatible endpoint, such as vLLM, LocalAI or your own gateway, for speech recognition or AI polish. Recognition and polish each have their own address and key, so they can be two different servers.

1. Enter the endpoint's address under **Endpoint**, usually ending in `/v1`, for example `http://127.0.0.1:8000/v1`.
2. Enter a key if the endpoint needs one, then enter a model ID, or choose one from the list after testing the connection.
3. Choose **Test connection** to check the address and the key.

To run recognition on a GPU server of your own, see [Self-hosted recognition](/recognition/self-host). A Model Studio address follows the rules of [Alibaba Cloud Model Studio](#aliyun).

### The Responses interface

For AI polish, the custom provider's **Interface** can be Chat Completions (the default) or Responses. OpenAI's reasoning models and some gateways (kiro-provider, for example) speak the Responses interface.

- With Responses, **Reasoning effort** can be set (minimal, low, medium, high, xhigh); **Not set** leaves the service's default.
- The request carries no temperature and no output length limit: reasoning models and these gateways usually refuse both. It asks the service not to store the text.
- For example, a kiro-provider on this computer: endpoint `http://127.0.0.1:8787/v1`, model `claude-opus-5-5`, interface Responses, reasoning effort high. A higher effort polishes more slowly; when one polish takes longer than 30 seconds, Voltip inserts the recognised text without polish.
