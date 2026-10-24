---
description: 在自己的 Linux GPU 服务器上，用一条命令部署 Qwen3-ASR 语音识别服务（vLLM），带访问令牌、请求限制和 HTTPS，在 Voltip 中作为自定义接口使用。
---

# 自建识别服务

本页说明如何在自己的服务器上运行语音识别：一条命令在装有 NVIDIA 显卡的 Linux 服务器上部署 vLLM 和 Qwen3-ASR，前面是检查访问令牌、限制请求次数的网关；有域名时自动申请证书，启用 HTTPS。部署后，在 Voltip 中把它设为「自定义接口」，录音只发送到你的服务器。内置服务使用的也是 vLLM 和 Qwen3-ASR 1.7B。

这项服务只进行语音识别。AI 润色继续使用内置服务、你的服务商，或在本机运行的 Ollama，在「AI 模型」页设置。

## 准备

- **服务器**：Linux x86_64。使用 Ubuntu 或 Debian 时，安装脚本可以安装缺少的软件。已在 Ubuntu 24.04 上测试。
- **显卡**：NVIDIA 显卡，计算能力 7.5 及以上（Turing 及更新的架构），已在 NVIDIA L4 和 T4（驱动 595）上测试。计算能力低于 8.0 的显卡（例如 T4）使用 float16 精度，脚本会自动选择。显存 8 GB 及以上时默认使用 Qwen3-ASR 1.7B，否则使用 0.6B，见[显存](#显存)。
- **驱动**：已安装 NVIDIA 驱动并重启过服务器。安装脚本不安装驱动。在 Ubuntu 上可以执行：

  ```bash
  sudo ubuntu-drivers install --gpgpu && sudo reboot
  ```

- **磁盘**：至少 50 GB 可用空间。vLLM 镜像下载约 9 GB，解压后约占 34 GB；Qwen3-ASR 1.7B 约 4.8 GB。
- **HTTPS**（推荐）：一个解析到这台服务器的域名，并且 80 和 443 端口可以从互联网访问：申请和续期证书需要这两个端口。云服务器还需要在安全组或防火墙中放行它们。

## 部署

以 root 身份运行安装脚本，`--domain` 后填写你的域名：

```bash
curl -fsSL https://raw.githubusercontent.com/sunerpy/voltip/main/deploy/selfhost/install.sh | sudo sh -s -- --domain asr.example.com --install-deps
```

脚本依次完成以下步骤：

1. 检查显卡、驱动和驱动支持的 CUDA 版本，按显卡选择模型和计算精度，按驱动选择 vLLM 镜像（CUDA 13，或驱动较旧时的 CUDA 12.9 版本）。
2. 缺少 `nvidia-smi`、Docker Engine 或 NVIDIA Container Toolkit 时，从 Ubuntu、Docker 和 NVIDIA 的软件源安装它们。不加 `--install-deps` 时，脚本只列出缺少的软件并退出，不改动系统；安装 NVIDIA Container Toolkit 后会重启 Docker。
3. 把服务的文件写入 `/opt/voltip-asr`，生成访问令牌，下载镜像，为 vLLM 镜像加上它缺少的音频解码组件，然后启动服务。第一次启动时 vLLM 下载模型。
4. 等待模型就绪，经 HTTPS 发送一段测试录音，然后显示在 Voltip 中填写的内容。

在 AWS 首尔区域实测（2026-10-24）：g6.xlarge（NVIDIA L4 24 GB）上从只装有驱动的 Ubuntu 24.04 开始，包括安装 Docker 等软件，用时 12 分 34 秒；g4dn.xlarge（NVIDIA T4 16 GB）上已有 Docker 时用时 12 分 20 秒。大部分时间用于下载和解压 vLLM 镜像；vLLM 第一次启动（下载模型、编译和预热）分别用时 160 秒和 197 秒。

没有域名时，改用 `--port`，服务以 HTTP 在这个端口上运行：

```bash
curl -fsSL https://raw.githubusercontent.com/sunerpy/voltip/main/deploy/selfhost/install.sh | sudo sh -s -- --port 8080 --install-deps
```

HTTP 连接不加密，访问令牌和录音都以明文传输，只适合在可信的网络中使用，例如局域网或 VPN。

### 选项

| 选项 | 作用 |
| --- | --- |
| `--domain <域名>` | 使用 HTTPS。证书由 Let's Encrypt 签发，自动续期 |
| `--port <端口>` | 不使用域名时的 HTTP 端口，默认 8080 |
| `--model 1.7b` 或 `--model 0.6b` | 使用的模型。默认按显存选择 |
| `--model-source modelscope` | 从 ModelScope 下载模型，默认从 Hugging Face 下载 |
| `--gpu <编号>` | 使用哪块显卡，编号与 `nvidia-smi -L` 相同，默认 0 |
| `--gpu-memory <比例>` | vLLM 可以使用的显存比例，0.1 至 0.95，默认 0.85 |
| `--install-deps` | 安装缺少的 `nvidia-smi`、Docker Engine 和 NVIDIA Container Toolkit（Ubuntu 和 Debian） |
| `--dir <目录>` | 安装目录，默认 `/opt/voltip-asr` |

再次运行安装脚本即可升级。访问令牌和设置会保留；再次指定的选项替换原来的设置。

## 在 Voltip 中使用

1. 在「语音模型」页点「自定义接口」卡片。
2. 填写安装脚本最后显示的内容：「接口地址」为 `https://asr.example.com/v1`，「API 密钥」为访问令牌，「模型」为 `Qwen/Qwen3-ASR-1.7B`。保存密钥后，也可以从模型列表中选择。
3. 选择「测试连接」，然后选择「设为当前」。

- **热词**：[词典](/zh/recognition/dictionary)中的正确写法会作为热词发送，Qwen3-ASR 会优先识别为这些写法。例如一段说到「陈诗瞳」和「林小雁」的录音，没有词典时识别为「陈思彤」「林小燕」，把这两个名字加入词典后识别为正确的写法（2026-10-24，Qwen3-ASR 1.7B 实测）。热词提高这些写法的优先级，但不能保证每次都识别正确；仍然识别错的写法，可以填写为词条的「曾听成」写法，识别后会替换为正确写法。
- **实时预览**：说话过程中的实时预览由本地的实时识别模型提供，需要先在「语音模型」页下载（169 MB，见[本地识别](/zh/recognition/local)）；最终文字仍由自建服务识别。
- **上传的数据量**：录音以 WAV 格式上传，每分钟约 1.9 MB。

## 访问令牌

令牌保存在 `/opt/voltip-asr/tokens` 中，只有 root 可以读取，每行一个。可以为每个人或每台设备单独添加令牌，以便单独停用：

```bash
cd /opt/voltip-asr
openssl rand -hex 24 | sudo tee -a tokens    # 添加一个令牌，并显示它
sudo docker compose restart gateway          # 修改后重启网关，立即生效
```

令牌至少 16 个字符，只能包含字母、数字和 `.`、`_`、`~`、`+`、`/`、`=`、`-`；`#` 之后的内容是注释。删除一行并重启网关后，这个令牌立即失效。网关的日志用令牌所在的行号（例如 `t4`）表示是哪个令牌，不记录令牌本身。

## 网关

请求先经过 Caddy（HTTPS），再经过网关，最后到达 vLLM；vLLM 和网关只监听本机地址，不能从其他电脑直接访问。网关：

- 只接受带有效令牌的请求，缺少令牌或令牌无效时返回 `401`；
- 只开放 `POST /v1/audio/transcriptions` 和 `GET /v1/models`，其他路径返回 `404`；`/healthz` 不需要令牌，vLLM 就绪时返回 `200`；
- 按客户端地址限制请求：每秒 3 个，可以连续发送 30 个，同时最多 8 个连接；超过时返回 `429` 和 `Retry-After`；
- 单个请求最大 32 MB（约 17 分钟的 WAV 录音），超过时返回 `413`；识别超过 120 秒时返回 `504`；
- 只在内存中处理上传的录音，不写入磁盘；日志每个请求一行，记录时间、客户端地址、令牌行号、路径、状态和用时，不记录录音、文字和令牌。日志由 Docker 保存，最多 5 个文件，每个 10 MB。

vLLM 和 Hugging Face 的使用统计默认关闭。

## 性能与调优

在 AWS 首尔区域的 g6.xlarge（NVIDIA L4 24 GB）和 g4dn.xlarge（NVIDIA T4 16 GB）上，使用默认设置，识别一段录音的用时（秒；2026-10-24 实测，在服务器本机发送）：

| 录音 | L4 · 1.7B | L4 · 0.6B | T4 · 1.7B |
| --- | --- | --- | --- |
| 5.3 秒 | 0.37 | 0.15 | 0.41 |
| 7.3 秒 | 0.50 | 0.20 | 0.53 |
| 14.3 秒 | 0.73 | 0.30 | 0.79 |

同时发送多段 14.3 秒的录音时，全部完成的用时（秒）：

| 同时发送 | L4 · 1.7B | L4 · 0.6B | T4 · 1.7B |
| --- | --- | --- | --- |
| 8 段 | 0.85 | 0.38 | 1.16 |
| 16 段 | 1.63 | 0.71 | 2.28 |

同时发送的录音直接发给 vLLM，没有经过网关：网关对每个地址最多允许 8 个连接。从其他电脑使用时，还需要加上上传录音的时间。0.6B 更快，但在这几段录音中，人名和英文词的错误更多。

### 显存

vLLM 启动时按 `--gpu-memory` 的比例占用显存，其中模型之外的部分用于同时识别多段录音。占用的显存不随请求多少而变化。在 NVIDIA L4 24 GB 上（2026-10-24 实测）：

| 模型 | `--gpu-memory` | 占用显存 | 可同时识别 |
| --- | --- | --- | --- |
| 1.7B | 0.85 | 18.3 GB | 约 120 分钟 |
| 1.7B | 0.40 | 8.1 GB | 约 33 分钟 |
| 1.7B | 0.30 | 5.9 GB | 约 13 分钟 |
| 0.6B | 0.85 | 18.3 GB | 约 145 分钟 |
| 0.6B | 0.25 | 4.7 GB | 约 24 分钟 |
| 0.6B | 0.20 | 3.6 GB | 约 14 分钟 |

「可同时识别」是同一时刻可以识别的录音总长，按实测用量估算：一段 14.3 秒的录音及其文字共占 245 个 token。每种设置都能正常启动和识别，表中最低的设置识别单段录音的速度与默认设置相同。显存小于 8 GB 的显卡默认使用 0.6B。

### 设置

设置保存在 `/opt/voltip-asr/.env` 中。修改后，在该目录中执行 `sudo docker compose up -d`：

| 设置 | 默认值 | 作用 |
| --- | --- | --- |
| `VOLTIP_GPU_MEMORY_UTILIZATION` | `0.85` | vLLM 可以使用的显存比例。这块显卡还运行其他程序时调低 |
| `VOLTIP_MAX_NUM_SEQS` | `8` | 同时识别的请求数。超过时请求排队等待；调高可以提高吞吐，但占用更多显存 |
| `VOLTIP_MAX_MODEL_LEN` | `8192` | 单个请求的最大长度（音频和文字的 token 数），限制了单段录音的长度 |
| `VOLTIP_VLLM_ARGS` | 空 | 其他 vLLM 选项，用空格分隔，例如 `--enforce-eager`（启动更快，识别较慢） |
| `VOLTIP_RATE`、`VOLTIP_BURST`、`VOLTIP_CONNECTIONS` | `3`、`30`、`8` | 网关对每个客户端地址的限制：每秒请求数、可以连续发送的请求数、同时连接数 |
| `VOLTIP_MAX_UPLOAD_MB`、`VOLTIP_TIMEOUT` | `32`、`120` | 单个请求的最大大小（MB）和识别的最长时间（秒） |

## 管理

在 `/opt/voltip-asr` 中执行：

```bash
sudo docker compose ps                  # 状态
sudo docker compose logs -f asr gateway  # 日志
sudo docker compose restart             # 重启
sudo docker compose down                # 停止
```

服务器重启后，服务会自动启动。卸载时，先在该目录中执行 `sudo docker compose down --volumes --rmi all`，删除容器、镜像、下载的模型和证书，再删除 `/opt/voltip-asr`。

## 问题排查

- **HTTPS 无法使用**：确认域名解析到这台服务器，且 80 和 443 端口可以从互联网访问（云服务器的安全组、`ufw` 等防火墙）。证书申请的过程见 `sudo docker compose logs caddy`。
- **vLLM 无法启动**：查看 `sudo docker compose logs asr`。显存不足时，改用 0.6B（`--model 0.6b`），或在这块显卡还运行其他程序时调低 `--gpu-memory`。
- **测试连接返回 401**：「API 密钥」不是 `tokens` 文件中的令牌，或修改 `tokens` 后没有重启网关。
- **在中国大陆的服务器上**：Docker Hub 和 Hugging Face 的下载可能很慢或无法连接。可以用 `--model-source modelscope` 从 ModelScope 下载模型；Docker 镜像可以在 Docker 中配置镜像加速后下载。
