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

### Known v0.1 gaps (documented, not hidden)

These appear in `/help` or the blueprint but are **not** fully wired from the TUI chat line yet:

| Gap | Workaround today |
|-----|------------------|
| **`/undo` / Ctrl+Z** | `VfsEngine::revert_latest` works in tests; slash dispatch only handles `/snapshot` via `vfs::handle_slash_command` — `/undo` returns “unknown command” until v0.2 |
| **`/spar`** | Run via `SparEngine` in tests or embed in bootstrap; not in `execute_command` |
| **Frankenstein pipeline in chat** | Use `PipelineExecutor::execute` from code/tests; typing `@a \| > cmd \| @b` sends a normal `UserMessage`, not a pipeline run |
| **Auto-context on send** | Call `context::inject_context` explicitly; bus router does not prepend file/symbol snippets yet |
| **Channel management UI** | `create_channel` / assign agents via core API; no `/channel` slash command |
| **GitHub release assets** | Appear when a maintainer pushes a `v*` tag |

---

## v0.2 — Planned

**Goal:** End-user polish — everything advertised in `/help` works from the TUI without reading source, plus optional power features.

### User-facing

- [ ] Wire **`/undo`** and **Ctrl+Z** through `vfs::handle_slash_command` (or moderation delegate) with confirmation UX
- [ ] Wire **`/spar`** to `SparEngine` from slash dispatch; **Esc** → `SPAR_ABORT` (already in core)
- [ ] Detect **` | `** pipeline syntax on submit; run `PipelineExecutor` and drive sidebar `pipeline_viz` from bus events
- [ ] Hook **`context::inject_context`** in the bus path before PTY inject (respect `--nocontext`)
- [ ] Interactive revert: confirm overwrite / delete-new-files prompts in TUI (core messages exist)
- [ ] Publish **v0.2.0** release binaries for all `release.yml` targets; verify [INSTALL.md](INSTALL.md) on clean machines
- [ ] Optional: slash commands for **channels** (`/channel create`, assign agent) in Server mode

### Platform & ops

- [ ] Linux **eBPF** subagent ring-buffer drain (polling remains fallback)
- [ ] Harden Windows freeze/resume paths for VFS revert where stubs still log warnings
- [ ] `agenthub --version` and richer first-run banner (driver detection hints)

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
