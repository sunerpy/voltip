---
title: Try it online
description: Record or upload up to a minute of speech in your browser and see how Voltip's built-in service recognises and polishes it.
---

# Try it online

Record a few sentences in your browser, or upload an audio file of up to 60 seconds, to see what
Voltip's built-in service makes of it: Qwen3-ASR recognises the speech, then the AI model and the
style you choose polish the text. There is nothing to install; in the app, the same text is
inserted at your cursor.

<TryVoltip />

## Where the recording and the text go

The recording goes through this website to Voltip's built-in service for recognition, and the
text you polish to the built-in service for AI polish; the built-in service does not keep the
recording or the text. Before you start, a Cloudflare Turnstile check confirms that a person is
using the page. After the check, your browser keeps a cookie for 30 minutes; the page keeps
nothing else. See [Privacy](/privacy#try-it-online).

## Limits

The try page shares the built-in service with the app, so it has limits of its own: each network
address can run 20 recognitions and 10 polishes an hour, and all visitors together 300
recognitions and 60 polishes a day. When a limit is reached, the page says when you can try
again. Using the app does not count against these limits.

## In the app

In the app, you hold a shortcut and speak, and the text is inserted at the cursor of any program
when you let go. The app can also recognise with a local model, so the audio never leaves your
computer, and it can use a cloud service and an AI service of your own.
[Install](/guide/install) it, then follow the [quick start](/guide/quick-start).
