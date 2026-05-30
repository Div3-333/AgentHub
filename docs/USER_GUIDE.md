# AgentHub User Guide

AgentHub is a **Phantom Terminal Orchestrator**: one terminal UI that runs multiple AI CLIs (Gemini, Claude Code, Codex, Aider, Cursor CLI, and others) as hidden child processes, routes their output through a message bus, and gives you Discord-like workspace modes for multi-agent work.

This guide describes behavior verified in the **v0.1.0** codebase. For the full engineering spec, see [AGENTHUB_BLUEPRINT.md](AGENTHUB_BLUEPRINT.md). For what ships next, see [ROADMAP.md](ROADMAP.md).

---

## Prerequisites

| Requirement | Notes |
|-------------|--------|
| **Terminal** | 80×24 minimum; 120×40 or larger recommended for racing and sidebar layouts |
| **At least one AI CLI** | On your `PATH` and already authenticated (AgentHub does not store API keys) |
| **Rust toolchain** | Only if building from source ([INSTALL.md](INSTALL.md) covers pre-built binaries) |

Bundled driver profiles: `gemini`, `claude`, `codex`, `aider`, `cursor` (see `drivers/*.json`).

---

## Installation

### Pre-built binary (recommended)

Download the asset for your OS from [GitHub Releases](https://github.com/Div3-333/AgentHub/releases) when a `v*` tag exists (e.g. `v0.1.0`). Platform steps: [INSTALL.md](INSTALL.md).

### From source

```bash
git clone <your-repo-url>
cd AgentHub
cargo build -p agenthub --release
```

Binary: `target/release/agenthub` (or `agenthub.exe` on Windows).

### First-run files

| Path | Purpose |
|------|---------|
| `~/.agenthub/config.json` | Default mode, paths, `max_agents`, theme, `log_level`, `pty_debug_log` (default `false`) |
| `~/.agenthub/agenthub.db` | SQLite session log (WAL mode) |
| `~/.agenthub/drivers/` | Optional custom driver JSON (falls back to repo `drivers/`) |
| `.agenthub_shadow/` (in project CWD) | VFS snapshot copies (`shadow_dir` in config) |

---

## First run

1. Open a terminal in the **project directory** where agents should read and write files.
2. Run `agenthub` (or `cargo run -p agenthub` from a dev build).
3. AgentHub loads config, opens the database, starts the bus router, and opens the **live** TUI (`run_with_bridge`). No agents are spawned automatically.
4. Spawn a CLI, for example:

   ```text
   /spawn gemini
   ```

   Or press **F5**, type `gemini`, and press **Enter** (sends `/spawn gemini`).
5. Wait for induction (`READY` in the PTY). The agent appears in the sidebar when `AgentOnline` fires.
6. Send a prompt:

   ```text
   @gemini-1 explain this repo
   ```

   Tags are assigned at spawn (`gemini-1`, `gemini-2`, …). Use `/help` for the full command list.

Press **Ctrl+Q**, then confirm, to quit. All agent PTYs are torn down on exit.

---

## Workspace modes

Switch with `/mode dm`, `/mode groupchat`, or `/mode server`. **F2** cycles modes and, in a live session, sends the matching `/mode` command to core.

| Mode | Behavior |
|------|----------|
| **DM** (`direct_message`) | At most one active agent; no broadcast to other agents; implicit full permissions for the lone agent |
| **Group Chat** (default) | Up to `max_agents` (default 16); broadcast to all non-deafened agents; permissive RBAC; staggered injection when multiple agents are “thinking” |
| **Server** | Strict RBAC (`SEND_MESSAGES`, `RECEIVE_BROADCAST`, `EXECUTE_UNIX`, `WRITE_FILES`, etc.); `#channel` in a message limits broadcast to agents assigned to that channel (core API — no `/channel` slash command in v0.1) |

**DM spawn conflict:** spawning a second agent in DM mode is rejected or requires resolving the existing agent (see core `modes`).

---

## Talking to agents

### Direct mention

```text
@gemini-1 explain this module
```

The bus resolves the tag (case-insensitive) and injects into that agent’s PTY. In Group Chat / Server, other agents may still receive a broadcast unless they are deafened.

### Broadcast (no `@`)

In Group Chat and Server, a line without a resolved `@` target broadcasts to all non-deafened agents (subject to mode and `#channel` filter).

### Server channels

Include a channel tag in the message:

```text
#backend @gemini-1 update the API schema
```

Only agents assigned to `backend` receive the broadcast. Channel membership is managed via core APIs today (v0.2 may add slash commands).

### Auto-context (v0.2)

The library can prepend file or symbol snippets when prompts mention paths (`auth.rs`) or indexed symbol names. Prefix with `--nocontext` to skip. **v0.1:** injection is implemented in `agenthub_core::context` but not applied automatically on bus send.

---

## Slash commands

Type `/help` or press **F1** for the in-app catalog. With a live session (`agenthub` binary), commands run through core except where noted below.

### Moderation & agents (live)

| Command | Effect |
|---------|--------|
| `/mute @tag` | Hide agent output in chat (process keeps running) |
| `/unmute @tag` | Restore chat visibility |
| `/deafen @tag` | Stop broadcast injection (still gets direct user `@` mentions) |
| `/undeafen @tag` | Restore broadcast |
| `/timeout @tag 30s` | Suspend process (`30s`, `5m`, `2h`) |
| `/kick @tag [reason]` | Terminate agent |
| `/ban @tag [reason]` | Kick + block driver for this session |
| `/promote @tag to Role` | Assign role and permissions |
| `/demote @tag` | Revert to Observer |
| `/addrole Name PERM…` | Create custom role (Server mode) |
| `/removerole Name` | Delete custom role (not built-ins) |
| `/mode dm\|groupchat\|server` | Change workspace mode |
| `/spawn driver [--role R] [--tag T]` | Spawn CLI from driver profile |
| `/setprompt @tag text` | Inject system text into agent PTY |

### VFS (live / partial)

| Command | v0.1 behavior |
|---------|----------------|
| `/snapshot` | **Works** — checkpoints project tree under `.agenthub_shadow/` |
| `/undo` | **Listed in help; not wired** — returns `unknown command` from moderation dispatch. Revert API exists for v0.2 (**Ctrl+Z** same) |

### Sparring (library only in v0.1)

```text
/spar @gemini-1 as Coder vs @claude-1 as Reviewer --turns 5 --goal "Implement TCP echo server"
```

Parsed and executed by `SparEngine` in core tests; **not** routed through live slash dispatch yet. **Esc** sets `SPAR_ABORT` when a session is running from code.

---

## Frankenstein pipelines

Multi-step workflows use **space-pipe-space** between stages:

```text
@gemini-1 write a Rust HTTP server | > cargo check | @claude-1 fix the errors
```

| Stage prefix | Meaning |
|--------------|---------|
| `@tag prompt` | Send prompt to that agent |
| `> command` | Shell: `sh -c` (Unix) or `cmd /C` (Windows); previous stage stdout → stdin |

**v0.1:** `PipelineExecutor` runs this in integration tests. Typing a pipeline in the chat box sends it as a normal user message (no automatic stage execution). **v0.2** will detect ` | ` on submit and emit `PipelineStarted` / stage events for the sidebar.

Rules (when executed via executor):

- Unix stage non-zero exit → stderr becomes pipeline output; run stops
- VFS snapshot taken before run when DB is attached

---

## LLM Racing

Send the **same prompt** to multiple agents and compare outputs side by side.

### Activation

- **Syntax:** two or more `@tags` at the start, **no** ` | ` in the line:

  ```text
  @gemini-1 @claude-1 write a binary search in Rust
  ```

- **Keys:** **Ctrl+Enter** or **Ctrl+R** with multi-`@` input (sends `UserMessage`; bus starts racing)

Racing is **disabled** if the input contains ` | ` (pipeline syntax wins for detection).

### UI flow (live)

1. Bus emits `RacingStarted`; TUI opens split columns.
2. Outputs stream via racing bus events.
3. **← / →** select a column; **Enter** promotes that output to the main chat.
4. **Esc** discards the racing overlay.

Core takes a VFS snapshot before inject; contestants are injected within 50ms spread (`INJECT_SPREAD_MS`).

---

## VFS undo (time-travel)

Snapshots checkpoint your project tree under `.agenthub_shadow/<uuid>/`.

### When snapshots are created

| Trigger | v0.1 |
|---------|------|
| Manual | `/snapshot` or **F3** |
| LLM race | Automatic when race starts (with DB) |
| Pipeline / spar | Automatic inside `PipelineExecutor` / `SparEngine` when invoked programmatically |

Up to **20** snapshots; oldest pruned.

### Revert

| Action | v0.1 |
|--------|------|
| `/undo` or **Ctrl+Z** | **Not wired** to `revert_latest` from TUI (see [ROADMAP.md](ROADMAP.md)) |
| Programmatic / tests | `VfsEngine::revert_latest` freezes agent PIDs, restores files, optional delete-new-files |

When wired (v0.2), revert will confirm overwrite count, may prompt to delete files created after the snapshot, then resume agents.

---

## Key bindings

| Key | Action |
|-----|--------|
| **F1** | Help overlay (`/help` text) |
| **F2** | Cycle DM → GroupChat → Server (live: `/mode …`) |
| **F3** | `/snapshot` |
| **F5** | Spawn dialog → `/spawn …` |
| **F6** | Agent list (shortcuts for mute/kick/promote) |
| **Ctrl+Z** | `/undo` (v0.2) |
| **Ctrl+R** | LLM Racing (multi-`@` required) |
| **Ctrl+Enter** | LLM Racing |
| **Ctrl+S** | Export chat to file |
| **Ctrl+Q** | Quit (confirm) |
| **Ctrl+L** | Clear input |
| **F4** | Toggle chat scroll (j/k) vs typing in input |
| **j/k**, PgUp/PgDn, **G** | Scroll chat (j/k need F4 or click chat; **G** always jumps to latest) |
| **Ctrl+/** | Search chat history |
| **Tab** | Complete `@tags` and `/commands` (needs ≥2 chars after `/` or `@`) |
| **Esc** | Cancel overlay, racing, search |

Footer: `F1:Help  F4:Scroll  Ctrl+/:Search  F5:Spawn  Ctrl+Z:Undo  Ctrl+R:Race  Esc:Cancel`

---

## Agent sidebar indicators

| Icon | State |
|------|--------|
| ● | Idle, ready |
| ⏳ | Thinking |
| 🔇 | Muted |
| 🔕 | Deafened |
| ⏸ | Timed out / suspended |
| 💀 | Dead / kicked |
| ⚠ | Rate limited |

---

## Driver profiles

Each CLI is described by JSON in `drivers/` or `~/.agenthub/drivers/<name>.json` (filename must match `name`):

| Field | Role |
|-------|------|
| `executable`, `args`, `env` | Launch (`NO_COLOR`, `TERM=dumb`) |
| `prompt_regex` | Turn detection on last line |
| `silence_timeout_ms` | Fallback if prompt never matches |
| `auto_reply_patterns` | Auto-answer interactive prompts |
| `rate_limit_patterns` | Marks agent `RateLimited` |

If an agent never leaves “Thinking”, tune `prompt_regex` or increase `silence_timeout_ms`.

### PTY debug log (opt-in)

Set `"pty_debug_log": true` in `~/.agenthub/config.json` and restart.

- Chunks are **zstd-compressed**
- With DB: `pty_debug_log` table; without DB: `~/.agenthub/debug/{agent_id}/*.zst`
- Entries older than **48 hours** rotated on startup

**Privacy:** may capture prompt-like CLI output. Keep disabled on shared machines.

---

## Troubleshooting

| Symptom | What to check |
|---------|----------------|
| **`unknown command: /undo` or `/spar`** | Expected in v0.1 — see [ROADMAP.md](ROADMAP.md) |
| **Pipeline in chat does nothing** | v0.1: not auto-executed; use tests or wait for v0.2 |
| **Agent stuck “Thinking”** | Wrong `prompt_regex`; inspect real CLI prompt |
| **Rate limit (⚠)** | CLI quota; `rate_limit_patterns` in driver JSON |
| **Spawn fails** | CLI not on `PATH`; driver banned this session; `max_agents` / `max_instances` |
| **`command not found: agenthub`** | [INSTALL.md](INSTALL.md) — PATH / chmod |
| **Revert skipped files** | File locked during revert (warnings in log when revert runs) |
| **Subagent not listed** | Short-lived children may be missed on non-Linux; eBPF ring buffer not implemented (polling is default) |
| **No file context in prompt** | Auto-context not on bus path in v0.1 |

### Developer verification

```bash
# Full core integration (PTY, RBAC, pipeline, VFS)
cargo test -p agenthub-core --features full

# Faster: config + DB
cargo test -p agenthub-core --no-default-features --features config-tests,db-tests

# Workspace (Windows CI skips PTY via AGENTHUB_SKIP_PTY)
cargo test --workspace --all-features
cargo clippy --workspace --all-features -- -D warnings
```

Set `log_level` to `debug` or `trace` in `~/.agenthub/config.json` for sanitizer and subagent traces.

---

## Privacy and network

AgentHub makes **no outbound network calls** and does not collect telemetry. Your CLIs use their own authentication and may contact vendors independently.

---

## See also

- [INSTALL.md](INSTALL.md) — binary install
- [ROADMAP.md](ROADMAP.md) — v0.1 vs v0.2
- [AGENT_MANUAL.md](AGENT_MANUAL.md) — operators and implementers
- [AGENTHUB_BLUEPRINT.md](AGENTHUB_BLUEPRINT.md) — full specification
