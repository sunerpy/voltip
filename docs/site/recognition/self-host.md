---
description: Deploy Qwen3-ASR speech recognition (vLLM) on your own Linux GPU server with one command, with access tokens, request limits and HTTPS, and use it in Voltip as a custom endpoint.
---

# Self-hosted recognition

This page shows how to run speech recognition on your own server: one command deploys vLLM and Qwen3-ASR on a Linux server with an NVIDIA graphics card, behind a gateway that checks access tokens and limits requests; with a domain name, it obtains a certificate and serves HTTPS. Then set it up in Voltip as the **Custom endpoint**, and recordings go to your server only. The built-in service runs vLLM and Qwen3-ASR 1.7B as well.

The service does speech recognition only. AI polish keeps using the built-in service, your provider, or Ollama on this computer, set on the **AI models** page.

## Requirements

- **Server**: Linux x86_64. On Ubuntu or Debian the install script can install what is missing. Tested on Ubuntu 24.04.
- **Graphics card**: NVIDIA, compute capability 7.5 or later (Turing or a newer architecture), tested on an NVIDIA L4 and a T4. A card below compute capability 8.0, such as the T4, runs at float16 precision, which the script picks. With 8 GB of memory or more, Qwen3-ASR 1.7B is the default, otherwise 0.6B; see [GPU memory](#gpu-memory).
- **Driver**: an NVIDIA driver, installed, with the server restarted since. The install script does not install drivers. On Ubuntu:

  ```bash
  sudo ubuntu-drivers install --gpgpu && sudo reboot
  ```

- **Disk**: at least 50 GB free. The vLLM image is a download of about 9 GB that takes about 34 GB once unpacked; Qwen3-ASR 1.7B is about 4.8 GB.
- **HTTPS** (recommended): a domain name that resolves to the server, with ports 80 and 443 reachable from the internet, which getting and renewing the certificate needs. On a cloud server, also open both in its security group or firewall.

## Deploying

Run the install script as root, with your domain name after `--domain`:

```bash
curl -fsSL https://raw.githubusercontent.com/sunerpy/voltip/main/deploy/selfhost/install.sh | sudo sh -s -- --domain asr.example.com --install-deps
```

The script then:

1. Checks the graphics card, the driver and the CUDA version the driver supports, picks the model and the precision for the card and the vLLM image for the driver (CUDA 13, or the CUDA 12.9 version for an older driver).
2. Installs `nvidia-smi`, Docker Engine and the NVIDIA Container Toolkit when they are missing, from Ubuntu's, Docker's and NVIDIA's repositories. Without `--install-deps` it names what is missing and stops without changing the system. Installing the NVIDIA Container Toolkit restarts Docker.
3. Writes the service's files to `/opt/voltip-asr`, creates an access token, downloads the images, adds the audio decoders the vLLM image lacks, and starts the service. On its first start, vLLM downloads the model.
4. Waits for the model, sends a test recording over HTTPS, and shows what to enter in Voltip.

On an AWS g6.xlarge in the Seoul region (NVIDIA L4, 24 GB), starting from Ubuntu 24.04 with only the driver installed, the whole run took 12 min 34 s, of which vLLM's first start (downloading the model, compiling and warming up) took 160 s (measured 2026-10-24).

Without a domain name, use `--port` instead, and the service runs over HTTP on that port:

```bash
curl -fsSL https://raw.githubusercontent.com/sunerpy/voltip/main/deploy/selfhost/install.sh | sudo sh -s -- --port 8080 --install-deps
```

HTTP is not encrypted: the access token and the recordings travel in plain text. Use it only on a network you trust, such as a local network or a VPN.

### Options

| Option | Effect |
| --- | --- |
| `--domain <name>` | Serve HTTPS. Let's Encrypt issues the certificate, which renews on its own |
| `--port <port>` | The HTTP port without a domain name, 8080 by default |
| `--model 1.7b` or `--model 0.6b` | The model. By default chosen by the GPU's memory |
| `--model-source modelscope` | Download the model from ModelScope instead of Hugging Face |
| `--gpu <number>` | Which graphics card, numbered as `nvidia-smi -L` lists them, 0 by default |
| `--gpu-memory <share>` | The share of GPU memory vLLM may use, 0.1 to 0.95, 0.85 by default |
| `--install-deps` | Install a missing `nvidia-smi`, Docker Engine and NVIDIA Container Toolkit (Ubuntu and Debian) |
| `--dir <directory>` | Where the service lives, `/opt/voltip-asr` by default |

To upgrade, run the install script again. The access tokens and the settings stay; an option given again replaces the stored setting.

## Using it in Voltip

1. On the **Speech models** page, click the **Custom endpoint** card.
2. Enter what the install script showed at the end: the **Endpoint** `https://asr.example.com/v1`, the access token as the **API key**, and `Qwen/Qwen3-ASR-1.7B` as the **Model**. Once the key is saved, the model can be picked from the list as well.
3. Choose **Test connection**, then **Set as current**.

- **Hotwords**: the right spellings in the [dictionary](/recognition/dictionary) are sent as hotwords, and Qwen3-ASR prefers them. In a recording that names "陈诗瞳" and "林小雁", for example, recognition without the dictionary wrote "陈思彤" and "林小燕"; with both names in the dictionary, it wrote them correctly (measured 2026-10-24 with Qwen3-ASR 1.7B). Hotwords make those spellings more likely, not certain; a spelling that still comes out wrong can go into the entry as a **heard as** form, which is replaced with the right spelling after recognition.
- **Live preview**: the preview while you speak comes from the on-device realtime model, which has to be downloaded on the **Speech models** page first (169 MB, see [Local recognition](/recognition/local)); the final text still comes from your server.
- **Upload size**: recordings are uploaded as WAV, about 1.9 MB a minute.

## Access tokens

The tokens are in `/opt/voltip-asr/tokens`, one a line, readable by root only. Give each person or device a token of its own to be able to revoke it alone:

```bash
cd /opt/voltip-asr
openssl rand -hex 24 | sudo tee -a tokens    # add a token and show it
sudo docker compose restart gateway          # after a change: restart the gateway, effective at once
```

A token has at least 16 characters, letters, digits and `.`, `_`, `~`, `+`, `/`, `=`, `-` only; anything after `#` is a comment. After a line is deleted and the gateway restarted, that token stops working at once. The gateway's log names a token by its line number (`t4`, for example), never by its value.

## The gateway

A request passes Caddy (HTTPS), then the gateway, then reaches vLLM; vLLM and the gateway listen on the server's own loopback address, out of reach from other computers. The gateway:

- accepts only requests with a valid token; a missing or unknown token gets `401`;
- passes on only `POST /v1/audio/transcriptions` and `GET /v1/models`, and answers `404` for any other path; `/healthz` needs no token and answers `200` once vLLM is ready;
- limits each client address: 3 requests a second, 30 in a row, 8 connections at a time; past that it answers `429` with `Retry-After`;
- takes requests of up to 32 MB (about 17 minutes of WAV), answering `413` above that, and answers `504` when recognition takes longer than 120 seconds;
- keeps uploaded recordings in memory, never on disk, and logs one line per request with the time, the client's address, the token's line, the path, the status and the duration, never the recording, the text or the token. Docker keeps the log in at most 5 files of 10 MB.

vLLM's and Hugging Face's usage statistics are turned off.

## Performance and tuning

On an AWS g6.xlarge (NVIDIA L4, 24 GB) and a g4dn.xlarge (NVIDIA T4, 16 GB) in the Seoul region, with the default settings (measured 2026-10-24, sent on the server itself):

| Recording | L4 · 1.7B | L4 · 0.6B | T4 · 1.7B |
| --- | --- | --- | --- |
| 5.3 s | 0.37 s | 0.15 s | 0.41 s |
| 7.3 s | 0.50 s | 0.20 s | 0.53 s |
| 14.3 s | 0.73 s | 0.30 s | 0.79 s |
| 8 recordings of 14.3 s sent at once, all done | 0.85 s | 0.38 s | 1.16 s |
| 16 recordings of 14.3 s sent at once, all done | 1.63 s | 0.71 s | 2.28 s |

The recordings sent at once went to vLLM directly, not through the gateway, which allows each address 8 connections. From another computer, add the time it takes to upload the recording. 0.6B is faster, but in these recordings it got more names and English words wrong.

### GPU memory

vLLM takes the share of GPU memory set by `--gpu-memory` when it starts; what the model does not need goes to recognising several recordings at once. The memory taken does not change with the number of requests. On an NVIDIA L4 with 24 GB (measured 2026-10-24):

| Model | `--gpu-memory` | Memory taken | Recordings recognised at once, in total |
| --- | --- | --- | --- |
| 1.7B | 0.85 | 18.3 GB | about 120 minutes |
| 1.7B | 0.40 | 8.1 GB | about 33 minutes |
| 1.7B | 0.30 | 5.9 GB | about 13 minutes |
| 0.6B | 0.85 | 18.3 GB | about 145 minutes |
| 0.6B | 0.25 | 4.7 GB | about 24 minutes |
| 0.6B | 0.20 | 3.6 GB | about 14 minutes |

Every setting started and recognised normally; the lowest recognised a single recording as fast as the default. The total length is estimated from what was measured: a recording of 14.3 seconds and its text took 245 tokens. A card with less than 8 GB uses 0.6B by default.

### Settings

The settings are in `/opt/voltip-asr/.env`. After a change, run `sudo docker compose up -d` in that directory:

| Setting | Default | Effect |
| --- | --- | --- |
| `VOLTIP_GPU_MEMORY_UTILIZATION` | `0.85` | The share of GPU memory vLLM may use. Lower it when other programs use the same card |
| `VOLTIP_MAX_NUM_SEQS` | `8` | Requests recognised at once. More wait in a queue; raising it raises throughput and takes more memory |
| `VOLTIP_MAX_MODEL_LEN` | `8192` | The longest request (audio and text, in tokens), which limits the length of one recording |
| `VOLTIP_VLLM_ARGS` | empty | More vLLM options, separated by spaces, such as `--enforce-eager` (starts faster, recognises more slowly) |
| `VOLTIP_RATE`, `VOLTIP_BURST`, `VOLTIP_CONNECTIONS` | `3`, `30`, `8` | The gateway's limits for each client address: requests a second, requests in a row, connections at a time |
| `VOLTIP_MAX_UPLOAD_MB`, `VOLTIP_TIMEOUT` | `32`, `120` | The largest request (MB) and the longest recognition (seconds) |

## Managing the service

In `/opt/voltip-asr`:

```bash
sudo docker compose ps                  # status
sudo docker compose logs -f asr gateway  # logs
sudo docker compose restart             # restart
sudo docker compose down                # stop
```

The service starts again when the server restarts. To uninstall, run `sudo docker compose down --volumes --rmi all` in that directory, which removes the containers, the downloaded model and the images, then delete `/opt/voltip-asr`.

## Troubleshooting

- **HTTPS does not work**: check that the domain name resolves to the server and that ports 80 and 443 are reachable from the internet (the cloud server's security group, a firewall such as `ufw`). `sudo docker compose logs caddy` shows how getting the certificate went.
- **vLLM does not start**: see `sudo docker compose logs asr`. Without enough GPU memory, switch to 0.6B (`--model 0.6b`), or lower `--gpu-memory` when other programs use the same card.
- **Test connection answers 401**: the **API key** is not a token from the `tokens` file, or the gateway was not restarted after `tokens` changed.
- **On a server in mainland China**: downloads from Docker Hub and Hugging Face can be slow or fail. `--model-source modelscope` downloads the model from ModelScope; for the Docker images, set up a registry mirror in Docker first.
