#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"
DATABASE_PATH="${1:-$REPO_ROOT/data/knowledge.db}"
SCHEMA_PATH="${2:-$REPO_ROOT/data/schema.sql}"

if [[ ! -f "$SCHEMA_PATH" ]]; then
  echo "Schema file not found: $SCHEMA_PATH" >&2
  exit 1
fi

if ! command -v sqlite3 >/dev/null 2>&1; then
  echo "sqlite3 was not found in PATH. Install sqlite3 and rerun this script." >&2
  exit 1
fi

mkdir -p "$(dirname -- "$DATABASE_PATH")"
sqlite3 "$DATABASE_PATH" ".read $SCHEMA_PATH"
echo "Initialized knowledge database at $DATABASE_PATH"
