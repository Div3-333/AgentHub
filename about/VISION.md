# Vision

## What we are building

AgentHub is a **Phantom Terminal Orchestrator**: a single Rust-native terminal application that unifies every free-tier AI CLI you already use—Gemini CLI, Claude Code, Codex CLI, Aider, Cursor CLI, GitHub Copilot CLI, and future tools that ship a **Driver Profile** JSON.

You stay in one surface. Agents run as real child processes in hidden pseudo-terminals. Prompts go in as keystrokes; answers come back as sanitized text on a shared message bus. No API keys inside AgentHub. No subscriptions to AgentHub. No telemetry from AgentHub.

## The problem

Developers today open five or six terminal tabs, each with a different CLI, each with different spinner noise, rate-limit wording, and interactive prompts. Context does not flow between tools. Nobody has a consistent way to run “Gemini drafts, Claude reviews, shell checks” as a governed workflow. Multi-agent products often assume cloud APIs; power users already live in the terminal.

## The solution

AgentHub treats the terminal as the integration layer:

1. **Spawn** — Driver profiles describe how to launch each CLI and detect when it is ready for input.
2. **Sanitize** — A virtual terminal grid strips ANSI spinners and detects turn boundaries without an API.
3. **Route** — A central bus delivers messages by tag, mode, channel, and permission.
4. **Recover** — VFS snapshots let you undo agent-driven file changes (revert from the TUI completes in v0.2).
5. **Compare** — LLM Racing sends one prompt to many agents and lets you pick the winner in split panes.

The experience is intentionally **Discord-like**: you are always the server admin; agents are members with roles, tags, and moderation commands.

## Workspace modes (product metaphor)

| Mode | User story |
|------|------------|
| **DM** | One model, zero ceremony. |
| **Group Chat** | Several agents in one room, brainstorming. |
| **Server** | Roles, permissions, `#channel` routing, and safety rails for structured work. |

Modes change broadcast rules, RBAC enforcement, and spawn limits—not just labels.

## Principles

1. **Local-first** — SQLite history, shadow snapshots, and config on your machine. AgentHub does not operate a cloud control plane.
2. **CLI-native** — We orchestrate terminal processes, not REST shims.
3. **Deterministic** — Typed `BusEvent`s, logged transitions, reproducible tests with `mock_cli`.
4. **Extensible** — New CLIs are JSON profiles; custom roles live beside config.
5. **Honest scope** — PTY children are not a security sandbox; snapshots and RBAC are the safety net.

## Who it is for

- **Individual developers** who already authenticate via vendor CLIs and want one control room.
- **Power users** running pipelines (`@agent | > cargo | @agent`), racing, and (soon) sparring from the same UI.
- **Contributors** implementing against a sealed blueprint and integration tests.

## What success looks like

**v0.1 (shipped):** Production `agenthub` binary, live TUI, PTY orchestration, moderation, LLM racing, snapshots, and a complete core library (pipeline, spar, context, VFS revert) validated by tests. Some `/help` items (undo, spar, chat pipelines, auto-context) land in **v0.2**.

**v0.2:** Every advertised slash command and chat syntax works without reading source; published release binaries; optional eBPF subagent on Linux.

**Longer term:** CoW snapshot backends (APFS/Btrfs), richer driver ecosystem, optional PTY debug capture for profile authors only.

## What we refuse to become

- An API aggregator competing on model pricing
- A web or Electron app pretending to be a terminal
- A `tmux` layout with no semantic routing
- A black box that phones home

## Documentation map

| Document | Audience |
|----------|----------|
| [docs/AGENTHUB_BLUEPRINT.md](../docs/AGENTHUB_BLUEPRINT.md) | Complete engineering specification |
| [docs/USER_GUIDE.md](../docs/USER_GUIDE.md) | Daily TUI usage |
| [docs/AGENT_MANUAL.md](../docs/AGENT_MANUAL.md) | Operators and implementers |
| [docs/ROADMAP.md](../docs/ROADMAP.md) | v0.1 vs v0.2 |
| [about/ARCHITECTURE.md](ARCHITECTURE.md) | Runtime structure |
