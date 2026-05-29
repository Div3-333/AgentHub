# AgentHub

**Phantom Terminal Orchestrator** — a Rust-native, terminal-based control surface for multiple free-tier AI CLIs, with no API wrappers and no subscriptions.

AgentHub spawns each supported CLI (Gemini CLI, Claude Code, Codex CLI, Aider, Cursor CLI, GitHub Copilot CLI, and others conforming to a bundled **Driver Profile** JSON under `drivers/`) inside a hidden pseudo-terminal (PTY), injects prompts as keystrokes, sanitizes output, and routes everything through a central message bus into one unified terminal UI.

## What it is

| Property | Description |
|----------|-------------|
| **Orchestration model** | PTY multi-CLI — real terminal child processes, not HTTP clients |
| **UX metaphor** | Discord-like — you are the server admin; agents are members |
| **Runtime** | Terminal only (Ratatui); no web app, no Electron |
| **Network** | AgentHub itself makes no outbound calls; CLIs use their own auth |

## What it is not

- **Not an API wrapper** — no Anthropic, Google, or OpenAI HTTP clients in AgentHub
- **Not a tmux wrapper** — I/O is parsed, tagged, sanitized, and routed semantically
- **Not non-deterministic** — state transitions are typed, logged, and recoverable

## Discord-like modes

Three workspace modes control how agents interact (full behavior is specified in the blueprint):

| Mode | Purpose |
|------|---------|
| **DM** | One human, one agent — direct, minimal ceremony |
| **Group Chat** | Multiple agents in one channel with lightweight coordination |
| **Server** | Full RBAC, moderation, induction, and governance |

## How it works (high level)

```text
┌─────────────┐     keystrokes      ┌──────────────┐
│  TUI (you)  │ ──────────────────► │  PTY manager │
└─────────────┘                     │  (per agent) │
       ▲                            └──────┬───────┘
       │ sanitized text                     │ raw PTY I/O
       │                            ┌───────▼───────┐
       └────────────────────────────│  Message bus  │
                                    └───────────────┘
```

1. **Spawn** — Driver profiles (`drivers/*.json`) define how to launch and prompt each CLI.
2. **Inject** — User input is sent as terminal input to the correct PTY.
3. **Sanitize** — ANSI/grid parsing and heuristics strip spinner noise and detect turn boundaries.
4. **Route** — Tags and channels deliver messages to the right pane and persistence layer.

Local state (snapshots, SQLite history) lives under `.agenthub_shadow/` in the project directory by default. See [docs/AGENTHUB_BLUEPRINT.md](docs/AGENTHUB_BLUEPRINT.md) for the full specification.

## Install (binary — no Rust required)

**Version 0.1.0** — download one executable from [GitHub Releases](https://github.com/Div3-333/AgentHub/releases). Assets are published when a maintainer pushes a `v*` tag (for example `v0.1.0`):

| Platform | Asset name |
|----------|------------|
| Linux x86_64 | `agenthub-linux-x86_64` |
| macOS Intel | `agenthub-macos-x86_64` |
| macOS Apple Silicon | `agenthub-macos-arm64` |
| Windows x86_64 | `agenthub-windows-x86_64.exe` |

Put the binary on your `PATH` (or run it from the download folder). Platform-specific install steps (permissions, Gatekeeper, SmartScreen) are in [docs/INSTALL.md](docs/INSTALL.md).

**You need:** a terminal and at least one supported AI CLI installed and authenticated (AgentHub does not store API keys).

## First run

1. `cd` into the project folder where you want agents to work.
2. Run `agenthub` — the TUI opens and creates `.agenthub_shadow/` for local history and snapshots.
3. Spawn a driver with `/spawn <name>` (for example `/spawn gemini` using `drivers/gemini.json`).
4. Type your prompt and press **Enter** — keystrokes go to the agent PTY; the sanitized reply appears in the chat pane.
5. `/help` lists slash commands; **Ctrl+Q** twice quits and tears down all agent processes.

## 60-second quickstart

From your project folder, with [Gemini CLI](https://github.com/google-gemini/gemini-cli) already installed and logged in:

```text
1. agenthub                          # start the TUI
2. /spawn gemini                     # spawn Gemini in a PTY (driver: drivers/gemini.json)
3. Type your prompt, press Enter     # injected as keystrokes to the agent
4. Wait for the sanitized reply      # appears in the chat pane
```

Type `/help` for slash commands. Press **Ctrl+Q** twice to quit and tear down agent processes.

## Build from source (developers)

Requires the [Rust](https://rustup.rs/) toolchain (edition 2021; stable recommended).

```bash
cargo build -p agenthub --release
cargo run -p agenthub --release
```

Development build:

```bash
cargo run -p agenthub
```

Workspace checks:

```bash
cargo test --workspace
cargo clippy --workspace -- -D warnings
```

The canonical binary crate is [`crates/agenthub`](crates/agenthub/). Do not use a legacy root `src/main.rs` — it has been removed in favor of the workspace layout in [Part 1 of the blueprint](docs/AGENTHUB_BLUEPRINT.md).

## Repository layout

```text
agenthub/
├── crates/
│   ├── agenthub/   # Binary entry point
│   ├── core/       # PTY, bus, server modes, pipeline, VFS, DB
│   └── tui/        # Terminal UI (per blueprint phases)
├── drivers/        # Bundled CLI driver profiles (JSON; per blueprint Part 1)
├── docs/           # Blueprint, agent manual, user guide
└── tests/          # Integration tests and mock CLI fixtures
```

## Documentation

| Document | Audience |
|----------|----------|
| [docs/INSTALL.md](docs/INSTALL.md) | Binary install and first-run setup |
| [docs/AGENTHUB_BLUEPRINT.md](docs/AGENTHUB_BLUEPRINT.md) | Complete engineering spec (source of truth) |
| [docs/AGENT_MANUAL.md](docs/AGENT_MANUAL.md) | Autonomous implementers and coding agents |
| [docs/USER_GUIDE.md](docs/USER_GUIDE.md) | End users operating the TUI |

## License

MIT — see workspace `Cargo.toml` for package metadata (version **0.1.0**).
