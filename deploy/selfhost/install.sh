#!/bin/sh
# One-command install of Voltip's self-hosted recognition service on a Linux x86_64 server with an
# NVIDIA GPU: vLLM serving Qwen3-ASR, a gateway that checks access tokens and limits requests, and
# Caddy for HTTPS (deploy/selfhost/compose.yaml). Run it as root:
#
#   curl -fsSL https://raw.githubusercontent.com/sunerpy/voltip/main/deploy/selfhost/install.sh | sudo sh -s -- --domain asr.example.com
#
#   --domain NAME        HTTPS at NAME, which must resolve to this server. Caddy obtains and renews
#                        the certificate from Let's Encrypt, so ports 80 and 443 must be reachable.
#   --port N             without --domain: plain HTTP on port N (default 8080). Unencrypted; for a
#                        network you trust only.
#   --model 1.7b|0.6b    Qwen3-ASR-1.7B or Qwen3-ASR-0.6B (default: 1.7b on a GPU with at least
#                        8 GB of memory, else 0.6b)
#   --model-source S     where vLLM downloads the model: huggingface (default) or modelscope
#   --gpu N              the GPU to use, numbered as nvidia-smi does (default 0)
#   --gpu-memory F       the share of the GPU's memory vLLM may take, 0.1 to 0.95 (default 0.85)
#   --install-deps       install what is missing, on Ubuntu or Debian: the driver's nvidia-smi,
#                        Docker Engine and the NVIDIA Container Toolkit (which restarts Docker)
#   --dir DIR            where the service lives (default /opt/voltip-asr)
#   --ref REF            take the service's files from this branch or tag of sunerpy/voltip
#                        (default main)
#
# It never installs a GPU driver. Running it again upgrades the service and keeps the access tokens
# (DIR/tokens) and the settings (DIR/.env); an option given again replaces the stored value.
set -eu

REPO=sunerpy/voltip
VLLM_VERSION=v0.32.0
FILES="compose.yaml asr-serve.sh Caddyfile gateway/default.conf.template gateway/40-voltip-tokens.sh"
# 1.7B by default from this much GPU memory (MiB): it started and recognised within a 6.9 GB share on
# an L4 (2026-10-24), and 8 GB cards report a little under 8192 MiB.
LARGE_MODEL_MIB=7500
# The image compose.yaml runs: $vllm_image with vLLM's audio extra.
LOCAL_IMAGE=voltip-asr-vllm:local

err() {
	printf 'voltip-asr: %s\n' "$1" >&2
	exit 1
}

info() {
	printf 'voltip-asr: %s\n' "$1" >&2
}

usage() {
	cat <<'EOF'
Usage: install.sh [--domain NAME | --port N] [--model 1.7b|0.6b] [--model-source huggingface|modelscope]
                  [--gpu N] [--gpu-memory F] [--install-deps] [--dir DIR] [--ref REF]
Installs Voltip's self-hosted recognition service (vLLM with Qwen3-ASR, an access-token gateway and
Caddy for HTTPS). See https://voltip.firlab.app/recognition/self-host
EOF
}

dir=/opt/voltip-asr
ref=main
opt_domain=
opt_port=
opt_model=
opt_source=
opt_gpu=
opt_gpu_memory=
install_deps=0

need_value() {
	[ "$#" -ge 2 ] && [ -n "$2" ] || err "$1 needs a value"
}

while [ "$#" -gt 0 ]; do
	case "$1" in
	--domain) need_value "$@"; opt_domain=$2; shift 2 ;;
	--domain=*) opt_domain=${1#*=}; shift ;;
	--port) need_value "$@"; opt_port=$2; shift 2 ;;
	--port=*) opt_port=${1#*=}; shift ;;
	--model) need_value "$@"; opt_model=$2; shift 2 ;;
	--model=*) opt_model=${1#*=}; shift ;;
	--model-source) need_value "$@"; opt_source=$2; shift 2 ;;
	--model-source=*) opt_source=${1#*=}; shift ;;
	--gpu) need_value "$@"; opt_gpu=$2; shift 2 ;;
	--gpu=*) opt_gpu=${1#*=}; shift ;;
	--gpu-memory) need_value "$@"; opt_gpu_memory=$2; shift 2 ;;
	--gpu-memory=*) opt_gpu_memory=${1#*=}; shift ;;
	--install-deps) install_deps=1; shift ;;
	--dir) need_value "$@"; dir=$2; shift 2 ;;
	--dir=*) dir=${1#*=}; shift ;;
	--ref) need_value "$@"; ref=$2; shift 2 ;;
	--ref=*) ref=${1#*=}; shift ;;
	-h | --help) usage; exit 0 ;;
	*) usage >&2; err "unknown option: $1" ;;
	esac
done

is_uint() {
	case "$1" in '' | *[!0-9]*) return 1 ;; esac
}

# The options' own checks, before anything changes on this server.
if [ -n "$opt_domain" ]; then
	[ -z "$opt_port" ] || err "--port is for plain HTTP without a domain; with --domain the service uses ports 80 and 443"
	case "$opt_domain" in
	*[!A-Za-z0-9.-]* | .* | *. | *..* | *.-* | *-.*) err "--domain $opt_domain is not a domain name" ;;
	*.*) ;;
	*) err "--domain $opt_domain is not a domain name" ;;
	esac
fi
if [ -n "$opt_port" ]; then
	is_uint "$opt_port" && [ "$opt_port" -ge 1 ] && [ "$opt_port" -le 65535 ] || err "--port must be a number from 1 to 65535"
fi
case "$opt_model" in
'') ;;
1.7b | 1.7B | Qwen/Qwen3-ASR-1.7B) opt_model=Qwen/Qwen3-ASR-1.7B ;;
0.6b | 0.6B | Qwen/Qwen3-ASR-0.6B) opt_model=Qwen/Qwen3-ASR-0.6B ;;
*) err "--model must be 1.7b or 0.6b" ;;
esac
case "$opt_source" in
'' | huggingface | modelscope) ;;
*) err "--model-source must be huggingface or modelscope" ;;
esac
if [ -n "$opt_gpu" ]; then
	is_uint "$opt_gpu" || err "--gpu must be a GPU's number, as nvidia-smi -L lists them"
fi
if [ -n "$opt_gpu_memory" ]; then
	awk -v f="$opt_gpu_memory" 'BEGIN { exit !(f ~ /^0?\.[0-9]+$/ && f >= 0.1 && f <= 0.95) }' ||
		err "--gpu-memory must be a share from 0.1 to 0.95, such as 0.6"
fi
case "$dir" in /*) ;; *) err "--dir must be an absolute path" ;; esac

[ "$(id -u)" = 0 ] || err "run it as root, e.g. curl -fsSL <this script's address> | sudo sh -s -- <options>"
[ "$(uname -s)" = Linux ] && [ "$(uname -m)" = x86_64 ] || err "this service runs on Linux x86_64 only"
command -v curl >/dev/null 2>&1 || err "curl is required"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# --install-deps: apt on Ubuntu or Debian, the index refreshed once.
apt_updated=
apt_ready() {
	command -v apt-get >/dev/null 2>&1 || err "--install-deps installs with apt (Ubuntu or Debian); install the missing part yourself"
	# shellcheck source=/dev/null
	os_id=$(. /etc/os-release && printf '%s' "${ID:-}")
	case "$os_id" in
	ubuntu | debian) ;;
	*) err "--install-deps supports Ubuntu and Debian, not ${os_id:-this system}; install the missing part yourself" ;;
	esac
	if [ -z "$apt_updated" ]; then
		apt-get update -qq
		apt_updated=1
	fi
}

apt_install() {
	DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends "$@"
}

installed() {
	dpkg-query -W -f='${Status}' "$1" 2>/dev/null | grep -q 'install ok installed'
}

# nvidia-smi, which reports the driver and the GPU. A driver installed for computing only (Ubuntu's
# `ubuntu-drivers install --gpgpu`) comes without it.
ensure_nvidia_smi() {
	command -v nvidia-smi >/dev/null 2>&1 && return 0
	[ -r /proc/driver/nvidia/version ] ||
		err "no NVIDIA driver is loaded. Install one and restart the server, then run this again; on Ubuntu: sudo ubuntu-drivers install --gpgpu && sudo reboot"
	major=$(sed -n 's/.*  \([0-9][0-9]*\)\.[0-9.]*  .*/\1/p' /proc/driver/nvidia/version | head -n 1)
	[ -n "$major" ] || err "nvidia-smi is missing; install the NVIDIA utilities that match the loaded driver"
	if installed "libnvidia-compute-$major-server"; then pkg=nvidia-utils-$major-server; else pkg=nvidia-utils-$major; fi
	[ "$install_deps" = 1 ] ||
		err "the NVIDIA driver $major is loaded but nvidia-smi is missing: run this again with --install-deps, or install $pkg"
	apt_ready
	info "installing $pkg"
	apt_install "$pkg"
	command -v nvidia-smi >/dev/null 2>&1 || err "$pkg did not provide nvidia-smi"
}

install_docker() {
	apt_ready
	info "installing Docker Engine from Docker's apt repository"
	apt_install ca-certificates curl
	install -m 0755 -d /etc/apt/keyrings
	curl -fsSL "https://download.docker.com/linux/$os_id/gpg" -o /etc/apt/keyrings/docker.asc
	chmod a+r /etc/apt/keyrings/docker.asc
	# shellcheck source=/dev/null
	codename=$(. /etc/os-release && printf '%s' "${UBUNTU_CODENAME:-${VERSION_CODENAME:-}}")
	printf 'deb [arch=%s signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/%s %s stable\n' \
		"$(dpkg --print-architecture)" "$os_id" "$codename" >/etc/apt/sources.list.d/docker.list
	apt-get update -qq
	apt_install docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
	systemctl enable --now docker >/dev/null 2>&1 || true
}

ensure_docker() {
	if ! command -v docker >/dev/null 2>&1; then
		[ "$install_deps" = 1 ] ||
			err "Docker Engine is not installed: run this again with --install-deps, or install it yourself (https://docs.docker.com/engine/install/)"
		install_docker
	fi
	if ! docker compose version >/dev/null 2>&1; then
		[ "$install_deps" = 1 ] ||
			err "the Docker Compose plugin is missing: run this again with --install-deps, or install docker-compose-plugin"
		apt_ready
		info "installing the Docker Compose plugin"
		apt_install docker-compose-plugin || apt_install docker-compose-v2
	fi
	if ! docker info >/dev/null 2>&1; then
		systemctl start docker >/dev/null 2>&1 || true
		docker info >/dev/null 2>&1 || err "the Docker daemon is not running (systemctl status docker)"
	fi
}

ensure_toolkit() {
	if command -v nvidia-container-runtime-hook >/dev/null 2>&1 || command -v nvidia-ctk >/dev/null 2>&1; then
		return 0
	fi
	[ "$install_deps" = 1 ] ||
		err "the NVIDIA Container Toolkit is missing: run this again with --install-deps, or install it yourself (https://docs.nvidia.com/datacenter/cloud-native/container-toolkit/latest/install-guide.html)"
	apt_ready
	info "installing the NVIDIA Container Toolkit from NVIDIA's apt repository"
	apt_install ca-certificates curl gnupg
	curl -fsSL https://nvidia.github.io/libnvidia-container/gpgkey |
		gpg --dearmor --yes -o /usr/share/keyrings/nvidia-container-toolkit-keyring.gpg
	curl -fsSL https://nvidia.github.io/libnvidia-container/stable/deb/nvidia-container-toolkit.list |
		sed 's#deb https://#deb [signed-by=/usr/share/keyrings/nvidia-container-toolkit-keyring.gpg] https://#' \
			>/etc/apt/sources.list.d/nvidia-container-toolkit.list
	apt-get update -qq
	apt_install nvidia-container-toolkit
	nvidia-ctk runtime configure --runtime=docker >/dev/null
	info "restarting Docker to load the NVIDIA runtime"
	systemctl restart docker
}

# A value from the existing .env, without quotes; empty when there is none.
stored() {
	[ -f "$dir/.env" ] || return 0
	sed -n "s/^$1=//p" "$dir/.env" | tail -n 1 | sed 's/^"\(.*\)"$/\1/'
}

# --- The server -------------------------------------------------------------------------------------

ensure_nvidia_smi
nvidia-smi -L >/dev/null 2>&1 || err "nvidia-smi cannot reach a GPU (nvidia-smi -L); check the driver"

gpu=${opt_gpu:-$(stored VOLTIP_GPU)}
gpu=${gpu:-0}
query=$(nvidia-smi -i "$gpu" --query-gpu=name,memory.total,compute_cap --format=csv,noheader,nounits 2>/dev/null) ||
	err "there is no GPU $gpu (nvidia-smi -L lists them)"
gpu_name=$(printf '%s' "$query" | awk -F', *' '{print $1}')
gpu_mib=$(printf '%s' "$query" | awk -F', *' '{print int($2)}')
gpu_cap=$(printf '%s' "$query" | awk -F', *' '{print $3}')
cap10=$(printf '%s' "$gpu_cap" | awk -F. '{print $1 * 10 + $2}')
[ "$cap10" -ge 75 ] || err "$gpu_name (compute capability $gpu_cap) is older than vLLM supports (7.5 or later)"
# bfloat16 needs compute capability 8.0.
if [ "$cap10" -ge 80 ]; then dtype=auto; else dtype=float16; fi

# The image for the CUDA version the driver supports: CUDA 13 needs a driver from 580 on.
cuda=$(nvidia-smi | sed -n 's/.*CUDA Version: *\([0-9][0-9]*\)\.[0-9][0-9]*.*/\1/p' | head -n 1)
is_uint "$cuda" || err "could not read the CUDA version the driver supports (nvidia-smi)"
if [ "$cuda" -ge 13 ]; then
	vllm_image=vllm/vllm-openai:$VLLM_VERSION
elif [ "$cuda" -eq 12 ]; then
	vllm_image=vllm/vllm-openai:$VLLM_VERSION-cu129
else
	err "the NVIDIA driver supports CUDA $cuda; vLLM needs a driver for CUDA 12 or later"
fi
info "GPU $gpu: $gpu_name, $gpu_mib MiB, compute capability $gpu_cap, driver for CUDA $cuda"

ensure_docker
ensure_toolkit

# --- Settings ---------------------------------------------------------------------------------------

if [ -n "$opt_domain" ]; then
	site=$opt_domain
elif [ -n "$opt_port" ]; then
	site=http://:$opt_port
else
	site=$(stored VOLTIP_SITE)
	site=${site:-http://:8080}
fi
case "$site" in
http://:*) domain= ; port=${site#http://:} ;;
*) domain=$site ; port= ;;
esac

model=${opt_model:-$(stored VOLTIP_MODEL)}
if [ -z "$model" ]; then
	if [ "$gpu_mib" -ge "$LARGE_MODEL_MIB" ]; then model=Qwen/Qwen3-ASR-1.7B; else model=Qwen/Qwen3-ASR-0.6B; fi
fi
case "$opt_source" in
modelscope) use_modelscope=true ;;
huggingface) use_modelscope=false ;;
*) use_modelscope=$(stored VOLTIP_USE_MODELSCOPE); use_modelscope=${use_modelscope:-false} ;;
esac
gpu_memory=${opt_gpu_memory:-$(stored VOLTIP_GPU_MEMORY_UTILIZATION)}
gpu_memory=${gpu_memory:-0.85}

setting() { # name default: the stored value, or the default
	value=$(stored "$1")
	printf '%s' "${value:-$2}"
}
max_model_len=$(setting VOLTIP_MAX_MODEL_LEN 8192)
max_num_seqs=$(setting VOLTIP_MAX_NUM_SEQS 8)
vllm_args=$(stored VOLTIP_VLLM_ARGS)
asr_port=$(setting VOLTIP_ASR_PORT 8001)
gateway_port=$(setting VOLTIP_GATEWAY_PORT 8090)
rate=$(setting VOLTIP_RATE 3)
burst=$(setting VOLTIP_BURST 30)
connections=$(setting VOLTIP_CONNECTIONS 8)
max_upload=$(setting VOLTIP_MAX_UPLOAD_MB 32)
timeout=$(setting VOLTIP_TIMEOUT 120)

# --- Ports ------------------------------------------------------------------------------------------

running=
if [ -f "$dir/compose.yaml" ]; then
	running=$(cd "$dir" && docker compose ps --status running --services 2>/dev/null || true)
fi
check_port() { # port service
	if printf '%s\n' "$running" | grep -qx "$2"; then return 0; fi
	if ss -Hltn "sport = :$1" 2>/dev/null | grep -q .; then
		err "port $1 is already in use on this server (ss -ltnp 'sport = :$1' shows by what)"
	fi
}
if [ -n "$domain" ]; then
	check_port 80 caddy
	check_port 443 caddy
else
	check_port "$port" caddy
fi
check_port "$gateway_port" gateway
check_port "$asr_port" asr

# --- Files ------------------------------------------------------------------------------------------

# Run from a checkout (sh deploy/selfhost/install.sh), the files beside it; piped, those of $ref.
here=
case "$0" in
*/install.sh | install.sh) here=$(cd "$(dirname -- "$0")" && pwd) ;;
esac
mkdir -p "$tmp/files/gateway"
if [ -n "$here" ] && [ -f "$here/compose.yaml" ] && [ "$here" != "$dir" ]; then
	for f in $FILES; do cp "$here/$f" "$tmp/files/$f"; done
else
	base=https://raw.githubusercontent.com/$REPO/$ref/deploy/selfhost
	info "downloading the service's files from $REPO ($ref)"
	for f in $FILES; do
		curl -fsSL --retry 3 "$base/$f" -o "$tmp/files/$f" || err "could not download $base/$f"
	done
fi
mkdir -p "$dir/gateway"
chmod 755 "$dir"
for f in $FILES; do
	cp "$tmp/files/$f" "$dir/$f"
	chmod 644 "$dir/$f"
done
chmod 755 "$dir/asr-serve.sh" "$dir/gateway/40-voltip-tokens.sh"

new_token=
if ! grep -Eqv '^[[:space:]]*(#|$)' "$dir/tokens" 2>/dev/null; then
	new_token="vasr_$(LC_ALL=C tr -dc 'A-Za-z0-9' </dev/urandom | head -c 40)"
	(
		umask 077
		printf '%s\n' \
			"# Access tokens of Voltip's recognition service, one a line. After an edit run" \
			"#   docker compose restart gateway" \
			"# in $dir. The gateway's log names a token by its line number." \
			"$new_token" >"$dir/tokens"
	)
fi
chmod 600 "$dir/tokens"
test_token=$(sed -n 's/#.*//; s/^[[:space:]]*//; s/[[:space:]]*$//; /./{p;q;}' "$dir/tokens")

{
	echo "# Settings of Voltip's self-hosted recognition service, written by install.sh. After a change, run"
	echo "#   docker compose up -d"
	echo "# in this directory. install.sh keeps them when it runs again, except VOLTIP_VLLM_IMAGE and"
	echo "# VOLTIP_DTYPE, which it chooses for this server's driver and GPU."
	echo "VOLTIP_SITE=$site"
	echo "VOLTIP_MODEL=$model"
	echo "VOLTIP_USE_MODELSCOPE=$use_modelscope"
	echo "VOLTIP_GPU=$gpu"
	echo "VOLTIP_GPU_MEMORY_UTILIZATION=$gpu_memory"
	echo "VOLTIP_MAX_MODEL_LEN=$max_model_len"
	echo "VOLTIP_MAX_NUM_SEQS=$max_num_seqs"
	echo "VOLTIP_VLLM_ARGS=\"$vllm_args\""
	echo "VOLTIP_VLLM_IMAGE=$vllm_image"
	echo "VOLTIP_DTYPE=$dtype"
	echo "VOLTIP_ASR_PORT=$asr_port"
	echo "VOLTIP_GATEWAY_PORT=$gateway_port"
	echo "VOLTIP_RATE=$rate"
	echo "VOLTIP_BURST=$burst"
	echo "VOLTIP_CONNECTIONS=$connections"
	echo "VOLTIP_MAX_UPLOAD_MB=$max_upload"
	echo "VOLTIP_TIMEOUT=$timeout"
} >"$dir/.env.new"
mv "$dir/.env.new" "$dir/.env"
chmod 644 "$dir/.env"

# --- Start ------------------------------------------------------------------------------------------

cd "$dir"
info "downloading the gateway and Caddy images"
docker compose pull --quiet gateway caddy

# The vLLM image: the official one plus vLLM's audio extra, which it ships without. It is made with
# docker commit rather than docker build: a build keeps a second copy of the 25 GB base image in
# BuildKit's cache. Made again only when the base changes; the image it replaces is removed below,
# and the base's own tag right away (its layers stay, as the new image's).
made_from=$(docker image inspect -f '{{index .Config.Labels "io.voltip.base"}}' "$LOCAL_IMAGE" 2>/dev/null || true)
old_image=
if [ "$made_from" != "$vllm_image" ]; then
	old_image=$(docker image inspect -f '{{.Id}}' "$LOCAL_IMAGE" 2>/dev/null || true)
	info "downloading $vllm_image (about 9 GB the first time)"
	docker pull "$vllm_image"
	version=$(docker run --rm --entrypoint python3 "$vllm_image" -c 'import importlib.metadata as m; print(m.version("vllm"))')
	info "adding the audio decoders of vLLM $version"
	docker rm -f voltip-asr-prepare >/dev/null 2>&1 || true
	docker run --name voltip-asr-prepare --entrypoint uv "$vllm_image" pip install --system --no-cache --quiet "vllm[audio]==$version"
	docker commit --change "LABEL io.voltip.base=$vllm_image" --change 'ENTRYPOINT ["vllm", "serve"]' \
		voltip-asr-prepare "$LOCAL_IMAGE" >/dev/null
	docker rm voltip-asr-prepare >/dev/null
	docker rmi "$vllm_image" >/dev/null 2>&1 || true
fi
docker run --rm --gpus "device=$gpu" --entrypoint nvidia-smi "$LOCAL_IMAGE" -L >/dev/null 2>&1 ||
	err "a container cannot use GPU $gpu; check the NVIDIA Container Toolkit (nvidia-ctk runtime configure --runtime=docker, then systemctl restart docker)"
docker compose up -d --remove-orphans
# The image an upgrade replaced, unless something else still uses it.
if [ -n "$old_image" ]; then docker rmi "$old_image" >/dev/null 2>&1 || true; fi

info "waiting for vLLM to serve $model; its first start downloads the model, which can take several minutes"
started=$(date +%s)
said=$started
while :; do
	code=$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$gateway_port/healthz" || true)
	[ "$code" = 200 ] && break
	id=$(docker compose ps -q asr)
	state=$(docker inspect -f '{{.State.Status}} {{.RestartCount}}' "$id" 2>/dev/null || echo "missing 0")
	case "$state" in
	running\ 0 | created\ 0 | restarting\ 0) ;;
	*)
		docker compose logs --tail 30 asr >&2 || true
		err "vLLM stopped while starting (above, its last lines); after fixing the cause, run this again"
		;;
	esac
	now=$(date +%s)
	[ $((now - started)) -lt 2700 ] || err "vLLM is not ready after 45 minutes; see docker compose logs asr in $dir"
	if [ $((now - said)) -ge 60 ]; then
		info "still starting ($(((now - started) / 60)) min): $(docker compose logs --no-log-prefix --tail 1 asr 2>/dev/null | cut -c1-160)"
		said=$now
	fi
	sleep 5
done
info "vLLM is ready after $(($(date +%s) - started)) s"

# One second of silence through the way in, as Voltip sends a recording: 16 kHz, 16-bit, mono WAV.
{
	printf 'RIFF\044\175\000\000WAVEfmt \020\000\000\000\001\000\001\000\200\076\000\000\000\175\000\000\002\000\020\000data\000\175\000\000'
	head -c 32000 /dev/zero
} >"$tmp/silence.wav"
if [ -n "$domain" ]; then
	url=https://$domain
	# To Caddy on this machine with the domain's name, so it needs no route back from outside.
	resolve="--resolve $domain:443:127.0.0.1"
else
	url=http://127.0.0.1:$port
	resolve=
fi
tries=0
while :; do
	# shellcheck disable=SC2086 # $resolve is empty or two words
	code=$(curl -s $resolve -o "$tmp/answer" -w '%{http_code}' -H "Authorization: Bearer $test_token" \
		-F "file=@$tmp/silence.wav;type=audio/wav" -F "model=$model" "$url/v1/audio/transcriptions" || true)
	if [ "$code" = 200 ] && grep -q '"text"' "$tmp/answer"; then break; fi
	tries=$((tries + 1))
	if [ "$tries" -ge 36 ]; then
		if [ -n "$domain" ]; then
			err "HTTPS at $domain does not answer yet (HTTP status ${code:-none}). Check that $domain resolves to this server and that ports 80 and 443 are open to the internet; Caddy's log: docker compose logs caddy"
		fi
		err "the service does not answer a test recording (HTTP status ${code:-none}); see docker compose logs in $dir"
	fi
	sleep 5
done

if [ -n "$domain" ]; then
	endpoint=https://$domain/v1
else
	address=$(hostname -I 2>/dev/null | awk '{print $1}')
	endpoint=http://${address:-<this server>}:$port/v1
fi
echo
echo "Voltip's recognition service is ready. In Voltip, open Speech models > Custom endpoint"
echo "(语音模型 > 自定义接口) and enter:"
echo
echo "  Endpoint (接口地址):  $endpoint"
if [ -n "$new_token" ]; then
	echo "  API key (API 密钥):   $new_token"
else
	echo "  API key (API 密钥):   a token from $dir/tokens"
fi
echo "  Model (模型):         $model"
echo
if [ -z "$domain" ]; then
	echo "The connection is not encrypted: use it on a network you trust only, or install with --domain."
fi
echo "Tokens: $dir/tokens. Settings: $dir/.env. Logs: cd $dir && docker compose logs -f"
