# AgentHub: Industry-Grade Implementation Roadmap

This roadmap outlines the path from the current scaffold to a production-ready, industry-grade multi-agent orchestrator.

## Phase 1: Foundations & Documentation (COMPLETE)
- [x] Initial Vision & Architecture docs.
- [x] Directory structure setup.
- [x] `GEMINI.md` project-level instructions.

## Phase 2: Project Scaffolding (COMPLETE)
- [x] Initialize Cargo project.
- [x] Add core dependencies (`ratatui`, `tokio`, `crossterm`, `serde`).
- [x] Verified TUI "Hello World" compilation.

## Phase 3: Core Process Management (IN PROGRESS)
- [ ] **Agent Abstraction Layer**
    - [ ] Define `AgentAdapter` trait for unified I/O.
    - [ ] Implement `LocalProcess` wrapper for spawning child CLI processes.
    - [ ] Handle non-blocking async streaming of `stdout` and `stderr`.
    - [ ] Implement robust `stdin` injection for prompting.
- [ ] **Stream Processing**
    - [ ] ANSI escape code stripping/parsing (handling colors and loaders).
    - [ ] Message buffering and chunking to handle streaming responses.
    - [ ] Error propagation from child processes to the UI.

## Phase 4: Orchestration & Message Bus
- [ ] **Unified Message Bus**
    - [ ] Implement central event dispatcher (User -> Bus -> Agents).
    - [ ] Global history state management (In-memory for now).
- [ ] **Routing Logic**
    - [ ] Regex-based tag parsing (`@agent_name`).
    - [ ] Multi-cast support (sending one prompt to multiple agents).
- [ ] **Workflow Engine**
    - [ ] Sequential "Piping" (Agent A output -> Agent B input).
    - [ ] Simple state-machine for autonomous turn-taking.

## Phase 5: Industry-Grade UI/UX
- [ ] **Layout Refinement**
    - [ ] Scrollable chat history with word-wrapping.
    - [ ] Sidebar for active agent status (CPU usage, process health).
    - [ ] "Raw Log" toggle for debugging specific agent I/O.
- [ ] **Interactive Elements**
    - [ ] Tab-completion for agent tags.
    - [ ] Vim-style or Emacs-style keybindings for navigation.
    - [ ] Support for resizing terminal windows gracefully.
- [ ] **Theming & Aesthetics**
    - [ ] Custom color palettes.
    - [ ] Markdown-like rendering (bolding code blocks, etc.) within the TUI.

## Phase 6: Configuration & Persistence
- [ ] **User Configuration**
    - [ ] `config.yaml/toml` for defining agent paths and flags.
    - [ ] Default system prompts per agent.
- [ ] **Session Persistence**
    - [ ] SQLite or JSON-based local storage for chat history.
    - [ ] Export session to Markdown/HTML.
- [ ] **Context Management**
    - [ ] Strategy for "Context Compressing" (summarizing long histories for agents).

## Phase 7: Robustness & Reliability (The "Industry Grade" Layer)
- [ ] **Error Handling**
    - [ ] Graceful recovery from agent process crashes.
    - [ ] Timeout management for hanging agents.
    - [ ] Handling of high-frequency output (rate limiting UI updates).
- [ ] **Testing Suite**
    - [ ] Unit tests for all routing and parsing logic.
    - [ ] Integration tests with mock "stub" agents.
    - [ ] Performance benchmarks for the message bus.
- [ ] **Observability**
    - [ ] Internal logging to a file (`agenthub.log`).
    - [ ] Health checks for external dependencies (API keys, path checks).

## Phase 8: Distribution & Security
- [ ] **Packaging**
    - [ ] Cross-platform builds (Windows, macOS, Linux) via GitHub Actions.
    - [ ] Installer scripts or Homebrew/Scoop/Winget recipes.
- [ ] **Security**
    - [ ] Secure environment variable handling (preventing key leakage).
    - [ ] Sandbox considerations for untrusted CLI tools.
- [ ] **Documentation**
    - [ ] Comprehensive User Guide (Manual).
    - [ ] API/Trait documentation for 3rd party adapters.
