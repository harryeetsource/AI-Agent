# Claw Code (Offline Rewrite)

This fork is configured for **local-only model execution**.

It does **not** require:

- Anthropic API keys
- OAuth login
- OpenAI-compatible schemas
- cloud provider authentication

Instead, it uses:

- a **local model runner**
- a **local daemon (`clawd`)**
- a **local tool/runtime loop**
- an optional **SQLite retrieval database**

---

## Quick Start

### 1. Build the workspace

From the `rust/` directory:

```bash
cargo build --workspace --release
```

---

### 2. Install SQLite (required for local retrieval DB)

If you plan to use the retrieval/indexing workflow, you need the `sqlite3` CLI installed and available in your `PATH`. The project’s offline docs already depend on `sqlite3 data/db/knowledge.db` for DB creation. 

#### Windows

1. Download the SQLite tools bundle from the official SQLite download page.
2. Extract `sqlite3.exe` to a permanent location such as:

```text
C:\Tools\sqlite\
```

3. Add that directory to your system `PATH`.
4. Verify:

```powershell
sqlite3 --version
```

#### Linux

```bash
sudo apt install sqlite3
```

#### macOS

```bash
brew install sqlite
```

Without `sqlite3`, local DB initialization and indexing will fail. :contentReference[oaicite:1]{index=1}

---

### 3. Place your local model and runner binaries

Expected layout from the offline setup notes:

```text
rust/
├── data/
│   ├── models/
│   │   └── qwen2.5-coder-1.5b-instruct-q4_k_m.gguf
│   ├── knowledge.db
│   └── corpus/
│       ├── rust/
│       ├── asm/
│       └── docs/
├── runners/
│   └── llama/
│       ├── llama-server.exe
│       └── llama-cli.exe
├── scripts/
└── .env.example
``` 

This layout is explicitly described in the offline setup notes. :contentReference[oaicite:2]{index=2}

---

### 4. Initialize the SQLite database

If your project includes the helper scripts, use them:

#### PowerShell

```powershell
.\scripts\init-knowledge-db.ps1
```

#### Bash

```bash
./scripts/init-knowledge-db.sh
```

If you need to create it manually:

```bash
sqlite3 data/db/knowledge.db <<'SQL'
CREATE TABLE IF NOT EXISTS documents (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS chunks (
    id INTEGER PRIMARY KEY,
    document_id INTEGER NOT NULL,
    chunk_index INTEGER NOT NULL,
    text TEXT NOT NULL,
    FOREIGN KEY(document_id) REFERENCES documents(id)
);

CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
    text,
    content='chunks',
    content_rowid='id'
);

CREATE TABLE IF NOT EXISTS sessions (
    id INTEGER PRIMARY KEY,
    session_key TEXT NOT NULL UNIQUE,
    summary TEXT,
    transcript_json TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS tool_results (
    id INTEGER PRIMARY KEY,
    tool_name TEXT NOT NULL,
    input_json TEXT NOT NULL,
    output_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);
SQL
```

That schema is based on the offline rewrite notes. 

---

### 5. Start the local model runner

Your local runner must accept native Claw `MessageRequest` JSON and return native Claw `MessageResponse` JSON. The API types support:

- `tools`
- `tool_choice`
- `tool_use`
- `tool_result`

through the native request/response schema. 

The current `clawd` daemon defaults to:

- **daemon host:** `127.0.0.1`
- **daemon port:** `8080`
- **runner base URL:** `http://127.0.0.1:8081`
- **runner messages path:** `/v1/messages` :contentReference[oaicite:5]{index=5}

So your normal startup flow is:

#### PowerShell

```powershell
.\scripts\run-llama.ps1
```

#### Bash

```bash
./scripts/run-llama.sh
```

If you start the runner manually, make sure it serves:

```text
http://127.0.0.1:8081/v1/messages
```

---

### 6. Start `clawd`

From `rust/`:

```bash
cargo run -p clawd
```

By default, `clawd` serves the Web UI backend on:

```text
http://127.0.0.1:8080
```

and forwards model requests to the runner at:

```text
http://127.0.0.1:8081/v1/messages
```

Those defaults come directly from the current daemon source. :contentReference[oaicite:6]{index=6}

---

### 7. Start the CLI (optional)

The CLI binary is `claw`. The current CLI help/output path is in `rusty-claude-cli`, and the default model aliases resolve to local Qwen variants rather than cloud models. For example:

- `opus` → `qwen2.5-coder:32b`
- `sonnet` → `qwen2.5-coder:14b`
- `haiku` → `qwen2.5-coder:7b` :contentReference[oaicite:7]{index=7}

Run the REPL:

```bash
cargo run -p rusty-claude-cli --
```

Run a one-shot prompt:

```bash
cargo run -p rusty-claude-cli -- prompt "summarize this repo"
```

The CLI help and usage flow are defined in the current CLI source. :contentReference[oaicite:8]{index=8}

---

## Current Runtime Topology

The current offline stack is:

```text
Browser / Web UI
    ↓
clawd (127.0.0.1:8080)
    ↓
local runner (127.0.0.1:8081/v1/messages)
    ↓
tool loop / hooks / plugins / runtime
```

The daemon currently exposes:

- `/`
- `/static/app.js`
- `/static/styles.css`
- `/health`
- `/v1/messages` :contentReference[oaicite:9]{index=9}

The Web UI posts directly to `/v1/messages` with a native `MessageRequest` payload and `stream: false`. :contentReference[oaicite:10]{index=10}

---

## Tooling and Agent Loop

The project already has native tool schemas and tool-capable message types:

### Request capabilities

`MessageRequest` supports:

- `model`
- `messages`
- `system`
- `tools`
- `tool_choice`
- `stream` :contentReference[oaicite:11]{index=11}

### Response capabilities

`MessageResponse` supports `OutputContentBlock`, including:

- `Text`
- `ToolUse`
- `Thinking`
- `RedactedThinking` :contentReference[oaicite:12]{index=12}

### Built-in tools

The current tool registry includes tools such as:

- `bash`
- `read_file`
- `write_file`
- `edit_file`
- `glob_search`
- `grep_search`
- `WebFetch`
- `WebSearch`
- `TodoWrite`
- `Skill`
- `Agent`
- `ToolSearch`
- `NotebookEdit`
- `Sleep`
- `SendUserMessage`
- `Config`
- `StructuredOutput`
- `REPL`
- `PowerShell` 

### Hooks

The hook system already supports:

- `PreToolUse`
- `PostToolUse`

and can:

- allow tool execution
- deny tool execution
- emit warning messages
- attach extra output text 

---

## Optional Retrieval / Corpus Workflow

The offline rewrite notes recommend three local inputs:

1. a local model daemon
2. a local SQLite retrieval DB
3. a local corpus directory :contentReference[oaicite:15]{index=15}

Recommended corpus root:

```text
data/corpus/
```

Recommended contents:

- this Claw workspace
- Rust source and notes
- assembly references
- curated bug-fix examples
- local markdown design notes :contentReference[oaicite:16]{index=16}

Desktop-friendly guidance from the notes:

- your own repo and docs first
- curated Rust/assembly examples second
- SQLite FTS retrieval instead of massive raw datasets :contentReference[oaicite:17]{index=17}

---

## Common Commands

### Build everything

```bash
cargo build --workspace --release
```

### Run the daemon

```bash
cargo run -p clawd
```

### Run the CLI REPL

```bash
cargo run -p rusty-claude-cli --
```

### Run a one-shot prompt

```bash
cargo run -p rusty-claude-cli -- prompt "summarize this repo"
```

### Initialize the retrieval DB (PowerShell)

```powershell
.\scripts\init-knowledge-db.ps1
```

### Launch the local runner (PowerShell)

```powershell
.\scripts\run-llama.ps1
```

---

## Troubleshooting

### `sqlite3` is not recognized

Install SQLite and add `sqlite3.exe` to your `PATH`, then verify:

```powershell
sqlite3 --version
```

This is required for DB initialization and indexing. :contentReference[oaicite:18]{index=18}

---

### `clawd` is running but Web UI shows runner problems

Check the daemon defaults:

- daemon: `127.0.0.1:8080`
- runner: `127.0.0.1:8081/v1/messages` :contentReference[oaicite:19]{index=19}

Verify the runner directly:

```powershell
Invoke-WebRequest http://127.0.0.1:8081/v1/messages -Method POST -ContentType "application/json" -Body '{"model":"local","max_tokens":16,"messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}],"stream":false}'
```

---

### The model gives generic “I cannot access your files” answers

That means the current path is still not using the tool-capable runtime loop properly, even though the schema, tool registry, and hook runner all exist in the codebase. The current Web UI request builder does send native requests to `/v1/messages`, but by itself that does not guarantee that tool execution is happening. 

---

## Workspace Overview

The offline notes describe the workspace direction as:

- `crates/api/` — local model transport and request/response types
- `crates/runtime/` — conversation runtime, sessions, permissions, prompts
- `crates/tools/` — tool registry and subagent support
- `crates/rusty-claude-cli/` — main CLI binary (`claw`) :contentReference[oaicite:21]{index=21}

The current CLI/runtime/tool source you uploaded matches that architecture:
- local Qwen defaults in the CLI/runtime path :contentReference[oaicite:22]{index=22}
- tool execution through `GlobalToolRegistry` and `CliToolExecutor` :contentReference[oaicite:23]{index=23}
- hook execution through the hook runner :contentReference[oaicite:24]{index=24}

---

## Summary

This repo is intended to run:

- **fully offline**
- with a **local runner**
- with a **local daemon**
- with **native tool calls**
- with an optional **SQLite-backed retrieval layer**

The key startup points are:

- `clawd` on **127.0.0.1:8080**
- local runner on **127.0.0.1:8081/v1/messages**
- optional SQLite retrieval via `sqlite3`
- CLI via `cargo run -p rusty-claude-cli --`

That setup is consistent with the current daemon defaults and the offline rewrite documentation. 
