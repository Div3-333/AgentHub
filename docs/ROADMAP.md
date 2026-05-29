# The "Phantom Terminal" Implementation Roadmap

This roadmap details the engineering required to build a world-class CLI wrapper and orchestrator without relying on APIs.

## Phase 1: Foundations & The PTY Engine
- [x] Initial project scaffolding (Rust + Ratatui).
- [ ] **Pseudo-Terminal (PTY) Integration:**
    - [ ] Integrate `portable-pty` or similar crate.
    - [ ] Implement `PtyProcess` wrapper: Spawn an arbitrary shell command (e.g., `cmd.exe /c gemini`) inside a hidden PTY.
    - [ ] Implement asynchronous, non-blocking readers for the PTY output stream.
    - [ ] Implement robust `stdin` writing (sending prompts as if typed by a keyboard, followed by `\n` or `\r\n`).

## Phase 2: Stream Sanitization & Parsing
- [ ] **The ANSI Stripper:**
    - [ ] Implement a robust state-machine parser to strip ANSI escape codes (colors, cursor movements).
    - [ ] Handle terminal bell (`\x07`) and backspace (`\x08`) characters properly.
- [ ] **Prompt Detection:**
    - [ ] Implement regex-based detection to know when an agent has finished its turn. (e.g., waiting for the `>` or `User:` prompt to appear in the stream).
    - [ ] Build a buffering system that accumulates chunks of text until the "ready" prompt is detected, then fires a `MessageComplete` event.

## Phase 3: Driver Profiles & The Orchestrator
- [ ] **Driver Configuration:**
    - [ ] Define the `AgentDriver` schema (Path to executable, startup flags, prompt regex).
    - [ ] Create pre-configured profiles for popular free CLIs (Gemini, Claude, Cursor, Aider).
- [ ] **The Message Bus:**
    - [ ] Implement the central `tokio` event bus.
    - [ ] Tag routing: If user types `@gemini hello`, route `hello` exclusively to the Gemini PTY's `stdin`.

## Phase 4: The Unified TUI (Group Chat)
- [ ] **Chat Interface:**
    - [ ] Scrollable message history distinguishing between User, Agent A, and Agent B.
    - [ ] Auto-scrolling as text streams in from the PTYs.
- [ ] **Agent Status Dashboard:**
    - [ ] Sidebar showing active agents.
    - [ ] State indicators: "Idle" (prompt detected), "Typing" (stream active), "Offline" (process died).

## Phase 5: Pipeline & Collaboration Logic
- [ ] **Sequential Handoffs:**
    - [ ] Logic to pipe outputs: Trigger Agent B automatically when Agent A finishes, injecting Agent A's final text into Agent B's `stdin`.
- [ ] **Autonomous Agent Loops (The "Sparring" Match):**
    - [ ] Setup infinite or semi-infinite loops where Agent A and Agent B continuously prompt each other (e.g., Coder vs. Reviewer).
    - [ ] Inject wrapper prompts during handoff (e.g., "Agent B said: [Output]. Please review this and reply.").
    - [ ] **Safety Rails:** Implement a `max_turns` limit (e.g., stop after 5 back-and-forths) and a global `Escape` hotkey to prevent them from getting stuck in an infinite "Thank you!" loop.
- [ ] **Context Injection:**
    - [ ] Mechanism to prepend recent chat history to a prompt before sending it to an agent, ensuring they are aware of the "group chat" context even though they are isolated processes.

## Phase 6: Robustness & Edge Cases
- [ ] **Process Lifecycle Management:**
    - [ ] Ensure all child PTY processes are forcefully killed (`SIGKILL` / `taskkill`) when AgentHub closes to prevent orphaned background processes.
- [ ] **Timeout & Hang Recovery:**
    - [ ] If an agent streams nothing for X seconds, assume it crashed, kill the PTY, restart it, and notify the user.
- [ ] **Interactive Bypasses:**
    - [ ] Logic to auto-answer `[Y/n]` prompts that CLIs sometimes throw unexpectedly.