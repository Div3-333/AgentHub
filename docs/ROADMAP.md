# AgentHub: Zero-Debt Production Roadmap (Shippable & Enterprise-Ready)

This document is the absolute source of truth for "Definition of Done." Completion of all points guarantees a product ready for immediate public launch, signup waitlists, and enterprise deployment.

## Phase 1: Foundations & Core Logic
- [x] **Project Vision & Strategy:** (COMPLETE) Initial architecture and goals documented.
- [x] **Project Scaffolding:** (COMPLETE) Rust + Cargo + Ratatui + Tokio setup.
- [ ] **Agent Abstraction Layer (The "Adapter" Trait)**
    - [ ] `trait AgentAdapter`: Implement async `send_prompt`, `stream_response`, and `terminate`.
    - [ ] **Stream Handling:** Multi-threaded non-blocking capture of `stdout` and `stderr` using `tokio::sync::mpsc`.
    - [ ] **ANSI Sanitization:** 100% removal of non-deterministic ANSI sequences (loading bars, moving cursors) using a state-machine parser.
    - [ ] **Process Lifecycle:** Automatic PID tracking and guaranteed cleanup on app exit (no "zombie" processes).
- [ ] **The "Omnibus" Message Router**
    - [ ] **Atomic Message Bus:** Ensure no race conditions when multiple agents stream simultaneously.
    - [ ] **Tag Parser:** Regex-based `@agent` extraction with support for aliases.
    - [ ] **History Store:** Thread-safe, indexed conversation history with O(1) lookups for the last `N` messages.

## Phase 2: Production-Grade UI/UX
- [ ] **Robust Terminal Interface**
    - [ ] **Resize Resilience:** UI must not crash or distort on terminal resize (handled via `Terminal::draw` loops).
    - [ ] **Word Wrapping:** Intelligent word wrapping for long agent outputs (code blocks must remain un-wrapped but scrollable horizontally).
    - [ ] **Performance:** UI rendering must maintain 60FPS even when agents are flooding the bus with data (render throttling).
- [ ] **Advanced Input System**
    - [ ] **Vim-Style Navigation:** `j/k` for scrolling, `i` for input, `/` for search in history.
    - [ ] **Tab Completion:** Dynamic completion for agent names, file paths, and common commands.
    - [ ] **History Navigation:** Up/Down arrow for command history (shell-like).
- [ ] **Visual Polish**
    - [ ] **Syntax Highlighting:** In-TUI syntax highlighting for code blocks in agent responses.
    - [ ] **Status Dashboard:** Real-time RAM/CPU usage per agent process + connectivity health.

## Phase 3: Reliability & "Never Revisit" Engineering
- [ ] **Error Handling & Self-Healing**
    - [ ] **Panic Safety:** Catch panics in agent threads to prevent the main TUI from crashing.
    - [ ] **Backoff Strategy:** Implement exponential backoff for restarting crashed agents.
    - [ ] **Resource Limits:** Hard caps on RAM/CPU usage per agent to prevent a rogue agent from freezing the host OS.
- [ ] **Validation & Testing**
    - [ ] **Fuzz Testing:** Fuzz the ANSI parser and Tag router to ensure zero crashes on unexpected input.
    - [ ] **Integration Tests:** 100% coverage of the Message Bus logic with mock agents.
    - [ ] **`agenthub doctor`:** CLI command to verify environment variables, CLI paths, and system compatibility.

## Phase 4: Configuration & Persistence
- [ ] **Configuration Schema**
    - [ ] **Strong Typing:** Use `serde` with strict validation (fail on unknown fields).
    - [ ] **Default Overrides:** Logic for merging `Global Config -> Project Config -> CLI Flags`.
- [ ] **Storage Engine**
    - [ ] **SQLite Backend:** Durable storage for all chat logs with full-text search capability.
    - [ ] **Context Pruning:** Automatic summarization of old history when approaching agent token limits.

## Phase 5: Security & Privacy (Enterprise Grade)
- [ ] **Credential Scrubbing:** Automatically redact API keys or detected secrets from logs and UI.
- [ ] **Safe Execution:** Run agent processes with restricted permissions (where OS allows).
- [ ] **Data Locality:** 100% local-first architecture; ensure no data leaves the machine unless explicitly requested.

## Phase 6: Shipping & Distribution
- [ ] **Packaging & Binary Quality**
    - [ ] **LTO & Stripping:** Optimize binaries for size and speed (Link Time Optimization).
    - [ ] **Code Signing:** Properly signed binaries for Windows (EV Cert) and macOS (Notarization) to avoid "Unknown Publisher" warnings.
- [ ] **CI/CD Pipeline**
    - [ ] **Multi-Platform Matrix:** Automatic builds for Linux (x64/ARM), macOS (Intel/Silicon), and Windows.
    - [ ] **Auto-Update:** Self-updating mechanism (e.g., via `self_update` crate) with checksum verification.
- [ ] **Compliance & Docs**
    - [ ] **SBOM:** Generate a Software Bill of Materials for security compliance.
    - [ ] **User Manual:** Hyperdetailed, searchable online documentation + built-in `help` command.
    - [ ] **Sign-up Integration:** Built-in "Join the Community" or "Waitlist" CLI command for the launch phase.
