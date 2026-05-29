# Agent Manual

Guide for operators, contributors, and autonomous coding agents extending AgentHub. The **source of truth** for behavior and invariants is [AGENTHUB_BLUEPRINT.md](AGENTHUB_BLUEPRINT.md). End-user prose is in [USER_GUIDE.md](USER_GUIDE.md).

---

## Quick reference

| Task | Command / entry point |
|------|------------------------|
| Run production stack | `cargo run -p agenthub` → [`bootstrap::run`](../crates/agenthub/src/bootstrap.rs) |
| Preview UI only (no core) | `cargo run -p agenthub-tui` or `agenthub_tui::run()` → `App::new_demo` |
| Spawn agent | `pty::spawn_agent` or `/spawn` via `moderation::execute_command` |
| User → PTY path | `BusEvent::UserMessage` → `spawn_bus_router` → `route_message_injection` |
| Racing | `bus::racing::try_dispatch_racing_user_message` |
| Pipeline | `pipeline::PipelineExecutor::execute` |
| Spar | `pipeline::SparEngine::run` + `parse_spar_command` |
| Snapshot | `vfs::create_snapshot_with_config` or `/snapshot` via `vfs::handle_slash_command` |
| Revert | `vfs::revert_latest` / `VfsEngine::revert_latest` |
| Context | `context::inject_context` (not called from router in v0.1) |
| Slash dispatch (TUI) | `tui::events::dispatch_slash_core` → `try_handle_slash_command` then `execute_command` |

---

## Build and verify

```bash
# Full workspace (CI parity)
cargo fmt --all -- --check
cargo clippy --workspace --all-features -- -D warnings
cargo test --workspace --all-features

# Core integration only (PTY, sanitizer, RBAC, pipeline, VFS)
cargo test -p agenthub-core --features full

# Skip real PTY on hosts without mock CLI / Windows CI
AGENTHUB_SKIP_PTY=1 cargo test --workspace --all-features
```

| Feature flag (`agenthub-core`) | Enables |
|-------------------------------|---------|
| `config-tests` | Config load/validate |
| `db-tests` | SQLite schema + `DbClient` |
| `bus-tests` | Bus types + router unit tests |
| `vfs-tests` | Snapshot/revert without full PTY |
| `server-tests` | RBAC/modes unit tests |
| `context-tests` | Indexer/injector compile tests |
| `full` | PTY, sanitizer, pipeline, server integration tests |

Production binary links core with the full orchestration stack via the `agenthub` crate dependency graph (not `no-default-features`).

---

## Workspace layout

```text
crates/agenthub/     # Binary: bootstrap, shutdown, TUI bridge
crates/core/         # agenthub-core — all orchestration
crates/tui/          # agenthub-tui — Ratatui UI
drivers/             # Bundled DriverProfile JSON
tests/fixtures/mock_cli/   # Fake CLI for CI
tests/integration/         # Workspace shims → core/tests
.github/workflows/         # ci.yml, release.yml
```

| Crate | Role |
|-------|------|
| `agenthub` | `AgentHubStack::boot`, `run_tui`, panic-safe `shutdown` + `kill_all_agents` |
| `agenthub-core` | PTY, bus, server, pipeline, vfs, context, db |
| `agenthub-tui` | `App::new_live` / `new_demo`, `CoreBridge`, events Part 15.2 |

---

## Boot sequence (operators)

```text
AgentHubConfig::load()     → ~/.agenthub/config.json (create defaults)
DbClient::init_pool        → ~/.agenthub/agenthub.db + migrations
ServerState::new()         → set_mode from config.default_mode
spawn_bus_router           → broadcast bus + mpsc to TUI
run_with_bridge            → live App + CoreBridge { moderation, db, bus_tx, cwd }
```

On exit: `kill_all_agents`, `db.end_session`, no zombie PTYs (`AgentPty::Drop`, global shutdown hook).

**Do not** document or rely on a root `src/main.rs`; entry is `crates/agenthub/src/main.rs` only.

---

## Engineering invariants (never violate)

1. No heap allocation in PTY read / parser / bus hot paths (ring buffer, pre-sized grids).
2. No `std::sync::Mutex` in async code — `tokio::sync`, `DashMap`, atomics on `AgentPty`.
3. No `unwrap()` / `expect()` in production paths — `AgentHubError` + `Result`.
4. `AgentPty::Drop` and `kill_agent` must terminate children (no zombies).
5. No API keys, telemetry, or outbound HTTP from AgentHub.
6. Durable state types derive `serde::Serialize` / `Deserialize` where persisted.
7. `cargo clippy -- -D warnings` clean before merge.
8. Tests must pass for touched crates; integration tests use `mock_cli`.

---

## Subsystems (implementer map)

### PTY (`core/src/pty/`)

- `spawn_agent`: driver validation, portable-pty, reader task → ring buffer → sanitizer task
- `PtyStatus` atomics; `visible_in_chat` / `receives_broadcast` for mute/deafen
- `subagent_watcher_task`: 250ms polling; `on_subagent_exec` registers `Subagent` role stubs
- `SUBAGENT_CAPTURE_PENDING = false` — polling subagent registration is active; Linux eBPF is optional (`--features ebpf`) and falls back to polling when not loaded

### Sanitizer (`core/src/sanitizer/`)

- `VirtualGrid` + `vte` for spinner-safe text
- Prompt regex + 100ms confirm; silence timeout fallback; `auto_reply_patterns`

### Bus (`core/src/bus/`)

- Capacity `BUS_CHANNEL_CAPACITY` (1024)
- `route_message_injection`: UserMessage → racing OR mention/broadcast + channel filter
- Injection format: `[{tag} says]: {content}\n`
- Chaos stagger when multiple agents `Thinking`

### Server (`core/src/server/`)

- `rbac.rs`: `Permissions` bitflags, built-in roles, `~/.agenthub/roles.json`
- `moderation.rs`: slash commands (no `/spar`, `/undo`, `/snapshot` — snapshot via vfs)
- `induction.rs`: template + `READY` timeout
- `modes.rs`: DM / GroupChat / Server, `create_channel`, `parse_channel_tag`

### Pipeline (`core/src/pipeline/` — feature `full`)

- `parser::parse`: split on ` | `
- `PipelineExecutor::execute`: snapshot, sequential stages, 5min agent / 60s unix timeouts
- `SparEngine`, `parse_spar_command`, `SPAR_ABORT`

### VFS (`core/src/vfs/` — feature `vfs-tests` or `full`)

- Blake3 manifest, jwalk copy, exclude `.git`, `target`, `.agenthub_shadow`, etc.
- `handle_slash_command`: **only** `/snapshot` today
- `revert_latest`: freeze PIDs, atomic rename restore

### Context (`core/src/context/`)

- `AstIndexer` + tree-sitter; `inject_context` / `--nocontext`
- **Integration gap:** not invoked from `route_message_injection` (v0.2)

### TUI (`crates/tui/`)

- `on_bus_event`: agents, chat, racing, pipeline viz from bus
- `on_submit`: slash → `route_slash_command`; else `UserMessage` on bus
- `dispatch_slash_core`: vfs first, then moderation

---

## v0.1 integration gaps (do not claim shipped)

When updating docs or marketing, treat these as **v0.2** unless code changes:

| Item | Code location | Fix |
|------|---------------|-----|
| `/undo` | `moderation::execute_command` unknown | Extend `vfs::handle_slash_command` or moderation delegate |
| `/spar` | Same | Call `SparEngine::run` from async slash handler |
| Chat pipeline | `app::on_submit` | Detect `parse()` ok + ` \| ` → spawn `PipelineExecutor` |
| Auto-context | `bus/router.rs` | Call `inject_context` before inject |
| Channel UX | `modes::create_channel` | Optional slash + TUI |

---

## Adding a driver profile

1. Copy `drivers/gemini.json` → `~/.agenthub/drivers/mycli.json`.
2. Set `"name": "mycli"` (must match filename stem).
3. Set `executable`, `prompt_regex` (test against real CLI prompt line), `silence_timeout_ms`.
4. Add `rate_limit_patterns` and `auto_reply_patterns` from observed CLI behavior.
5. Validate: `cargo test -p agenthub-core --no-default-features --features config-tests config`.

Spawn: `/spawn mycli` or `spawn_agent("mycli", ...)`.

---

## Database

- Migration: `crates/core/src/db/migrations/001_initial_schema.sql`
- WAL pragmas in `DbClient::apply_pragmas`
- Primary writer path: bus router `log_bus_event`
- Tables: `sessions`, `agents`, `messages`, `pipelines`, `pipeline_stages`, `snapshots`, `snapshot_files`, `custom_roles`, `pty_debug_log`

---

## Testing strategy

| Test file | Requires | Covers |
|-----------|----------|--------|
| `pty_lifecycle` | `full` | spawn, inject, kill |
| `stream_sanitizer` | `full` | grid, turn detection |
| `rbac_moderation` | `full` | slash commands, induction |
| `pipeline_frankenstein` | `full` | pipeline + spar |
| `vfs_snapshot_revert` | `vfs-tests` | snapshot + revert |
| `bus_routing` | `bus-tests` | routing helpers |

Use `mock_cli` env vars (`MOCK_CLI_PROMPT`, `MOCK_CLI_LATENCY_MS`, etc.) per blueprint Part 17.

---

## CI / release

- **ci.yml:** ubuntu + macos + windows (`AGENTHUB_SKIP_PTY=1` on Windows), fmt, clippy `-D warnings`, test `--all-features`, release build
- **release.yml:** tagged `v*` → per-triple binaries (`agenthub-linux-x86_64`, …)

---

## Phase gates (blueprint Part 18)

All 12 phases are marked complete in the blueprint. New work should close **v0.2 integration gaps** above without breaking Part 0.3 invariants.

Part 19 “shippable” checklist: see blueprint table — criteria 1–10 documented as met for the engine; user-facing gap list is in [ROADMAP.md](ROADMAP.md).

---

## Related documents

- [USER_GUIDE.md](USER_GUIDE.md) — operators using the TUI
- [ROADMAP.md](ROADMAP.md) — v0.1 shipped / v0.2 planned
- [ARCHITECTURE.md](../about/ARCHITECTURE.md) — runtime diagram
- [VISION.md](../about/VISION.md) — product narrative
