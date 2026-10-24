# Self-hosted recognition

One command deploys speech recognition on a Linux server with an NVIDIA GPU: vLLM serving
Qwen3-ASR, a gateway that checks access tokens and limits requests, and Caddy for HTTPS. Voltip
then uses it as the **Custom endpoint** recognition provider.

```bash
curl -fsSL https://raw.githubusercontent.com/sunerpy/voltip/main/deploy/selfhost/install.sh | sudo sh -s -- --domain asr.example.com --install-deps
```

The guide, with the requirements, the options, measured timings and tuning, is on the website:
[English](https://voltip.firlab.app/recognition/self-host) ·
[中文](https://voltip.firlab.app/zh/recognition/self-host) (`docs/site/recognition/self-host.md`).

| File | Role |
| --- | --- |
| `install.sh` | checks the GPU, installs what is missing (`--install-deps`), writes `/opt/voltip-asr`, prepares the vLLM image and starts the service |
| `compose.yaml` | the three services, all on the host's network: `asr` (vLLM on 127.0.0.1), `gateway` (nginx on 127.0.0.1), `caddy` (the way in) |
| `asr-serve.sh` | the `asr` entrypoint: `vllm serve` with the settings from `.env` |
| `gateway/default.conf.template` | the gateway: tokens, per-address limits, the two endpoints Voltip uses, uploads kept in memory |
| `gateway/40-voltip-tokens.sh` | turns the `tokens` file into the gateway's token map at start |
| `Caddyfile` | HTTPS for a domain name (Let's Encrypt), or plain HTTP on a port |
| `test_gateway.py` | the gateway in its nginx image in front of a stub of vLLM; run by `make verify` (needs Docker) |

The production built-in service is set up differently (`docs/runbook.md`, 自建内置服务); this kit is
for one person's or one team's own server.
