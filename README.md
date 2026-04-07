# Clawd

`clawd` is the WebUI backend agent responsible for orchestrating conversations between the browser UI, the model runner, and the local tool runtime.

Unlike a simple proxy, `clawd` executes tools locally and feeds results back into the model loop until a final response is produced.

This enables real analysis workflows such as:

- repository inspection
- multi-file reading
- semantic search
- CLI tool execution
- hook-driven policy enforcement
- plugin-based tool expansion

---

# Architecture Overview

```text
Browser UI
    ↓
POST /v1/messages
    ↓
clawd
    ↓
ConversationRuntime
    ↓
GlobalToolRegistry
    ↓
Tool Executor
    ↓
Hooks (Pre/Post)
    ↓
Tool Results
    ↓
Model Runner
    ↓
Final assistant response
```

The model does not hallucinate file contents. Instead, it invokes real tools which retrieve data from the local filesystem or other configured sources.

---

# Features

## Tool Execution Loop

`clawd` implements a full tool loop:

1. Send user request to runner.
2. Runner returns response blocks.
3. Detect `tool_use` blocks.
4. Execute tools locally.
5. Append tool results to the conversation.
6. Continue until a final assistant response is produced.

This enables iterative reasoning workflows where the model can inspect multiple files before generating an answer.

## Supported Tool Types

### File system tools

| Tool | Purpose |
|------|---------|
| `glob_search` | Find files matching patterns |
| `grep_search` | Search inside files |
| `read_file` | Read file contents |
| `list_dir` | Directory inspection |

### CLI tools

Tools can execute local commands through the tool executor.

Example use cases:

- cargo metadata inspection
- git queries
- custom analysis binaries
- language-specific parsers

## Hook System

Hooks allow inspection or modification of tool execution.

Supported hook events:

- `PreToolUse`
- `PostToolUse`
- `Stop`
- `SubagentStop`

Hooks can:

- block tool execution
- modify tool input
- log tool usage
- enforce policy constraints

Hook runner:

```rust
hooks::HookRunner
```

Configured via:

```text
~/.claw/hooks.json
```

Example:

```json
{
  "hooks": [
    {
      "event": "PreToolUse",
      "command": "echo tool invoked"
    }
  ]
}
```

---

# HTTP API

## `POST /v1/messages`

Primary conversation endpoint used by the WebUI.

### Request

```json
{
  "model": "runner",
  "messages": [
    {
      "role": "user",
      "content": [
        {
          "type": "text",
          "text": "analyze my project"
        }
      ]
    }
  ]
}
```

### Response

```json
{
  "content": [
    {
      "type": "text",
      "text": "Control flow overview..."
    }
  ]
}
```

Tool calls are handled internally and are not exposed directly to the WebUI.

---

# Configuration

## Runner endpoint

Default:

```text
http://127.0.0.1:11434/v1/messages
```

Configured in:

```text
src/main.rs
```

## Hooks directory

Default:

```text
~/.claw/hooks.json
```

Example:

```json
{
  "hooks": [
    {
      "event": "PreToolUse",
      "command": "echo tool invoked"
    }
  ]
}
```

---

# Build

Build `clawd`:

```bash
cargo build -p clawd
```

Build the full workspace:

```bash
cargo build --workspace
```

Release build:

```bash
cargo build -p clawd --release
```

---

# Running

Start runner:

```text
runner daemon
```

Start `clawd`:

```bash
cargo run -p clawd
```

Start WebUI:

```bash
npm install
npm run dev
```

Open:

```text
http://localhost:3000
```

---

# Example Workflow

User request:

```text
analyze my source code and give me an overview of control flow:

C:\repo\floatpack\src
```

Tool sequence:

1. `glob_search`
2. `read_file` on entry points like `main.rs` or `lib.rs`
3. `grep_search` for function references
4. `read_file` on dependent modules
5. summarize control flow

Final output is synthesized from real source files.

---

# Troubleshooting

## WebUI shows "Runner Unknown"

Verify runner is running:

```bash
curl http://127.0.0.1:11434/v1/messages
```

## No tools executing

Verify the tool registry is initialized:

```rust
GlobalToolRegistry::default()
```

## Hooks not firing

Check:

```text
~/.claw/hooks.json
```

Ensure valid JSON format.

## Requests not reaching backend

Check the browser console for:

```text
POST /v1/messages
```

Ensure the request returns HTTP 200.

---

# Development Notes

Key modules:

```text
ConversationRuntime
GlobalToolRegistry
CliToolExecutor
HookRunner
```

The WebUI backend should not directly forward requests to the runner without executing the tool loop.

---

# Summary

`clawd` acts as the orchestration layer between:

- WebUI
- model runner
- local tools
- hooks
- plugins

It ensures the model can iteratively inspect local resources before producing responses.
