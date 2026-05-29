# Installing AgentHub

AgentHub **v0.1.0** ships as a single static binary per platform. You do not need Rust or Cargo to run it.

## Prerequisites

- A terminal (Windows Terminal, iTerm2, GNOME Terminal, etc.)
- At least one supported AI **CLI** installed and logged in on your machine (for example [Gemini CLI](https://github.com/google-gemini/gemini-cli))
- AgentHub does **not** store API keys; each CLI uses its own authentication

## Download

Open [GitHub Releases](https://github.com/Div3-333/AgentHub/releases) and pick the asset for your OS (built when a maintainer pushes a `v*` tag, e.g. `v0.1.0`):

| Platform | Download asset |
|----------|----------------|
| Linux (x86_64) | `agenthub-linux-x86_64` |
| macOS (Intel) | `agenthub-macos-x86_64` |
| macOS (Apple Silicon) | `agenthub-macos-arm64` |
| Windows (x86_64) | `agenthub-windows-x86_64.exe` |

## Install steps

### Linux

```bash
chmod +x agenthub-linux-x86_64
sudo mv agenthub-linux-x86_64 /usr/local/bin/agenthub
# or: mkdir -p ~/.local/bin && mv agenthub-linux-x86_64 ~/.local/bin/agenthub
```

Ensure `~/.local/bin` is on your `PATH` if you use a user-local install.

### macOS

```bash
chmod +x agenthub-macos-arm64   # or agenthub-macos-x86_64 on Intel
sudo mv agenthub-macos-arm64 /usr/local/bin/agenthub
```

On first launch, if Gatekeeper blocks the binary: **System Settings → Privacy & Security → Open Anyway**, or `xattr -dr com.apple.quarantine /usr/local/bin/agenthub`.

### Windows

1. Download `agenthub-windows-x86_64.exe`.
2. Move it to a folder on your `PATH` (for example `%USERPROFILE%\.local\bin`) or run it from the download folder.
3. Optionally rename to `agenthub.exe`.
4. If SmartScreen warns, choose **More info → Run anyway** (unsigned community build).

Verify the binary runs:

```powershell
agenthub
```

The TUI should start (a `--version` flag may be added in a later release).

## First run

1. Open a terminal in your **project directory** (where you want agents to work).
2. Run `agenthub`.
3. AgentHub creates local state under `.agenthub_shadow/` in that directory (SQLite history, snapshots). Data stays on your machine; AgentHub itself makes no outbound network calls.
4. Spawn an agent, for example `/spawn gemini` (requires [Gemini CLI](https://github.com/google-gemini/gemini-cli) installed and authenticated).
5. Type a prompt and press **Enter** — input is injected as keystrokes into the agent PTY.
6. Type `/help` for slash commands. Press **Ctrl+Q** twice to quit and tear down agent processes.

## 60-second quickstart

```text
cd your-project/
agenthub
/spawn gemini
<type your prompt, Enter>
```

See [README.md](../README.md#60-second-quickstart) for the full walkthrough.

## Build from source (developers)

See [README.md](../README.md#build-from-source-developers) if you are hacking on the Rust workspace.

## Troubleshooting

| Issue | What to try |
|-------|-------------|
| `agenthub: command not found` | Add the install directory to `PATH`; open a new terminal |
| Spawn fails for `gemini` | Install the Gemini CLI and run its login flow in a normal shell first |
| Permission denied (Linux/macOS) | `chmod +x` on the downloaded binary |
| macOS quarantine | `xattr -dr com.apple.quarantine /path/to/agenthub` |

For day-to-day TUI usage, see [USER_GUIDE.md](USER_GUIDE.md).
