# Claw Code (Offline Rewrite)

This fork is configured for **local-only model execution**.
It no longer depends on Anthropic API credentials or OAuth login.

## Local backend

The CLI expects an OpenAI-compatible local endpoint.
A practical default is an Ollama-compatible server listening on `http://127.0.0.1:11434`.

Environment variables:

```bash
export CLAW_LOCAL_BASE_URL="http://127.0.0.1:11434"
```

If `CLAW_LOCAL_BASE_URL` is not set, the client defaults to `http://127.0.0.1:11434`.
`OLLAMA_HOST` is also accepted as a fallback.

## Run

```bash
cargo run -p rusty-claude-cli --
cargo run -p rusty-claude-cli -- prompt "summarize this repo"
cargo run -p rusty-claude-cli -- --model qwen2.5-coder:14b "explain crates/runtime/src/conversation.rs"
```

## Model aliases

| Alias | Local model |
|---|---|
| `opus` | `qwen2.5-coder:32b` |
| `sonnet` | `qwen2.5-coder:14b` |
| `haiku` | `qwen2.5-coder:7b` |

## Current direction

- offline-first runtime
- local model transport
- no provider login/logout flow
- local retrieval/database work can be layered in later

## Workspace

- `crates/api/` — local model transport and request/response adaptation
- `crates/runtime/` — conversation runtime, sessions, permissions, prompts
- `crates/tools/` — tool registry and subagent support
- `crates/rusty-claude-cli/` — main CLI binary (`claw`)
