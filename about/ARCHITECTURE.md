# Technical Architecture

## Technical Stack
*   **Core Language:** Rust (Performance, safety, and concurrency).
*   **TUI Framework:** `ratatui` + `crossterm`.
*   **Async Runtime:** `tokio`.
*   **Serialization:** `serde` / `serde_json`.

## System Components

### 1. Process Adapters
*   **Abstract Interface:** A trait-based system for CLI wrappers.
*   **I/O Stream Hijacking:** Non-blocking capture of `stdout`/`stderr` and injection into `stdin`.
*   **ANSI Sanitization:** Robust stripping of control codes to retrieve clean text.

### 2. The Message Bus (Hub)
*   **Event-Driven:** Centralized dispatcher for user inputs and agent outputs.
*   **History Management:** In-memory buffer of the full conversation.

### 3. Orchestration Engine
*   **Router:** Logic for parsing `@tags` and directing messages.
*   **Context Synthesizer:** Preparing tailored history chunks for agents based on their specific context windows.

### 4. TUI Frontend
*   **Reactive UI:** Instant updates as agents stream output.
*   **Multi-Pane Layout:** Separate areas for the main chat, agent status, and raw debug logs.
