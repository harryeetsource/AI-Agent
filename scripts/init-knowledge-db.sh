#!/usr/bin/env bash
set -euo pipefail

DB_PATH="${1:-./data/knowledge.db}"
SCHEMA_PATH="${2:-./data/schema.sql}"

if [[ ! -f "$SCHEMA_PATH" ]]; then
  echo "Schema file not found: $SCHEMA_PATH" >&2
  exit 1
fi

if ! command -v sqlite3 >/dev/null 2>&1; then
  echo "sqlite3 was not found in PATH." >&2
  exit 1
fi

sqlite3 "$DB_PATH" ".read $SCHEMA_PATH"
echo "Initialized knowledge database at $DB_PATH"
