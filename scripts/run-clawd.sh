#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"

export CLAW_RUNNER_BASE_URL="${CLAW_RUNNER_BASE_URL:-http://127.0.0.1:8081}"
export CLAW_RUNNER_MESSAGES_PATH="${CLAW_RUNNER_MESSAGES_PATH:-/v1/messages}"
export CLAWD_HOST="${CLAWD_HOST:-127.0.0.1}"
export CLAWD_PORT="${CLAWD_PORT:-8080}"
export CLAW_LOCAL_BASE_URL="http://${CLAWD_HOST}:${CLAWD_PORT}"

exec "$REPO_ROOT/target/releases/clawd"
