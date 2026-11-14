#!/usr/bin/env bash
# An example model server for Lumbridge Voice: NVIDIA Nemotron 3.5 Transcribe 30B-A4B NVFP4 on vLLM 0.32
# with the NeMo Speech plugin, following the "Start the Server" profile of the model card.
#
# It listens on 127.0.0.1:8807, the front's default `transcribe.url`. Callers talk to the front, not to it.
# Needs: an NVIDIA GPU that runs NVFP4 (Blackwell), CUDA 13, and a Python environment with vLLM 0.32 and
# NeMo Speech installed, with `vllm` on PATH. The weights are downloaded from Hugging Face on first start
# and have their own licence (see the model card).
#
#   PORT=8807 GPU_UTIL=0.5 examples/serve-transcribe-vllm.sh
#
# GPU_UTIL is the share of GPU memory vLLM may take. Any extra arguments go to `vllm serve`.
set -euo pipefail
MODEL="${MODEL:-nvidia/NVIDIA-Nemotron-3.5-Transcribe-30B-A4B-NVFP4}"
export VLLM_PLUGINS=nemo_speechlm
export VLLM_MAX_AUDIO_DECODE_DURATION_S=3600
export VLLM_MAX_AUDIO_CLIP_FILESIZE_MB=180
export VLLM_USE_V2_MODEL_RUNNER=0                              # word timestamps need the V1 runner
export VLLM_DISABLED_KERNELS=FlashInferFP8ScaledMMLinearKernel  # as the model card says
exec vllm serve "$MODEL" \
  --served-model-name nvidia/NVIDIA-Nemotron-3.5-Transcribe-30B-A4B-NVFP4 \
  --host 127.0.0.1 --port "${PORT:-8807}" \
  --trust-remote-code --dtype bfloat16 --kv-cache-dtype fp8 --gpu-memory-utilization "${GPU_UTIL:-0.5}" \
  --max-model-len 65536 --max-num-batched-tokens 4096 --max-num-seqs 8 \
  --enable-chunked-prefill --no-enable-prefix-caching \
  --use-replayssm --mamba-backend flashinfer --mamba-cache-mode none \
  --mm-processor-cache-gb 0 --no-async-scheduling \
  --limit-mm-per-prompt '{"audio":{"count":1,"length":57600000}}' \
  --speculative-config '{"method":"mtp","num_speculative_tokens":8}' \
  --reasoning-parser nemotron_v3 \
  --trust-request-mm-kwargs \
  --middleware nemo.collections.speechlm2.vllm.salm.ctc_serving.ctc_timestamp_middleware "$@"
