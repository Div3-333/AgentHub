# System Architecture: The Phantom Terminal (Enterprise Edition)

AgentHub uses a highly specialized architecture focused on Process Control, Stream Sanitization, and Local File System manipulation to achieve its "Killer Features."

## 1. High-Level Topology

```mermaid
graph TD
    UI[AgentHub TUI<br>(Ratatui)] <-->|Event Bus| Core[Rust Orchestrator]
    
    subgraph PTY Multiplexer
        Core <-->|PTY| CLI1[gemini-cli]
        Core <-->|PTY| CLI2[cursor-cli]
        Core <-->|PTY| CLI3[aider]
    end
    
    subgraph Local Context Engine
        Core -->|Tree-Sitter| AST[Local Code Parser]
        AST -->|Injected String| PTY Multiplexer
    end

    subgraph Time-Travel VFS
        Core -->|Pre-execution hook| Git[Shadow Git Stash]
    end
```

## 2. Component 1: The PTY Multiplexer Engine
The core of the system is the Pseudo-Terminal (PTY) manager.
*   **Isolation:** Every agent CLI is spawned inside an isolated PTY (`portable-pty`). The CLIs believe they are connected to a real TTY, preventing them from buffering output or crashing.
*   **Multiplexing:** The engine can duplicate a single user input string and push it to the `stdin` of multiple PTYs simultaneously (enabling **LLM Racing**).

## 3. Component 2: The Adaptive Stream Sanitizer
Free-tier CLIs use unpredictable ANSI escape sequences for colors and animations. 
*   **State-Machine Parsing:** AgentHub uses an advanced ANSI parser (`vte` crate) to decode the byte stream. It strips out cursor movements (`\x1b[A`), clears (`\x1b[2K`), and colors, emitting only pure semantic text.
*   **Heuristic Prompt Detection:** Because we don't have APIs, we don't get a "Done" signal. AgentHub uses regex heuristics (defined in JSON Driver Profiles) to detect when an agent is waiting for input (e.g., detecting `\n> ` or `User: ` sitting idle in the buffer for >100ms).

## 4. Component 3: The Time-Travel VFS (Virtual File System)
To guarantee absolute safety when agents modify local files.
*   **Pre-Hook Checkpointing:** When a prompt is submitted, AgentHub triggers a background task that snapshots the current working directory's state. It leverages a hidden `.agenthub_shadow/` directory using lightweight git-like hashing to store file states.
*   **Instant Revert:** If the user triggers an Undo, AgentHub performs a hard reset from the shadow directory and pops the last message off the TUI state, effectively rewinding time.

## 5. Component 4: The Auto-Context Engine (Zero-API RAG)
How to pass whole-repo context to a CLI that only accepts text input?
*   **Local Indexing:** Uses `ignore` to traverse the workspace and `tree-sitter` to parse code structure.
*   **Prompt Concatenation:** If a user types `@gemini fix auth.rs`, AgentHub intercepts the prompt, reads `auth.rs`, strips comments and whitespace to save tokens, and dynamically rewrites the input to the PTY:
    *   *Hidden Input:* `Context:\n[minified code]\nUser prompt: fix auth.rs`
*   This gives the illusion of deep repository integration without paying for embedding APIs.

## 6. Component 5: The Frankenstein Pipeline Router
A mini-interpreter built into the chat box.
*   **Syntax:** `@agentA prompt | > local_cmd | @agentB`
*   **Execution:** 
    1. Send prompt to Agent A PTY. 
    2. Wait for completion, capture sanitized output. 
    3. Spawn standard `std::process::Command` (e.g., `bash`, `python`, `cargo`). Pass Agent A's output to its `stdin`. 
    4. Capture the `stdout`/`stderr` of the local command. 
    5. Inject that output into Agent B PTY's `stdin` with a wrapper: `Command output was: [...]`.