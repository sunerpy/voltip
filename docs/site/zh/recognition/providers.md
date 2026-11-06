---
description: Voltip 支持的每个服务商需要开通什么、在哪里获取凭据、填写到哪里，以及推荐的模型、注意事项和官方文档。
---

# 服务商设置

本页说明 Voltip 支持的每个服务商需要开通什么、在哪里获取凭据，以及填写到哪里。通用的设置步骤见[设置服务商](/zh/recognition/cloud#设置服务商)，内置服务见[内置服务](/zh/recognition/cloud#内置服务)，本地模型见[本地识别](/zh/recognition/local)。

在「语音模型」或「AI 模型」页打开服务商的设置，「配置方法」中有简要的步骤和直达链接：开通服务、创建应用或充值的页面，「获取密钥」，以及打开本页对应一节的「完整说明」。下面的页面地址于 2026 年 11 月按各服务商的官方文档核对；控制台的名称、免费额度和价格以服务商的网站为准。

<ScreenFigure src="/screens/providers-zh-light.webp" width="1440" height="900"
  alt="「语音模型」页中火山引擎豆包语音的设置：模型下方是「配置方法」，列出三条步骤，下面是「开通服务」「获取密钥」和「完整说明」三个链接。"
  caption="服务商设置中的「配置方法」。" />

## OpenAI {#openai}

用于语音识别和 AI 润色，两项服务共用一把密钥。

1. 登录 [OpenAI Platform](https://platform.openai.com/)，在 [Billing](https://platform.openai.com/account/billing/overview) 中添加余额。API 按用量计费。
2. 在 [API keys](https://platform.openai.com/api-keys) 页面创建密钥。
3. 在 OpenAI 的设置中填写密钥。

- 推荐模型：识别为 `gpt-transcribe`、`gpt-4o-mini-transcribe` 和 `whisper-1`，润色为 `gpt-6-luna` 和 `gpt-6-sol`。OpenAI 将于 2027 年 2 月 26 日停用 `whisper-1`、`gpt-4o-transcribe` 和 `gpt-4o-mini-transcribe`，建议使用 `gpt-transcribe`。
- 官方文档：[快速入门](https://developers.openai.com/api/docs/quickstart)、[语音转文字](https://developers.openai.com/api/docs/guides/speech-to-text)。

## Groq {#groq}

用于语音识别和 AI 润色，两项服务共用一把密钥。Groq 有免费套餐，请求次数受速率限制。

1. 登录 [GroqCloud 控制台](https://console.groq.com/)。
2. 在 [API Keys](https://console.groq.com/keys) 页面创建密钥。
3. 在 Groq 的设置中填写密钥。

- 推荐模型：识别为 `whisper-large-v3-turbo` 和 `whisper-large-v3`，润色为 `qwen/qwen3.8-27b`、`openai/gpt-oss-120b` 和 `openai/gpt-oss-20b`。
- 官方文档：[语音转文字](https://console.groq.com/docs/speech-to-text)、[速率限制](https://console.groq.com/docs/rate-limits)。

## Google AI Studio {#google}

只用于 AI 润色，提供 Gemini 模型。

1. 用 Google 账号登录 [Google AI Studio](https://aistudio.google.com/)，并接受服务条款。
2. 在 [API Keys](https://aistudio.google.com/apikey) 页面复制或创建密钥。
3. 在「AI 模型」页 Google AI Studio 的设置中填写密钥。

模型列表里推荐的是有免费额度的 Gemini 模型，第一个是 `gemini-3.5-flash-lite`：润色只处理短文字，Flash-Lite 响应更快。保存密钥后，列表还会列出这把密钥能用的全部文本模型，直接从列表中选择，不必手动填写模型名。

Google 的模型有时会因请求过多暂时不可用（HTTP 503），这时悬浮窗显示「未润色 · 服务繁忙」。可以稍后再试，或换一个模型。

使用免费额度时，Google 可能用你发送的内容改进其产品；绑定了结算账号的 Google Cloud 项目的密钥不会这样使用。见[隐私](/zh/privacy)。

- 官方文档：[Gemini API 密钥](https://aistudio.google.com/docs/api-key)、[OpenAI 兼容接口](https://aistudio.google.com/docs/openai)。

## 硅基流动 {#siliconflow}

用于语音识别和 AI 润色，两项服务共用一把密钥。

1. 登录[硅基流动](https://cloud.siliconflow.cn/)，在[实名认证](https://cloud.siliconflow.cn/account/authentication)页面完成认证：完成认证后才能充值，并使用全部免费模型。
2. 在 [API 密钥](https://cloud.siliconflow.cn/account/ak)页面选择「新建 API 密钥」。
3. 在硅基流动的设置中填写密钥。

- 推荐模型：识别为 `FunAudioLLM/SenseVoiceSmall` 和 `TeleAI/TeleSpeechASR`，润色为 `Qwen/Qwen3-8B` 和 `deepseek-ai/DeepSeek-V3`。
- 官方文档：[快速上手](https://docs.siliconflow.cn/docs/userguide/quickstart)、[实名认证](https://docs.siliconflow.cn/docs/userguide/faqs/authentication)、[语音转文本接口](https://docs.siliconflow.cn/docs/api/audio-transcriptions-post)。

## 阿里云百炼 {#aliyun}

用于语音识别和 AI 润色，两项服务共用一把 API Key。

1. 完成阿里云实名认证后，打开[百炼控制台](https://bailian.console.aliyun.com/model/market)，选择华北2（北京）地域，阅读并同意服务协议：百炼随即开通，并为各模型发放免费额度。
2. 语音识别模型需要先在百炼控制台的业务空间中开通权限，才能调用并使用免费额度。
3. 在 [API Key](https://bailian.console.aliyun.com/cn-beijing/model/settings/api-key) 页面选择「创建 API Key」。每个地域的 API Key 和接入地址各自独立，不能混用。
4. 在阿里云百炼的设置中填写 API Key。

百炼的语音识别模型使用百炼自己的接口，Voltip 按模型名称选择对应的接口：

- **实时识别模型**（默认的 qwen-audio-3.1-asr-flash-streaming，以及 qwen-audio-3.1-asr-flash-message、fun-asr-realtime、paraformer-realtime-v2 等）：录音的同时发送给百炼并完成识别，悬浮窗显示识别中的文字，松开快捷键后只需等待最后一句，整段录音不会再识别一次。悬浮窗的标签显示「预览 · 百炼」。在「实时预览」中关闭「服务商的实时文字」后，录音仍在说话过程中发送并识别，结果和等待时间不变，只是悬浮窗不显示文字。输出方式选择「整段输出」时，按「边说边识别」运行，文字仍在松开后一次插入。
- **整段识别模型**（qwen-audio-3.1-asr-flash、qwen3-asr-flash、fun-asr-flash）：松开快捷键后识别整段录音，单次录音最长约 4 分钟。
- 名称中含 filetrans 的模型、fun-asr、paraformer-v2 和 sensevoice-v1 只能在后台转写录音文件，不能用于听写；qwen3-asr-flash-realtime 暂不支持。模型列表中不列出这些模型；手动填写并选择它们时，听写会提示改用其他模型。

接口地址留空时使用公共地址 `https://dashscope.aliyuncs.com/compatible-mode/v1`。使用业务空间的专属地址时，在「接口地址」中填写，例如 `https://<业务空间 ID>.cn-beijing.maas.aliyuncs.com/compatible-mode/v1`；新加坡地域使用 `dashscope-intl.aliyuncs.com`。在「自定义接口」中填写百炼的地址时，同样按上面的规则识别。

[词典](/zh/recognition/dictionary)中的正确写法会作为热词发送给 qwen-audio 系列模型，作为上下文发送给 fun-asr-realtime 和 fun-asr-flash；百炼的其他识别模型不接收术语表。AI 润色使用百炼的千问或 DeepSeek 模型时，Voltip 关闭它们默认开启的思考模式，以免润色变慢。

百炼为新开通的模型提供免费额度，每个模型各有一份；只有华北2（北京）地域的模型有免费额度。阿里云百炼设置中的「查看额度」会打开百炼控制台的「免费额度」页，在那里查看各模型的剩余额度和用量：百炼不提供用 API 密钥查询额度的接口，Voltip 不显示额度和用量。如果只想使用免费额度，可以在该页开启「免费额度用完即停」；额度用完后，听写会提示「模型额度已用完」，也可以[改用其他模型](/zh/recognition/cloud#额度用完后改用其他模型)。

- 官方文档：[获取 API Key](https://help.aliyun.com/zh/model-studio/get-api-key)、[新人免费额度](https://help.aliyun.com/zh/model-studio/new-free-quota)。

## 火山引擎豆包语音 {#volcengine-speech}

只用于语音识别，边说边识别。火山方舟的密钥在这里不能使用。

1. 注册火山引擎账号并完成实名认证。
2. 在豆包语音新版控制台的[开通管理](https://console.volcengine.com/speech/new/setting/activate?projectName=default)中开通流式语音识别模型。新用户可以为默认项目一键开通全部模型，并获得免费试用礼包。
3. 在 [API Key 管理](https://console.volcengine.com/speech/new/setting/apikeys?projectName=default)中选择「创建 API Key」。
4. 在火山引擎豆包语音的设置中填写 API Key，模型选择与已开通的服务一致的资源 ID。

- 模型为服务的资源 ID：`volc.seedasr.sauc.duration` 是豆包流式语音识别模型 2.0 的小时版（默认），`volc.bigasr.sauc.duration` 是 1.0 的小时版，`volc.seedasr.sauc.concurrent` 是 2.0 的并发版。
- 使用旧版控制台时，在[应用管理](https://console.volcengine.com/speech/app)中创建应用，记下 App ID 和 Access Token；在设置的「凭据类型」中选择「旧版控制台：App ID 与 Access Token」，再填写这两项。火山引擎的旧版控制台将逐步下线，建议改用新版控制台。
- 官方文档：[新版控制台快速入门](https://docs.volcengine.com/docs/doubao-voice/quick-start-new-console)、[API Key 使用](https://docs.volcengine.com/docs/doubao-voice/api-key-usage)、[大模型流式语音识别 API](https://docs.volcengine.com/docs/doubao-voice/large-model-streaming-automatic-speech-recognition-api-1)。

## 腾讯云语音识别 {#tencent}

只用于语音识别，边说边识别。需要 AppId、SecretId 和 SecretKey 三项凭据；腾讯混元（TokenHub）的密钥在这里不能使用。

1. 打开[语音识别控制台](https://console.cloud.tencent.com/asr)，完成实名认证和人脸认证，勾选「我已阅读并同意《用户协议》」后选择「立即开通」。开通后，各项服务有免费调用额度，可以在[资源包管理](https://console.cloud.tencent.com/asr/resourcebundle)中查看。
2. 在 [API 密钥管理](https://console.cloud.tencent.com/cam/capi)页面选择「新建密钥」，复制 SecretId 和 SecretKey。
3. 在[账号中心](https://console.cloud.tencent.com/developer)复制 APPID。
4. 在腾讯云语音识别的设置中填写这三项。

- 模型为引擎类型：`16k_zh_en` 支持中文、英语、粤语和多种方言，`16k_zh` 为中文通用，`Hy-ASR-3.0-preview` 支持中英文和 20 种方言，单次识别最长 5 分钟，更长的录音分段识别。
- 官方文档：[一分钟接入服务端 API](https://cloud.tencent.com/document/product/1093/54362)、[实时语音识别（WebSocket）](https://cloud.tencent.com/document/product/1093/48982)、[计费概述](https://cloud.tencent.com/document/product/1093/35686)。

## 讯飞语音听写 {#iflytek}

只用于语音识别，边说边识别。单次识别最长 60 秒，录音更长时自动接着识别。

1. 登录[讯飞开放平台](https://www.xfyun.cn/)，在[我的应用](https://console.xfyun.cn/app/myapp)中创建应用。新应用的语音听写默认每天可调用 500 次。
2. 在[语音听写的服务页](https://console.xfyun.cn/services/iat)选择这个应用，复制 APPID、APIKey 和 APISecret。
3. 在讯飞语音听写的设置中填写这三项。

- 热词只能在服务页为应用上传，Voltip 不向讯飞发送词典的词条。
- 官方文档：[语音听写（流式版）WebAPI](https://www.xfyun.cn/doc/asr/voicedictation/API.html)、[快速指引](https://www.xfyun.cn/doc/platform/quickguide.html)。

## 讯飞实时语音转写 {#iflytek-rtasr}

只用于语音识别，边说边识别，不限时长，适合长录音。Voltip 使用的是实时语音转写标准版。

1. 在[实时语音转写的产品页](https://www.xfyun.cn/services/rtasr)选择「实时语音转写标准版」，领取免费试用包（个人 5 小时、企业 50 小时，有效期一年）。产品页默认显示的「实时语音转写大模型」是另一项服务，它的套餐在这里不能使用。
2. 在[我的应用](https://console.xfyun.cn/app/myapp)中打开应用（可以使用语音听写的同一个应用），进入实时语音转写的服务页。
3. 复制 APPID 和这项服务的 APIKey，填到讯飞实时语音转写的设置中。这个 APIKey 在应用添加实时语音转写服务时生成，与语音听写的不同。

- 官方文档：[实时语音转写（标准版）](https://www.xfyun.cn/doc/asr/rtasr/API.html)。

## 百度智能云语音 {#baidu}

只用于语音识别，边说边识别。需要应用的 AppID 和 API Key。

1. 注册[百度智能云](https://cloud.baidu.com/)账号并完成实名认证，然后打开[语音技术控制台](https://console.bce.baidu.com/ai-engine/speech/overview/index)。免费测试资源会在 5–10 分钟内自动发放，可以在「资源列表」中查看。
2. 在左侧「应用列表」中选择「创建应用」，填写应用名称、勾选要调用的服务接口，然后选择「立即创建」。
3. 在应用列表中复制应用的 AppID 和 API Key，填到百度智能云语音的设置中。

- 模型为识别模型编号：`15372` 是普通话（加强标点），`1537` 是普通话，`15376` 是多方言（中文、粤语、四川话和东北话）。
- 官方文档：[资源领取与应用创建](https://cloud.baidu.com/doc/SPEECH/s/4l9mh6qf9)、[实时语音识别 WebSocket API](https://cloud.baidu.com/doc/SPEECH/s/jlbxejt2i)。

## 阶跃星辰 {#stepfun}

只用于语音识别。可切换中国站与国际站，两个站点的账号和密钥不通用。

1. 在阶跃星辰开放平台注册：中国站为 [platform.stepfun.com](https://platform.stepfun.com/)，国际站为 [platform.stepfun.ai](https://platform.stepfun.ai/)。中国站需要先在「账户管理 › 账户信息」中完成实名认证；按量计费的费用从账户余额中扣除。
2. 在接口密钥页面创建 API Key：[中国站](https://platform.stepfun.com/interface-key)、[国际站](https://platform.stepfun.ai/interface-key)。
3. 在阶跃星辰的设置中选择注册的站点，填写 API Key。

- 模型：`stepaudio-2.5-asr` 识别整段录音；`stepaudio-2.5-asr-stream` 边说边识别，已录好的录音（例如重新识别）由 `stepaudio-2.5-asr` 识别。
- 官方文档：[中国站快速入门](https://platform.stepfun.com/docs/zh/quickstart/overview)、[流式语音识别](https://platform.stepfun.com/docs/zh/api-reference/audio/asr-stream)、[国际站快速入门](https://platform.stepfun.ai/docs/en/quickstart/overview)。

## DeepSeek {#deepseek}

只用于 AI 润色。

1. 登录 [DeepSeek 开放平台](https://platform.deepseek.com/)，在[充值](https://platform.deepseek.com/top_up)页面充值。余额不足时，请求会被拒绝。
2. 在 [API keys](https://platform.deepseek.com/api_keys) 页面创建 API key。
3. 在「AI 模型」页 DeepSeek 的设置中填写。

- 推荐模型：`deepseek-flash` 和 `deepseek-v4-pro`。
- 官方文档：[首次调用 API](https://api-docs.deepseek.com/zh-cn/)、[错误码](https://api-docs.deepseek.com/zh-cn/quick_start/error_codes)。

## Kimi {#kimi}

只用于 AI 润色。可切换中国站与国际站，两个站点的账号、余额和密钥不通用。

1. 在 Kimi 开放平台注册：中国站为 [platform.kimi.com](https://platform.kimi.com/)，国际站为 [platform.kimi.ai](https://platform.kimi.ai/)。
2. 推荐的 Kimi K3.1 Pro 模型需要先充值：中国站最低充值 10 元，注册认证赠送的代金券不能用于 K3.1 Pro；国际站最低充值 1 美元。
3. 在 API Keys 页面创建 API Key：[中国站](https://platform.kimi.com/console/api-keys)、[国际站](https://platform.kimi.ai/console/api-keys)。
4. 在 Kimi 的设置中选择注册的站点，填写 API Key。

- 推荐 `kimi-k3.1-pro-highspeed` 等 K3 系列模型。Kimi 的 K2 系列模型自 2026 年 10 月 31 日起不再向新注册用户开放。
- 官方文档：[获取 API Key](https://platform.kimi.com/docs/get-api-key)、[Kimi K3.1 Pro 快速开始](https://platform.kimi.com/docs/guide/kimi-k3-1-pro-quickstart)、[国际站获取 API Key](https://platform.kimi.ai/docs/get-your-api-key)。

## 智谱 GLM {#glm}

用于语音识别和 AI 润色，两项服务共用一把密钥。可切换中国站（智谱开放平台）与国际站（Z.ai），两个站点的账号和密钥不通用。

1. 在[智谱开放平台](https://bigmodel.cn/)或 [Z.ai](https://z.ai/model-api) 注册并登录。
2. 创建标准 API Key：中国站在 [API Keys](https://bigmodel.cn/usercenter/proj-mgmt/apikeys) 页面，国际站在 [API Keys](https://z.ai/manage-apikey/apikey-list) 页面。自 2026 年 10 月 16 日起，标准 API Key 与 GLM Coding Plan 套餐的 Key 分开管理，互不通用。
3. 在智谱 GLM 的设置中选择注册的站点，填写 API Key。

- AI 润色推荐 `glm-5.3-flash`；`glm-4.7-flash` 在两个站点都免费。国际站按量计费时，在 [Billing](https://z.ai/manage-apikey/billing) 中充值。
- 语音识别使用 `glm-asr-2512`，识别整段录音。单次最长 30 秒，录音更长时在停顿处分段识别，每段带上前面识别出的文字。
- 官方文档：[快速开始](https://docs.bigmodel.cn/cn/guide/start/quick-start)、[API Key 分类管理说明](https://docs.bigmodel.cn/cn/coding-plan/notice/api-key-management)、[`glm-asr-2512` 模型](https://docs.bigmodel.cn/cn/guide/models/sound-and-video/glm-asr-2512)、[Z.ai 快速开始](https://docs.z.ai/guides/overview/quick-start)。

## 火山方舟 {#volcengine}

只用于 AI 润色，提供豆包等模型。豆包语音的凭据在这里不能使用。

1. 在火山方舟控制台的[开通管理](https://ark.volcengine.com/region:cn-beijing/openManagement)中，为要使用的模型选择「开通服务」。模型需要先开通，否则请求会被拒绝。
2. 在 [API Key 管理](https://ark.volcengine.com/region:cn-beijing/apikey)中选择「创建 API Key」。
3. 在「AI 模型」页火山方舟的设置中填写。

- 推荐豆包 Seed 系列模型。
- 官方文档：[快速入门](https://docs.volcengine.com/docs/ark/quick-start?lang=zh)、[获取 API Key](https://docs.volcengine.com/docs/ark/api-key?lang=zh)、[开通管理](https://docs.volcengine.com/docs/ark/activation-management?lang=zh)。

## 腾讯混元 {#hunyuan}

只用于 AI 润色，通过腾讯云的大模型服务平台 TokenHub 使用；TokenHub 还提供其他厂商的模型。腾讯云语音识别的凭据在这里不能使用。

1. 完成腾讯云实名认证后，登录 [TokenHub 控制台](https://console.cloud.tencent.com/tokenhub)，按提示开通服务。
2. 在[启用管理](https://console.cloud.tencent.com/tokenhub/open-management)中启用要使用的模型。语言模型有新人免费体验额度；用完后，或模型没有免费额度时，需要开启按量付费。
3. 在 [API Key 管理](https://console.cloud.tencent.com/tokenhub/apikey)页面上方选择地域「广州」，再选择「创建 API Key」。
4. 在「AI 模型」页腾讯混元的设置中填写。

- 推荐 `hy3` 和 `hy4-preview`。使用新加坡站时，把它的地址 `https://tokenhub-intl.tencentmaas.com/v1` 填到接口地址中，并使用新加坡站创建的密钥；两个站点的服务不能交叉调用。
- 官方文档：[快速入门](https://cloud.tencent.com/document/product/1823/130058)、[启用管理](https://cloud.tencent.com/document/product/1823/137795)、[新人免费体验包](https://cloud.tencent.com/document/product/1823/130053)、[API Key 管理](https://cloud.tencent.com/document/product/1823/130090)。

## 小米 MiMo {#mimo}

用于语音识别和 AI 润色，两项服务共用一把密钥。

1. 用小米账号登录 [Xiaomi MiMo 开放平台](https://platform.xiaomimimo.com/)。中国大陆的账号需要先完成个人或企业实名认证，才能充值或购买套餐。
2. 在[账户余额](https://platform.xiaomimimo.com/#/console/balance)中充值。余额不足时，请求会被拒绝。
3. 在 [API Keys](https://platform.xiaomimimo.com/#/console/api-keys) 中创建 API Key。
4. 在小米 MiMo 的设置中填写。

- AI 润色推荐 `mimo-v2.6-flash` 和 `mimo-v2.6-pro`。使用 Token Plan 套餐时，把套餐的专属地址填到接口地址中，并使用套餐的密钥。
- 语音识别使用 `mimo-v2.5-asr`，识别整段录音，录音超过 3 分钟时在停顿处分段识别。
- 官方文档：[首次调用 API](https://mimo.mi.com/docs/zh-CN/quick-start/summary/first-api-call)、[支付与充值](https://mimo.mi.com/docs/zh-CN/quick-start/faq/payment)、[语音识别](https://mimo.mi.com/docs/zh-CN/quick-start/usage-guide/audio/Speech-Recognition)。

## OpenRouter {#openrouter}

只用于 AI 润色，用一把密钥使用多家厂商的模型。

1. 注册 [OpenRouter](https://openrouter.ai/) 账号，在 [Credits](https://openrouter.ai/settings/credits) 页面充值。免费模型每天可请求 50 次，购买至少 10 个 credits 后每天可请求 1000 次。
2. 在 [Keys](https://openrouter.ai/settings/keys) 页面创建密钥，可以为它设置额度上限。
3. 在「AI 模型」页 OpenRouter 的设置中填写。

- 推荐响应快、价格低的模型，例如 `google/gemini-3.5-flash-lite`。
- 官方文档：[快速入门](https://openrouter.ai/docs/quickstart)、[常见问题](https://openrouter.ai/docs/faq)。

## Ollama {#ollama}

在这台电脑上运行的 AI 润色，不需要密钥，文字不离开这台电脑。

1. 从 [ollama.com](https://ollama.com/download) 下载并安装 Ollama，保持它运行。在 Linux 上，如果 Ollama 没有在后台运行，在终端运行 `ollama serve`。
2. 在终端运行 `ollama pull <模型名称>` 下载模型，可用的模型见 [Ollama 模型库](https://ollama.com/library)。
3. 在「AI 模型」页 Ollama 的设置中，接口地址留空即使用 `http://127.0.0.1:11434/v1`，然后在模型列表中选择已下载的模型。

- 官方文档：[快速开始](https://docs.ollama.com/quickstart)、[OpenAI 兼容接口](https://docs.ollama.com/api/openai-compatibility)。

## 自定义接口 {#custom}

任意 OpenAI 兼容接口，例如 vLLM、LocalAI 或自建网关，用于语音识别或 AI 润色。识别和润色的接口地址与密钥分别设置，可以是两台不同的服务器。

1. 在「接口地址」中填写接口的地址，通常以 `/v1` 结尾，例如 `http://127.0.0.1:8000/v1`。
2. 接口需要密钥时填写密钥，然后填写模型 ID，或在测试连接后从列表中选择。
3. 选择「测试连接」，确认地址和密钥可用。

在自己的 GPU 服务器上部署识别服务，见[自建识别服务](/zh/recognition/self-host)。填写阿里云百炼的地址时，按[阿里云百炼](#aliyun)的规则识别。

### Responses 接口

自定义接口的 AI 润色可以在「接口类型」中选择 Chat Completions（默认）或 Responses。OpenAI 的推理模型和一些网关（例如 kiro-provider）支持 Responses 接口。

- 选择 Responses 后，可以设置「推理强度」（minimal、low、medium、high、xhigh）；选择「不设置」时使用服务的默认值。
- 请求不带温度和输出长度上限：推理模型和这些网关通常不接受这两项。请求会要求服务不保存这次的内容。
- 例如使用本机的 kiro-provider：接口地址 `http://127.0.0.1:8787/v1`，模型 `claude-opus-5-5`，接口类型 Responses，推理强度 high。推理强度越高，润色越慢；一次润色超过 30 秒时，Voltip 插入未经润色的识别结果。
