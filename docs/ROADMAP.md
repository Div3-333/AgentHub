# The Ultimate Phantom Terminal Roadmap

This roadmap is an exhaustive, granular engineering plan to build AgentHub into a world-class product. Completion of these phases guarantees an industry-grade developer multiplier with massive viral potential.

## Phase 1: Foundations & The PTY Core
- [x] Initial project scaffolding (Rust + Ratatui).
- [ ] **The PTY Manager Module:**
    - [ ] Integrate `portable-pty`.
    - [ ] Create `struct PtyChild { master: Box<dyn MasterPty>, slave: Box<dyn SlavePty> }`.
    - [ ] Implement non-blocking asynchronous `read_stdout` looping tasks.
    - [ ] Implement `write_stdin` with raw byte injection.
- [ ] **Driver Profile System:**
    - [ ] Define JSON schema for CLI profiles (executable name, init args, prompt regex patterns).
    - [ ] Build configuration loader to parse `~/.agenthub/drivers.json`.

## Phase 2: Stream Sanitization & Heuristic Extraction
- [ ] **The ANSI Stripper Pipeline:**
    - [ ] Implement the `vte` parser to process raw PTY bytes.
    - [ ] Filter out all non-printable control characters while preserving `\n` and `\t`.
- [ ] **Heuristic "Turn" Detection:**
    - [ ] Implement a sliding window buffer over the sanitized output.
    - [ ] Match buffer contents against the Driver's "Prompt Regex".
    - [ ] Add a `tokio::time::timeout` heuristic: If stream is silent for 500ms after a specific newline pattern, assume generation is complete.

## Phase 3: The Unified TUI & Multiplexing
- [ ] **The "Command Center" UI:**
    - [ ] Build a multi-pane `ratatui` interface (Chat History, Input Box, Status Sidebar).
    - [ ] Implement auto-scrolling and word-wrapping for incoming agent text.
- [ ] **Tag Router (`@agent`):**
    - [ ] Parse user input for tags.
    - [ ] Dispatch messages exclusively to the matched PTY's `stdin`.
- [ ] **LLM Racing (Hook Feature 1):**
    - [ ] Implement UI for split-pane parallel streaming.
    - [ ] Allow multiple tags (`@gemini @claude code a button`).
    - [ ] Route the single string to multiple PTYs simultaneously and manage concurrent UI updates.

## Phase 4: Autonomous Pipelines, Open Floors & The "Frankenstein" Router
- [ ] **The Pipeline Parser:**
    - [ ] Implement a custom mini-parser for the pipe `|` syntax in the chat box.
- [ ] **Agent-to-Agent Handoffs:**
    - [ ] Automatically capture the `MessageComplete` event from Agent A, wrap the output in a system string, and inject it to Agent B.
- [ ] **"Open Floor" Dynamics (Unconstrained N-Way Chat):**
    - [ ] **True Concurrency:** Maintain active listening loops on *all* agent PTYs simultaneously.
    - [ ] **Broadcast Injector:** When any entity (User or Agent X) speaks, their sanitized text is instantly broadcasted to the `stdin` of all other active agents with the prefix `[Agent X says]: ...`.
    - [ ] **Chaos Heuristics:** Implement random backoff timers so agents don't all attempt to reply to a broadcast at the exact same millisecond.
- [ ] **Unix Integration (Hook Feature 3):**
    - [ ] Support `> command` syntax to run standard OS commands mid-pipeline.
    - [ ] Capture OS `stderr` and route it back to the agent PTY automatically for self-healing code loops.


## Phase 4.5: The Discord Server Mechanics (RBAC & Moderation)
- [ ] **Agent Induction Protocol (System Prompting):**
    - [ ] Create a hidden initialization phase. When an agent is spawned, instantly push a massive context prompt into its `stdin` explaining AgentHub, its specific role, and how to read `[Agent X says]` prefixes.
    - [ ] Wait for the agent to acknowledge the rules before marking it "Online" in the UI.
- [ ] **Role-Based Access Control (RBAC):**
    - [ ] Define default roles: `Leader` (can override others), `Builder` (writes code), `Reviewer` (critiques), `Auditor` (security checks).
    - [ ] Build a JSON schema to allow users to create Custom Roles with custom induction prompts.
- [ ] **Server Moderation Commands:**
    - [ ] `/mute @agent`: Agent stays active but its output is hidden from the group chat.
    - [ ] `/deafen @agent`: Agent stays active but stops receiving broadcasts from other agents.
    - [ ] `/kick @agent`: Forcefully sends `SIGKILL` to the specific agent's PTY.
    - [ ] `/timeout @agent 5m`: Temporarily suspends the agent's PTY process.
- [ ] **Multi-Instance Scaling:**
    - [ ] Update the PTY Manager to spawn unique clones. E.g., spawn `/usr/bin/gemini-cli` three times, assigning them internal tags `@gemini-1`, `@gemini-2`, `@gemini-3`.
    - [ ] Implement graceful fallback: If an agent locks up due to IP limits (e.g., trying to run 5 Cursor CLIs), catch the error and alert the user.

## Phase 5: The Time-Travel Workspace (Safety Engine)
- [ ] **Shadow VFS Implementation (Hook Feature 2):**
    - [ ] Create `.agenthub_shadow/` architecture in the project root.
    - [ ] Before any pipeline runs, execute a hyper-fast differential copy of the working directory (excluding `.git` and `node_modules`).
- [ ] **The Undo Mechanic:**
    - [ ] Bind `Ctrl+Z` in the TUI to trigger the revert protocol.
    - [ ] Restore files from the shadow directory and pop the most recent interactions off the chat history state.

## Phase 6: Zero-API Auto-Context Engine
- [ ] **AST Indexing (Hook Feature 4):**
    - [ ] Integrate `ignore` crate to build a list of valid workspace files.
    - [ ] Integrate `tree-sitter` to parse definitions (functions, classes, structs) for quick lookup.
- [ ] **Dynamic Prompt Injection:**
    - [ ] Intercept user prompts mentioning filenames (e.g., `fix main.rs`).
    - [ ] Read `main.rs`, minify the string, and stealthily prepend it to the text sent to the PTY.

## Phase 7: Robustness & Enterprise Edge Cases
- [ ] **Orphan Process Annihilation:**
    - [ ] Implement panic handlers and `Drop` traits on the PTY manager to guarantee child processes (`gemini-cli`, etc.) receive `SIGKILL` when AgentHub exits.
- [ ] **Interactive Prompt Bypassing:**
    - [ ] Add specific Driver profile hooks to detect "Do you want to continue? [Y/n]" and automatically inject `Y\n` into the `stdin` to prevent pipeline stalls.
- [ ] **Log Rotation & Debugging:**
    - [ ] Maintain a raw, un-sanitized byte log for each PTY session in `/tmp/agenthub_debug/` to help users diagnose why a specific CLI driver profile is failing.

## Phase 8: Polish, Packaging & Launch
- [ ] **Configurable Keybindings:** Vim/Emacs mode support for the TUI input box.
- [ ] **Cross-Platform PTY Testing:** Ensure `portable-pty` behaves identically on Windows (ConPTY) and Unix (pseudoterminals).
- [ ] **Binary Distribution:** Setup GitHub actions to compile optimized binaries (`cargo build --release`) for Mac/Windows/Linux to ensure zero-friction installation.