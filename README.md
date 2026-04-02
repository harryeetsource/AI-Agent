# Claw Code (Offline Rewrite)

This fork is configured for **local-only model execution**.
It no longer depends on Anthropic credentials, OAuth login, or OpenAI-compatible endpoints.

## Offline architecture

Claw now expects a **local Claw model daemon** that implements a small native HTTP API:

- `POST /v1/messages`
- request body: `crates/api::types::MessageRequest`
- response body: `crates/api::types::MessageResponse`

Default base URL:

```bash
CLAW_LOCAL_BASE_URL=http://127.0.0.1:8080
```

If `CLAW_LOCAL_BASE_URL` is not set, the client defaults to `http://127.0.0.1:8080`.

## What you need locally

You need three things to run the agent offline:

1. **A local model daemon**
   - host your chosen local model behind `POST /v1/messages`
   - keep the transport native to Claw instead of depending on OpenAI-compatible schemas

2. **A local retrieval database**
   - recommended: `SQLite` with `FTS5`
   - file example: `data/knowledge.db`

3. **A local corpus / dataset directory**
   - recommended root: `data/corpus/`
   - include Rust source, docs, examples, API notes, assembly references, and any curated engineering examples

## Recommended database layout

Create a SQLite database and enable FTS5. A good starting layout is:

- `documents`
  - `id`
  - `path`
  - `kind`
  - `sha256`
  - `updated_at`
- `chunks`
  - `id`
  - `document_id`
  - `chunk_index`
  - `text`
- `chunks_fts`
  - FTS5 virtual table over `text`
- `sessions`
  - session transcripts and summaries
- `tool_results`
  - cached tool outputs and structured notes

## Suggested offline corpus contents

Populate `data/corpus/` with compact, high-value material instead of giant web dumps:

- this Claw workspace
- Rust standard library notes you rely on frequently
- Rust examples and internal coding patterns
- assembly references and calling-convention notes
- curated bug-fix examples
- local markdown design notes

For a desktop-friendly setup, prefer:

- your own repo and docs first
- curated Rust/assembly examples second
- SQLite FTS retrieval over massive raw datasets

## Minimal setup flow

### 1. Create the data directories

```bash
mkdir -p data/corpus
mkdir -p data/db
```

### 2. Create the SQLite database

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

### 3. Seed the corpus

Copy the Claw repo, your Rust notes, and assembly references into `data/corpus/`.
Then chunk and ingest them into `knowledge.db`.

### 4. Start your local model daemon

Your daemon should accept Claw-native `MessageRequest` JSON and return Claw-native `MessageResponse` JSON.

### 5. Run Claw

```bash
export CLAW_LOCAL_BASE_URL="http://127.0.0.1:8080"
cargo run -p rusty-claude-cli --
cargo run -p rusty-claude-cli -- prompt "summarize this repo"
```

## Current direction

- offline-first runtime
- native local transport
- no provider login/logout flow
- SQLite-backed local retrieval
- Rust and assembly corpus integration layered on top

## Workspace

- `crates/api/` — local model transport and request/response types
- `crates/runtime/` — conversation runtime, sessions, permissions, prompts
- `crates/tools/` — tool registry and subagent support
- `crates/rusty-claude-cli/` — main CLI binary (`claw`)
