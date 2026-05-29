# Implementation Roadmap

## Phase 1: Foundations & Documentation (CURRENT)
- [x] Initial Vision & Architecture docs.
- [x] Directory structure setup.
- [x] `GEMINI.md` project-level instructions.

## Phase 2: Project Scaffolding
- [ ] Initialize Cargo project.
- [ ] Add dependencies (`ratatui`, `tokio`, `crossterm`, `serde`).
- [ ] Basic TUI "Hello World" with dummy panes.

## Phase 3: Process Management
- [ ] Implement `AgentAdapter` trait.
- [ ] Implement `LocalProcess` wrapper for standard CLIs.
- [ ] Test with a mock CLI (echo-style).

## Phase 4: Orchestration Layer
- [ ] Unified Message Bus.
- [ ] Tag parsing logic (`@agent`).
- [ ] Simple sequential piping between agents.

## Phase 5: Polishing & Advanced Features
- [ ] ANSI escape code stripping.
- [ ] Configurable agent profiles.
- [ ] LLM-based autonomous "Moderator" mode.
