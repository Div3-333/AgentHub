# Architecture

AgentHub is a Cargo workspace: one binary crate, two libraries, integration tests, and bundled driver JSON. This document explains how components connect at runtime; module-level specs are in [docs/AGENTHUB_BLUEPRINT.md](../docs/AGENTHUB_BLUEPRINT.md).

---

## Workspace layout

```text
agenthub/
├── crates/agenthub/          # Binary: bootstrap → TUI
├── crates/core/              # agenthub-core — orchestration
├── crates/tui/               # agenthub-tui — Ratatui front end
├── drivers/                  # Bundled DriverProfile JSON
├── tests/fixtures/mock_cli/  # CI fake CLI
└── tests/integration/        # Workspace test entrypoints
```

| Crate | Depends on | Role |
|-------|------------|------|
| `agenthub` | `agenthub-tui`, `agenthub-core` | Config, DB, bus, `CoreBridge`, shutdown |
| `agenthub-tui` | `agenthub-core` | Input, layout, overlays, bus consumer |
| `agenthub-core` | — | PTY, bus, server, pipeline, VFS, context, db |

Core uses **feature flags** so CI can test config, DB, bus, and VFS without compiling the full PTY stack. The production binary uses the full dependency graph. See [AGENT_MANUAL.md](../docs/AGENT_MANUAL.md) for flags.

---

## Runtime data flow

```text
                    ┌──────────────────────────────────────┐
                    │           agenthub-tui               │
                    │  chat · input · sidebar · racing     │
                    └───────────────┬──────────────────────┘
                                    │ BusEvent (mpsc)
                                    ▼
┌──────────────┐    broadcast     ┌────────────────────────────┐
│  Producers   │ ───────────────► │      spawn_bus_router      │
│  sanitizer   │                  │  log → SQLite              │
│  moderation  │                  │  racing registry           │
│  (future:    │                  │  route_message_injection   │
│   pipeline)  │                  └─────────────┬──────────────┘
└──────────────┘                                │ stdin inject
                                                ▼
                              ┌─────────────────────────────────┐
                              │  ServerState.agents (DashMap)   │
                              │  each AgentPty + PtyRingBuffer  │
                              └─────────────────────────────────┘
                                                │
                    ┌───────────────────────────┴───────────────────────────┐
                    ▼                                                       ▼
            ┌───────────────┐                                       ┌───────────────┐
            │ pty_reader    │ ──bytes──► ring buffer ──► sanitizer │ child CLI     │
            └───────────────┘                                       └───────────────┘
```

### Production boot (`agenthub` binary)

1. `AgentHubStack::boot` — config, DB migrate, `ServerState`, `spawn_bus_router`.
2. `run_with_bridge` — `App::new_live` + `CoreBridge` (moderation, db, `bus_tx`, cwd).
3. Exit — `kill_all_agents`, `end_session`.

### Happy path: user message

1. TUI `on_submit` → `BusEvent::UserMessage` on broadcast bus (unless line starts with `/`).
2. Router logs event, forwards clone to TUI `mpsc`.
3. If `is_racing_input` (≥2 `@tags`, no ` | `) → `RacingRegistry` snapshot + parallel inject + `RacingStarted`.
4. Else resolve recipients (mention, broadcast, `#channel` in Server, mute/deafen).
5. `format_injection` → PTY stdin (staggered if chaos heuristics apply).
6. Sanitizer emits `AgentMessage` on turn complete → broadcast to peers per RBAC.

**Not on this path in v0.1:** `inject_context`, `PipelineExecutor::execute` (v0.2).

---

## Core subsystems

### PTY engine (`core/src/pty/`)

| Piece | Function |
|-------|----------|
| `manager.rs` | `spawn_agent`, status atomics, `Drop` kill escalation |
| `io.rs` | 64 KiB `PtyRingBuffer` (SPSC) |
| `subagent.rs` | 250ms polling watcher; stub registration for child PIDs; eBPF load stub on Linux |

Environment at spawn: `TERM=dumb`, `NO_COLOR=1`, `AGENTHUB=1`.

### Stream sanitizer (`core/src/sanitizer/`)

| Piece | Function |
|-------|----------|
| `parser.rs` | `VirtualGrid` + `vte::Perform` |
| `heuristic.rs` | Prompt regex, silence timeout, auto-reply |

### Message bus (`core/src/bus/`)

| Module | Function |
|--------|----------|
| `event.rs` | Serializable `BusEvent` |
| `routing.rs` | Recipients, injection format, stagger |
| `racing.rs` | Multi-`@` sessions, snapshot, DB rows |
| `router.rs` | Central loop |

Channel capacity: **1024** (`BUS_CHANNEL_CAPACITY`).

### Server (`core/src/server/`)

| Module | Function |
|--------|----------|
| `rbac.rs` | `Permissions`, built-in + custom roles |
| `moderation.rs` | Slash commands (spawn, mute, mode, …) |
| `modes.rs` | DM / GroupChat / Server, `channels` DashMap |
| `induction.rs` | Grand induction + `READY` |
| `state.rs` | `ServerState` |

### Pipeline (`core/src/pipeline/` — `full`)

| Module | Function |
|--------|----------|
| `parser.rs` | Frankenstein ` | ` stages |
| `executor.rs` | Sequential agent + unix stages |
| `loop_engine.rs` | `SparEngine`, `parse_spar_command` |

Library-complete; **chat auto-trigger v0.2**.

### VFS (`core/src/vfs/`)

| Module | Function |
|--------|----------|
| `snapshot.rs` | Blake3 manifest, copy, 20-snapshot cap; `/snapshot` slash |
| `revert.rs` | `revert_latest`, PID freeze — **slash/TUI v0.2** |

Shadow root: `config.shadow_dir` (default `.agenthub_shadow/` relative to CWD).

### Context (`core/src/context/`)

| Module | Function |
|--------|----------|
| `indexer.rs` | Tree-sitter + file list |
| `injector.rs` | Filename/symbol prepend, `--nocontext` |

Always compiled; **router hookup v0.2**.

### Database (`core/src/db/`)

SQLx SQLite, embedded `001_initial_schema.sql`, WAL. Bus router is the primary writer for chat and lifecycle events.

---

## TUI architecture (`crates/tui/`)

| Module | Function |
|--------|----------|
| `app.rs` | `App`, `on_bus_event`, `CoreBridge`, racing overlay |
| `events.rs` | Keybindings Part 15.2, `dispatch_slash_core` |
| `components/chat.rs` | Scrollable history |
| `components/input.rs` | Input + Tab completion |
| `components/sidebar.rs` | Agents, mode, snapshots, pipeline block |
| `components/racing.rs` | Multi-column racing |
| `components/pipeline_viz.rs` | Stage progress from bus pipeline events |
| `theme.rs` | Dark theme |

| Entry | Use |
|-------|-----|
| `run_with_bridge` | Production (`agenthub` binary) |
| `run()` / `new_demo` | UI preview without core (developers only) |

Slash dispatch order: `vfs::handle_slash_command` (`/snapshot`) → `moderation::execute_command`.

---

## Configuration and drivers

```text
~/.agenthub/config.json     → AgentHubConfig
~/.agenthub/drivers/*.json  → overrides
repo/drivers/*.json         → bundled fallback
```

`AgentHubConfig::load()` creates defaults on first access. Driver `name` must match JSON filename stem.

---

## Cross-cutting concerns

### Concurrency

- Tokio: bus router, PTY reader (blocking read offloaded per blueprint).
- `DashMap` for agents; atomics on `AgentPty`.
- Racing: `Arc<DashMap>` for per-contestant buffers.

### Error model

`AgentHubError` in `core/src/error.rs` with `u8` exit mapping for CLI embedding.

### Security posture

- No AgentHub-initiated network
- RBAC / mute / deafen are session policy, not OS sandboxing
- VFS revert is the primary undo for agent file damage
- Rate-limit detection surfaces vendor strings; no automatic retry

### Platform notes

| Area | Linux | macOS | Windows |
|------|-------|-------|---------|
| PTY | portable-pty | portable-pty | portable-pty |
| Timeout/kick | SIGSTOP / SIGTERM | SIGSTOP / SIGTERM | SuspendThread / kill APIs |
| Subagent | Polling + eBPF stub | Polling | Polling |
| VFS freeze | SIGSTOP | SIGSTOP | Best-effort (see revert logs) |

---

## CI and release

```text
.github/workflows/ci.yml       → fmt, clippy -D warnings, test --all-features (3 OS)
.github/workflows/release.yml → v* tag → per-target binaries
```

Integration tests use `mock_cli` — no vendor CLIs required in CI.

---

## v0.1 → v0.2 integration surface

| Gap | User impact |
|-----|-------------|
| `/undo`, Ctrl+Z | Revert API exists; slash not delegated |
| `/spar` | Engine tested; not in slash dispatch |
| Pipeline in chat | Executor tested; submit sends plain `UserMessage` |
| Auto-context | Injector tested; not before PTY write |
| Channel slash UI | `create_channel` in core only |

Details: [ROADMAP.md](../docs/ROADMAP.md).

---

## Related documents

- [VISION.md](VISION.md) — product narrative
- [USER_GUIDE.md](../docs/USER_GUIDE.md) — commands and keys
- [AGENT_MANUAL.md](../docs/AGENT_MANUAL.md) — operate and extend
- [AGENTHUB_BLUEPRINT.md](../docs/AGENTHUB_BLUEPRINT.md) — full specification
