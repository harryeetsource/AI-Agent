# Claw Code (Offline Rewrite)

This fork is configured for **local-only model execution**.
It no longer depends on Anthropic credentials or provider login flows.

## Offline architecture

Claw now expects a **local Claw model daemon** that implements a native HTTP API:

- `POST /v1/messages`
- request body: `crates/api::types::MessageRequest`
- response body: `crates/api::types::MessageResponse`

By default, the CLI talks to:

```bash
CLAW_LOCAL_BASE_URL=http://127.0.0.1:8080
```

The included `clawd` daemon is the local bridge you ship with Claw. It listens on `127.0.0.1:8080` by default and forwards Claw-native `MessageRequest` payloads to your local model runner.

## Included starter pieces

- `crates/clawd/` — local native daemon
- `data/schema.sql` — SQLite schema for `knowledge.db`
- `data/corpus/` — starter corpus directories
- `data/models/` — place GGUF model files here
- `runners/llama/` — place `llama.cpp` binaries here
- `scripts/init-knowledge-db.*` — initialize the SQLite DB
- `scripts/run-llama.*` — start `llama-server`
- `scripts/run-clawd.*` — start the Claw-native bridge daemon

## What you need locally

1. **A local model file**
   - example: `data/models/qwen2.5-coder-1.5b-instruct-q4_k_m.gguf`

2. **A local runner binary**
   - example: `runners/llama/llama-server.exe`

3. **A local retrieval database**
   - example: `data/knowledge.db`

4. **A local corpus directory**
   - example: `data/corpus/`

## Runner compatibility

`llama.cpp` exposes an HTTP server that supports OpenAI-compatible routes and an **Anthropic Messages API compatible** route. That makes it a practical local backend for Claw while keeping Claw's external API native. citeturn749974search0turn749974search1

The included `clawd` daemon forwards Claw requests to the runner at:

```bash
CLAW_RUNNER_BASE_URL=http://127.0.0.1:8081
CLAW_RUNNER_MESSAGES_PATH=/v1/messages
```

## Directory layout

```text
rust/
├── crates/
│   └── clawd/
├── data/
│   ├── models/
│   ├── corpus/
│   ├── schema.sql
│   └── knowledge.db
├── runners/
│   └── llama/
├── scripts/
│   ├── init-knowledge-db.ps1
│   ├── init-knowledge-db.sh
│   ├── run-llama.ps1
│   ├── run-llama.sh
│   ├── run-clawd.ps1
│   └── run-clawd.sh
└── .env.example
```

## Minimal setup

### 1. Put your model here

```text
data/models/qwen2.5-coder-1.5b-instruct-q4_k_m.gguf
```

### 2. Put your runner binaries here

Windows:

```text
runners/llama/llama-server.exe
runners/llama/llama-cli.exe
```

### 3. Initialize the database

PowerShell:

```powershell
powershell -ep bypass .\scripts\init-knowledge-db.ps1
```

Bash:

```bash
./scripts/init-knowledge-db.sh
```

### 4. Start llama.cpp on port 8081

PowerShell:

```powershell
powershell -ep bypass .\scriptsun-llama.ps1
```

Bash:

```bash
./scripts/run-llama.sh
```

### 5. Start the Claw-native daemon on port 8080

PowerShell:

```powershell
powershell -ep bypass .\scriptsun-clawd.ps1
```

Bash:

```bash
./scripts/run-clawd.sh
```

### 6. Start the CLI

```bash
cargo run -p rusty-claude-cli --
```

Single-prompt mode:

```bash
cargo run -p rusty-claude-cli -- prompt "summarize this repo"
```

## Notes

- The helper scripts now resolve paths relative to the repo, not the current shell directory.
- `clawd` is intentionally small: it is the ship-ready local bridge, while `llama.cpp` remains the inference engine and `knowledge.db` remains the retrieval layer.
