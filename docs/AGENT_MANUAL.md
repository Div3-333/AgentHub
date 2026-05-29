# AGENTHUB: ATOMIC CONSTRUCTION MANUAL

**TARGET AUDIENCE:** Autonomous AI Engineering Agents
**STATUS:** STRICT INSTRUCTION SET (DO NOT DEVIATE)

## 0. PREAMBLE & RULES OF ENGAGEMENT
This document is the absolute source of truth for building AgentHub. You are part of an AI workforce tasked with implementing this zero-api, Phantom Terminal orchestrator. 
1. **No External APIs:** You must never write code that imports `reqwest` to hit `api.openai.com` or `api.anthropic.com`. All AI interaction is done by spawning PTYs.
2. **Strict Invariants:** If a test fails, you do not comment out the test. You fix the logic.
3. **Atomic Execution:** You will implement this blueprint exactly task-by-task. Do not skip ahead.

---

## MODULE 1: THE PHANTOM PTY ENGINE (`src/pty/`)

### Task 1.1: PTY Spawning & Lifecycle
**Objective:** Spawn isolated, invisible terminal environments for CLIs.
*   **Create File:** `src/pty/manager.rs`
*   **Dependencies:** `portable-pty`, `tokio`.
*   **Struct Definition:**
    ```rust
    pub struct AgentPty {
        pub id: String, // e.g., "gemini-1"
        pub master: Box<dyn portable_pty::MasterPty + Send>,
        pub process: Box<dyn portable_pty::Child + Send>,
    }
    ```
*   **Function:** `pub fn spawn_agent(cmd: &str, args: &[&str], id: &str) -> Result<AgentPty, Error>`
    *   *Implementation Detail:* Must use `PtySystem::new()`. Set PTY size to 120x40. Spawn the command.
*   **Acceptance Criteria:** `cargo test test_spawn_agent` must verify that launching a basic `echo` command inside the PTY returns the text.

### Task 1.2: Asynchronous Stream Readers
**Objective:** Read bytes from the PTY non-blockingly.
*   **Create File:** `src/pty/reader.rs`
*   **Function:** `pub async fn read_stream(master: Arc<Mutex<Box<dyn MasterPty>>>, tx: broadcast::Sender<RawByteEvent>)`
*   **Implementation Detail:** Use a `tokio::task::spawn_blocking` loop to read from the PTY's `try_clone_reader()`, yielding `[u8; 1024]` buffers to the Tokio channel.
*   **Strict Invariant:** Do NOT use standard `tokio::process::Command` stdout piping. It will deadlock interactive CLIs.

### Task 1.3: Subagent Capture Protocol (Process Hooking)
**Objective:** Detect if a CLI (like Aider) spawns its own sub-process and hijack it.
*   **Create File:** `src/pty/subagent.rs`
*   **Dependencies:** `sysinfo`
*   **Function:** `pub async fn monitor_process_tree(pid: u32, tx: mpsc::Sender<NewSubagentEvent>)`
*   **Implementation Detail:** Poll `sysinfo::System` every 2 seconds for children of the agent's PID. If a new process appears that uses significant CPU, flag it as a subagent and emit an event to the UI.

---

## MODULE 2: ADAPTIVE STREAM SANITIZER (`src/sanitizer/`)

### Task 2.1: State-Machine ANSI Stripper
**Objective:** Convert raw colorful terminal output into clean semantic strings.
*   **Create File:** `src/sanitizer/ansi.rs`
*   **Dependencies:** `vte`
*   **Struct Definition:**
    ```rust
    pub struct CleanStream {
        buffer: String,
        parser: vte::Parser,
    }
    impl vte::Perform for CleanStream { ... }
    ```
*   **Implementation Detail:** Implement `vte::Perform`. Ignore `print_ext`, `csi_dispatch` (colors/cursor moves). Only append `print` chars to the buffer.
*   **Acceptance Criteria:** Must correctly turn `\x1b[32mHello\x1b[0m \x1b[KWorld` into `Hello World`.

### Task 2.2: Heuristic Turn Detection
**Objective:** Know when an agent is done "typing".
*   **Create File:** `src/sanitizer/heuristic.rs`
*   **Function:** `pub fn detect_turn_complete(buffer: &str, regex_pattern: &Regex) -> bool`
*   **Implementation Detail:** Combine regex matching (e.g., `(?m)^User: $`) with a `tokio::time::timeout(Duration::from_millis(500))` on the byte channel. If both trigger, the agent is done.

---

## MODULE 3: THE DISCORD SERVER MECHANICS (`src/server/`)

### Task 3.1: Role-Based Access Control (RBAC) Data Structures
**Objective:** Define the permission matrix.
*   **Create File:** `src/server/roles.rs`
*   **Enum Definition:**
    ```rust
    pub enum AgentRole {
        Leader,
        Builder,
        Reviewer,
        Auditor,
        Custom(String),
    }
    ```
*   **Struct Definition:**
    ```rust
    pub struct Permissions {
        pub can_broadcast: bool,
        pub can_run_unix_commands: bool,
        pub weight: u8, // 1-10 priority in Open Floor chat
    }
    ```

### Task 3.2: The Grand Induction (System Prompting)
**Objective:** Silently initialize the agents before joining the server.
*   **Create File:** `src/server/induction.rs`
*   **Function:** `pub async fn induct_agent(pty: &mut AgentPty, role: &AgentRole) -> Result<(), Error>`
*   **Implementation Detail:** Construct a string: `"You are in AgentHub. You are role {role}. Do not format with markdown. Acknowledge."` Write to PTY `stdin`. Await sanitized response containing `"Ack"`.

### Task 3.3: Moderation Commands
**Objective:** Mute, Deafen, and Kick agents.
*   **Create File:** `src/server/mod_controls.rs`
*   **Implementation Detail:** 
    *   `mute`: Set an internal atomic boolean `is_muted = true`. The central bus will drop messages from this agent ID.
    *   `deafen`: The central bus stops writing `[OtherAgent says]` strings to this agent's `stdin`.
    *   `kick`: Call `pty.process.kill()`.

---

## MODULE 4: THE TIME-TRAVEL VFS (`src/vfs/`)

### Task 4.1: Shadow Directory Checkpointing
**Objective:** Snapshot the workspace before an agent modifies it.
*   **Create File:** `src/vfs/snapshot.rs`
*   **Dependencies:** `fs_extra`, `ignore`
*   **Function:** `pub fn create_checkpoint(cwd: &Path) -> Result<String, Error>`
*   **Implementation Detail:** Use `ignore::WalkBuilder` to get all files not in `.gitignore`. Copy them to `.agenthub_shadow/{timestamp}/`. Return the timestamp ID.

### Task 4.2: The Undo Mechanic
**Objective:** Revert the workspace and chat state.
*   **Function:** `pub fn revert_checkpoint(id: &str, cwd: &Path) -> Result<(), Error>`
*   **Implementation Detail:** Force delete the current CWD contents (except `.git`) and recursively copy the contents of the shadow folder back into the CWD.

---

## MODULE 5: ZERO-API AUTO-CONTEXT (AST Indexing) (`src/context/`)

### Task 5.1: Tree-Sitter Parsing
**Objective:** Extract definitions from the repo for dynamic prompt injection.
*   **Create File:** `src/context/parser.rs`
*   **Dependencies:** `tree-sitter`, `tree-sitter-rust`, `tree-sitter-python`
*   **Function:** `pub fn extract_signatures(file_path: &Path) -> Result<String, Error>`
*   **Implementation Detail:** Parse the file. Query for `function_item` and `struct_item`. Return a minified string of just the signatures (no function bodies).

### Task 5.2: Stealth Injection
**Objective:** Paste context before the user's prompt.
*   **Create File:** `src/context/injector.rs`
*   **Function:** `pub fn inject_context(prompt: &str, cwd: &Path) -> String`
*   **Implementation Detail:** Regex search the prompt for words ending in `.rs`, `.py`, `.ts`. If found, call `extract_signatures` for those files, and prepend: `[CONTEXT:\n{signatures}]\nUSER: {prompt}`.

---

## MODULE 6: THE COMMAND CENTER TUI (`src/ui/`)

### Task 6.1: Workspace Modes State Machine
**Objective:** Support DM, Group Chat, and Server modes.
*   **Create File:** `src/ui/state.rs`
*   **Enum:** `pub enum WorkspaceMode { Dm(String), Group(Vec<String>), Server }`
*   **Implementation Detail:** UI render loops must dynamically adapt: Server mode shows a sidebar of channels/roles. DM mode is a simple full-screen terminal.

### Task 6.2: LLM Racing UI (Split Panes)
**Objective:** Render multiple agents streaming side-by-side.
*   **Create File:** `src/ui/racing.rs`
*   **Implementation Detail:** Use `ratatui::layout::Layout::direction(Direction::Horizontal)`. If 3 agents are tagged in a prompt (`@gemini @claude @aider write script`), spawn 3 equal-width `Paragraph` widgets. Update them concurrently via the Event Bus.

### Task 6.3: The Frankenstein Router Syntax
**Objective:** Parse pipeline strings in the input box.
*   **Create File:** `src/ui/input.rs`
*   **Implementation Detail:** If input matches `@agent_a <prompt> | > <cmd> | @agent_b`, construct a `PipelineTask` struct. Send it to the Orchestrator instead of standard broadcast.

---

## FINAL VALIDATION (THE GOD-TIER DoD)
You may not mark the project as complete until:
1. `cargo test --all` passes with 0 failures.
2. You have empirically spawned 3 instances of a dummy CLI in the TUI, assigned them different roles, successfully muted one, and used `Ctrl+Z` to revert a file change they made. 
3. Code is 100% formatted via `cargo fmt`.