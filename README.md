# 🦞 Claw Code — Rust Implementation

A high-performance Rust implementation of the **Claw autonomous code analysis agent**.

Claw performs **deep structural analysis of real source code projects** using native Rust tooling.  
It runs fully locally and directly invokes tools from the workspace to inspect files, discover relationships, and produce architectural insights.

No external API providers are required.

---

# Quick Start

```bash
# build
cd rust/
cargo build --release

# interactive REPL
./target/release/claw

# one-shot analysis
./target/release/claw prompt "analyze this repository"

# analyze a specific path
./target/release/claw prompt "analyze ./crates/runtime"
```

Claw automatically performs analysis by invoking local tools such as:

- glob_search
- grep_search
- read_file
- tool_search
- todo_write
- notebook_edit

The agent inspects source files directly and produces structured explanations of:

- architecture
- module relationships
- control flow
- data flow
- complexity risks
- refactoring opportunities

---

# First Time Setup

## Install SQLite (required for local retrieval index)

Claw uses SQLite to store indexed file metadata for retrieval-based analysis.

You must install the `sqlite3` CLI and ensure it is available in PATH.

### Windows

1. Download SQLite tools bundle:
   https://www.sqlite.org/download.html

2. Extract `sqlite3.exe` to:

```
C:\Tools\sqlite\
```

3. Add that directory to PATH.

4. Verify installation:

```powershell
sqlite3 --version
```

---

### Linux

```bash
sudo apt install sqlite3
```

---

### macOS

```bash
brew install sqlite
```

Without sqlite3, local indexing and retrieval initialization will fail.

---

# Configuration

Claw runs fully locally by default.

Configuration sources:

- `.claude.json`
- `CLAUDE.md`
- environment variables
- workspace tool manifests

Optional environment variables:

| Variable | Purpose |
|----------|--------|
| CLAW_SQLITE_PATH | override sqlite database location |
| CLAW_LOG_LEVEL | debug / info / warn / error |
| CLAW_MAX_DEPTH | limit recursive analysis depth |
| CLAW_MAX_FILES | cap file traversal size |
| CLAW_CACHE_DIR | override cache directory |

Example:

```bash
export CLAW_LOG_LEVEL=debug
export CLAW_MAX_DEPTH=6
```

---

# Autonomous Analysis Behavior

When given a prompt referencing a path, project, or repository, Claw:

1. discovers files using glob_search
2. locates relevant symbols using grep_search
3. reads source files using read_file
4. constructs a structural understanding of the project
5. produces:

- architectural overview
- module relationships
- data flow insights
- complexity observations
- improvement recommendations

Claw performs the analysis itself rather than instructing the user to manually run commands.

---

# Features

| Feature | Status |
|--------|--------|
| local autonomous agent loop | ✅ |
| native Rust tool execution | ✅ |
| recursive project analysis | ✅ |
| structural code understanding | ✅ |
| symbol discovery | ✅ |
| interactive REPL | ✅ |
| one-shot prompt mode | ✅ |
| session persistence | ✅ |
| SQLite retrieval index | ✅ |
| notebook editing | ✅ |
| todo tracking | ✅ |
| git-aware context | ✅ |
| markdown terminal rendering | ✅ |
| slash commands | ✅ |
| tool orchestration planning | ✅ |
| config hierarchy (.claude.json) | ✅ |
| CLAUDE.md project memory | ✅ |
| sub-agent task decomposition | ✅ |
| plugin system | planned |
| skills registry | planned |

---

# CLI Usage

```
claw [OPTIONS] [COMMAND]

Options:
  --model MODEL                reserved for future local models
  --permission-mode MODE       read-only | workspace-write | danger-full-access
  --allowedTools TOOLS         restrict tool usage
  --output-format FORMAT       text | json
  --version, -V                show version

Commands:
  prompt <text>      run single analysis prompt
  init               initialize workspace config
  doctor             check environment health
  self-update        update binary
```

---

# Slash Commands (REPL)

| Command | Description |
|--------|-------------|
| /help | show help |
| /status | show session state |
| /clear | clear conversation |
| /memory | show CLAUDE.md |
| /config | show configuration |
| /diff | show git diff |
| /export | export conversation |
| /session | resume previous session |
| /version | show version |

---

# Workspace Layout

```
rust/
├── Cargo.toml
├── Cargo.lock
└── crates/
    ├── api/                local model adapter interface
    ├── commands/           slash command registry
    ├── compat-harness/     manifest extraction harness
    ├── runtime/            agent loop and planning engine
    ├── rusty-claude-cli/   CLI interface
    └── tools/              tool implementations
```

---

# Crate Responsibilities

## runtime

Core agent loop:

- conversation state
- planning logic
- tool orchestration
- context assembly
- session persistence

## tools

Tool implementations:

| Tool | Purpose |
|------|--------|
| glob_search | enumerate files |
| grep_search | search symbols |
| read_file | inspect source |
| write_file | modify files |
| edit_file | patch content |
| tool_search | discover tools |
| todo_write | track tasks |
| notebook_edit | structured notes |
| bash | shell execution |

## rusty-claude-cli

CLI interface:

- interactive REPL
- streaming output
- slash commands
- argument parsing

## commands

Slash command definitions.

## compat-harness

Extracts manifest definitions from tool specifications.

## api

Interface layer for future local model backends.

---

# Example Prompts

Analyze entire workspace:

```
analyze this repository
```

Analyze a specific crate:

```
analyze ./crates/runtime
```

Find architectural issues:

```
identify architectural risks in this project
```

Map module relationships:

```
map module relationships and data flow
```

Locate complex code:

```
find high complexity modules
```

Security review:

```
identify unsafe patterns
```

---

# Stats

| Metric | Value |
|--------|------|
| language | Rust |
| workspace crates | 6 |
| binary name | claw |
| execution model | local |
| analysis depth | recursive |

---

# License

See repository root.
