# AgentHub: Project Instructions

Welcome to AgentHub. This file contains foundational instructions for any agent working on this codebase.

## Project Context
AgentHub is a Rust-based TUI orchestrator for multiple AI agent CLIs. It aims to provide a unified "Group Chat" interface where agents can interact and collaborate.

## Architectural Principles
1.  **Rust First:** All core logic must be in Rust. Prioritize performance and safety.
2.  **Trait-Based Extensibility:** Use traits for agent adapters to allow for easy integration of new CLI tools.
3.  **Async/Non-blocking:** Use `tokio` for all I/O and process management. The TUI must remain responsive even when agents are processing.
4.  **No Side-Effect CLI Usage:** When wrapping external CLIs, always use flags like `--non-interactive` or `-y` to avoid blocking the orchestrator.

## Documentation Reference
*   See `about/VISION.md` for the core philosophy.
*   See `about/ARCHITECTURE.md` for technical details.
*   See `docs/ROADMAP.md` for current progress and next steps.

## Coding Standards
*   Follow standard Rust idioms and naming conventions.
*   Use `ratatui` for UI components.
*   Ensure all new features are accompanied by relevant tests (unit tests for logic, mock tests for process handling).
