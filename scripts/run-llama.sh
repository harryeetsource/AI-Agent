#!/usr/bin/env bash
set -euo pipefail

MODEL_PATH="${1:-./data/models/qwen2.5-coder-1.5b-instruct-q4_k_m.gguf}"
SERVER_PATH="${CLAW_LLAMA_SERVER_PATH:-./runners/llama/llama-server}"
HOST="${CLAW_LLAMA_HOST:-127.0.0.1}"
PORT="${CLAW_LLAMA_PORT:-8081}"
CONTEXT="${CLAW_LLAMA_CONTEXT:-8192}"

if [[ ! -f "$SERVER_PATH" ]]; then
  echo "llama server binary not found: $SERVER_PATH" >&2
  exit 1
fi

if [[ ! -f "$MODEL_PATH" ]]; then
  echo "GGUF model not found: $MODEL_PATH" >&2
  exit 1
fi

echo "Starting llama.cpp server..."
echo "Binary: $SERVER_PATH"
echo "Model : $MODEL_PATH"
echo "URL   : http://$HOST:$PORT"

exec "$SERVER_PATH" -m "$MODEL_PATH" --host "$HOST" --port "$PORT" -c "$CONTEXT"
