# AgentHub — Final Ship Horde (complete)

**Goal:** Shippable per Blueprint Part 19 + Phases 11–12 + Parts 16–17.

## Horde results (14 agents, parallel)

| Agent | Scope |
|-------|--------|
| Bootstrap | `main` daemon: config, DB, bus, CoreBridge, shutdown |
| TUI-live | **Done** — `run_with_bridge` is production; `run()` / `new_demo` is dev preview only |
| Subagent-11 | Phase 11: subagent capture (poll + stub eBPF path) |
| Polish-12 | Orphan reap, SIGTERM, resize, keybinding E2E |
| CI-16 | `.github/workflows/ci.yml` green 3 OS |
| Release-16 | `release.yml` + README binary install |
| Integrate-17 | All integration tests + workspace wiring |
| Docs | USER_GUIDE, ROADMAP, AGENT_MANUAL depth |
| Pipeline-viz | Full `pipeline_viz.rs` |
| E2E-smoke | End-to-end bootstrap integration test |
| Windows-pty | Timeout/suspend/kick on Windows |
| Pty-debug | Opt-in zstd PTY debug log |
| Help-slash | `/help` + command catalog in TUI |
| Seal-audit | Part 19 checklist + fix stragglers |

**Lead verify (Windows, `AGENTHUB_SKIP_PTY=1`):** `cargo test --workspace --all-features` ✅ · `cargo clippy --workspace --all-features -- -D warnings` ✅

**Remaining for maintainer:** push `v0.1.0` tag → confirm GitHub Actions green on 3 OS → attach release binaries.

**Honest v0.1 scope:** see `docs/ROADMAP.md` — `/undo`, `/spar`, chat pipelines, auto-context inject are v0.2 (library ready, TUI slash path not wired).
