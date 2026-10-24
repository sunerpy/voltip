#!/bin/sh
# The asr service's entrypoint (compose.yaml mounts it): start vLLM with the settings compose.yaml
# passes in from .env. vLLM listens on 127.0.0.1 only: requests from other computers reach it through
# the gateway.
set -eu
set -- serve "${VOLTIP_MODEL:-Qwen/Qwen3-ASR-1.7B}" \
	--host 127.0.0.1 \
	--port "${VOLTIP_ASR_PORT:-8001}" \
	--dtype "${VOLTIP_DTYPE:-auto}" \
	--max-model-len "${VOLTIP_MAX_MODEL_LEN:-8192}" \
	--gpu-memory-utilization "${VOLTIP_GPU_MEMORY_UTILIZATION:-0.85}" \
	--max-num-seqs "${VOLTIP_MAX_NUM_SEQS:-8}"
# VOLTIP_VLLM_ARGS adds vLLM options of your own, separated by spaces, e.g. "--enforce-eager".
# shellcheck disable=SC2086 # split on purpose
exec vllm "$@" ${VOLTIP_VLLM_ARGS:-}
