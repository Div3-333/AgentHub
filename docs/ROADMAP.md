# Roadmap

High-level product milestones. Phase-level Definition of Done and engineering detail live in [AGENTHUB_BLUEPRINT.md](AGENTHUB_BLUEPRINT.md) Part 18–19.

---

## v0.1 — Shipped (0.1.0)

**Goal:** A credible local orchestrator — real PTYs, typed bus, governance primitives, and a production TUI wired to core — with integration tests as the contract.

### Product

| Area | Status |
|------|--------|
| **Binary** | `agenthub` boots config, SQLite (WAL), `ServerState`, bus router, and `run_with_bridge` TUI ([`crates/agenthub/src/bootstrap.rs`](../crates/agenthub/src/bootstrap.rs)) |
| **Install path** | Build from source; release workflow defined for tagged `v*` binaries ([`.github/workflows/release.yml`](../.github/workflows/release.yml)) |
| **Quickstart** | `agenthub` → `/spawn <driver>` → prompt → sanitized reply (requires CLI on `PATH` and prior login) |
| **Drivers** | Bundled profiles: `gemini`, `claude`, `codex`, `aider`, `cursor` (`drivers/*.json`) |
| **Modes** | DM, Group Chat (default), Server — `/mode` and **F2** update core state when live |
| **Moderation** | `/mute`, `/unmute`, `/deafen`, `/undeafen`, `/timeout`, `/kick`, `/ban`, `/promote`, `/demote`, `/addrole`, `/removerole`, `/spawn`, `/setprompt` |
| **LLM Racing** | Multi-`@` input (no ` \| `) → bus racing registry → parallel PTY inject → split-pane TUI; **Ctrl+R** / **Ctrl+Enter** |
| **VFS snapshot** | `/snapshot`, **F3**; automatic snapshots before races (and inside pipeline/spar executors when used programmatically) |
| **Message bus** | Tag routing, broadcast, staggered inject when multiple agents are thinking, `#channel` filter in Server mode (tag in message body) |
| **Persistence** | Session, agents, messages, pipelines, snapshots in `~/.agenthub/agenthub.db` |
| **Privacy** | No AgentHub outbound network; `pty_debug_log` defaults to **false** |

### Engine (library — `agenthub-core` with `full` feature)

| Subsystem | Status |
|-----------|--------|
| PTY spawn, ring buffer, sanitizer, turn detection | Complete |
| Grand induction + `READY` gate | Complete |
| Pipeline parser + `PipelineExecutor` | Complete (integration tests) |
| Sparring `SparEngine` + `/spar` parser | Complete (integration tests) |
| Auto-context indexer + injector | Complete (unit tests; **not** on live bus path yet) |
| VFS revert (`revert_latest`) | Complete (integration tests) |
| Subagent watcher | Polling fallback active; Linux eBPF loader stub (ring buffer not wired) |

### Quality

| Check | Status |
|-------|--------|
| `cargo test --workspace --all-features` | CI on Linux, macOS, Windows ([`.github/workflows/ci.yml`](../.github/workflows/ci.yml); Windows sets `AGENTHUB_SKIP_PTY=1`) |
| `cargo clippy --workspace --all-features -- -D warnings` | CI |
| Blueprint Part 19 shippable criteria | Met in spec (see blueprint table) |

### Known v0.1 gaps (resolved in v0.2)

See **v0.2 — Shipped** below. Remaining polish:

| Gap | Workaround today |
|-----|------------------|
| **Interactive revert confirm** | `/undo` runs immediately; TUI confirmation UX planned for v0.3 |
| **Channel management UI** | `create_channel` via core API; no `/channel` slash yet |
| **GitHub release assets** | Push `v0.2.0` tag to publish binaries |

---

## v0.2 — Shipped (0.2.0)

**Goal:** Everything advertised in `/help` works from the TUI input line.

### Wired in v0.2

| Feature | Status |
|---------|--------|
| **`/undo` / Ctrl+Z** | `vfs::handle_slash_command` — reverts latest snapshot |
| **`/spar`** | `moderation::execute_command` → `SparEngine` |
| **Frankenstein pipeline** | `@a \| > cmd \| @b` on submit → `PipelineExecutor` via bus router |
| **Auto-context** | `inject_context` on bus before PTY inject (respects `--nocontext`) |
| **`agenthub --version`** | Prints workspace package version |

---

## v0.3 — Planned

### Nice-to-have (post v0.2)

- APFS / Btrfs CoW snapshot fast path
- Additional driver profiles (Copilot CLI, custom community JSON)
- Config hot-reload via `notify`

---

## Phase reference (blueprint)

All 12 implementation phases are marked complete in the blueprint. v0.2 work above is **integration and UX**, not re-litigating phase scope.

| Phase | Scope |
|-------|--------|
| 1–4 | Foundations, PTY, sanitizer, bus |
| 5–6 | TUI, server / RBAC / moderation |
| 7–8 | Pipeline, LLM racing |
| 9–10 | VFS, auto-context |
| 11–12 | Subagent capture, polish & CI |

See [AGENTHUB_BLUEPRINT.md](AGENTHUB_BLUEPRINT.md) Part 18 for per-phase DoD.
