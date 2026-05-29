# AGENTHUB: THE MONOLITHIC CONSTRUCTION MANUAL (V4: ENTERPRISE SPEC)

**TARGET AUDIENCE:** Autonomous AI Engineering Agents & Senior Systems Programmers
**STATUS:** STRICT INSTRUCTION SET (DEVIATION IS CAUSE FOR REJECTION)
**AUTHOR:** World-Class Systems Architect

## 0. PREAMBLE & INVARIANTS
This is not a high-level guide. This is a low-level, atomic implementation blueprint. You are to write the Rust code exactly as structurally defined here.
*   **Zero External APIs:** Do not use `reqwest` to hit LLM endpoints. All interaction is via local PTY manipulation.
*   **Memory Safety:** Strict adherence to `Send + Sync` bounds. No `unsafe` blocks unless explicitly wrapping OS-level syscalls for process management.
*   **Error Handling:** Use the `thiserror` crate for all module-specific errors. Never use `unwrap()` or `expect()` in production paths.

---

## MODULE 1: THE PHANTOM PTY ENGINE (`crates/core/src/pty/`)

### 1.1 `manager.rs`: PTY Lifecycle & Spawning
**Objective:** Spawn isolated, invisible pseudo-terminals that convince CLIs they are running in an interactive TTY.
*   **Dependencies:** `portable-pty = "0.8"`, `tokio = { version = "1.40", features = ["full"] }`
*   **Data Structures:**
    ```rust
    use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem, MasterPty, Child};
    use std::sync::Arc;
    use tokio::sync::Mutex;

    pub struct AgentPty {
        pub id: String,
        pub role: crate::roles::AgentRole,
        pub master: Arc<Mutex<Box<dyn MasterPty + Send + Sync>>>,
        pub child: Box<dyn Child + Send + Sync>,
        pub status: AtomicU8, // 0 = Spawning, 1 = Online, 2 = Offline
    }
    ```
*   **Implementation Steps:**
    1. Initialize `NativePtySystem`.
    2. Define `PtySize { rows: 40, cols: 120, pixel_width: 0, pixel_height: 0 }`.
    3. Create a `CommandBuilder` using the provided executable path (e.g., `gemini-cli`).
    4. Inject environment variables: `TERM=xterm-256color` and `COLORTERM=truecolor` to force the CLI to emit ANSI codes (which we need for heuristic parsing).
    5. Call `pty_system.spawn_pty_async()`.
    6. Return the `AgentPty` struct.

### 1.2 `io.rs`: Non-Blocking Byte Streams
**Objective:** Continuously drain the PTY's `stdout` without blocking the main event loop.
*   **Implementation Steps:**
    1. Clone the `MasterPty` reader: `let reader = master.try_clone_reader().unwrap();`
    2. Spawn a dedicated `tokio::task::spawn_blocking` thread. (PTY reads are blocking OS calls).
    3. Inside the loop, allocate a buffer: `let mut buf = [0u8; 4096];`
    4. Read bytes. If `bytes_read == 0`, the process died. Break loop and emit `ProcessDeathEvent`.
    5. Transmit the raw bytes via a multi-producer, single-consumer `tokio::sync::mpsc::Sender<Vec<u8>>` channel to the Sanitizer module.

### 1.3 `subagent.rs`: OS-Level Process Hooking
**Objective:** Intercept sub-processes spawned by the primary CLI (e.g., Aider spawning a search script).
*   **Dependencies:** `sysinfo = "0.30"`
*   **Implementation Steps:**
    1. Instantiate `sysinfo::System::new_all()`.
    2. Create a background heartbeat task `tokio::time::interval(Duration::from_millis(2000))`.
    3. On tick, call `sys.refresh_processes()`.
    4. Iterate `sys.processes()`. Check if `process.parent() == Some(agent_pty.child.id())`.
    5. If a new child PID is found, capture its `exe` path. Emit a `SubagentDetectedEvent(parent_id, child_pid, exe_name)` to the UI state manager.

---

## MODULE 2: ADAPTIVE STREAM SANITIZER (`crates/core/src/sanitizer/`)

### 2.1 `parser.rs`: VTE State Machine
**Objective:** Strip volatile ANSI control sequences while preserving pure text.
*   **Dependencies:** `vte = "0.11"`
*   **Data Structures:**
    ```rust
    pub struct CleanStream {
        pub output_buffer: String,
        pub parser: vte::Parser,
    }
    
    impl vte::Perform for CleanStream {
        fn print(&mut self, c: char) { self.output_buffer.push(c); }
        fn execute(&mut self, byte: u8) { 
            if byte == 0x08 { self.output_buffer.pop(); } // Handle backspace
            if byte == 0x0A { self.output_buffer.push('\n'); } // Handle linefeed
        }
        fn csi_dispatch(&mut self, _params: &[[u16; 2]], _intermediates: &[u8], _ignore: bool, _action: char) {
            // IGNORE ALL CSI (Colors, Cursor moves, Screen clears)
        }
    }
    ```
*   **Implementation Steps:**
    1. As raw byte buffers arrive from the `io.rs` channel, iterate through them.
    2. Call `parser.advance(&mut clean_stream, byte)`.
    3. Emit the updated `output_buffer` to the UI channel for real-time rendering.

### 2.2 `heuristic.rs`: Probabilistic Turn Detection
**Objective:** Determine when the LLM is waiting for user input.
*   **Dependencies:** `regex = "1.10"`
*   **Implementation Steps:**
    1. Maintain a `sliding_window` of the last 128 chars of the `output_buffer`.
    2. Check the window against the specific CLI's compiled regex (e.g., `(?m)^(?:User|Prompt|>>)\s*$`).
    3. **The Micro-Timeout:** If the regex matches, do NOT fire immediately. Wait `250ms`. If no new bytes arrive on the PTY stream in that 250ms window, the agent has definitively yielded the floor.
    4. Clear the `output_buffer` and emit `AgentTurnCompleteEvent(sanitized_text)`.

---

## MODULE 3: SERVER MECHANICS & RBAC (`crates/core/src/server/`)

### 3.1 `rbac.rs`: Bitflag Permission Matrix
**Objective:** Strict memory-efficient permission tracking.
*   **Dependencies:** `bitflags = "2.4"`
*   **Data Structures:**
    ```rust
    bitflags::bitflags! {
        pub struct AgentPerms: u32 {
            const CAN_BROADCAST = 0b00000001;
            const CAN_READ_FILES = 0b00000010;
            const CAN_WRITE_FILES = 0b00000100;
            const CAN_EXEC_UNIX = 0b00001000;
            const IS_ADMIN = 0b10000000;
        }
    }
    
    pub struct Role {
        pub name: String,
        pub perms: AgentPerms,
        pub induction_prompt: String,
        pub priority_weight: u8,
    }
    ```
*   **Implementation Steps:**
    1. Define default Roles in a lazy static map: `Leader`, `Reviewer`, `Sandbox`.
    2. A `Sandbox` role possesses only `CAN_BROADCAST`. If it outputs a unix command syntax in chat, the router must block it.

### 3.2 `moderation.rs`: Admin Overrides
**Objective:** Implement slash commands for the user.
*   **Implementation Steps:**
    1. When the TUI receives `/mute @agent1`, set `agent1_state.muted = true`. The central router drops their `AgentTurnCompleteEvent`s.
    2. For `/timeout @agent1 5m`, use OS syscalls. On Unix: `libc::kill(pid, libc::SIGSTOP)`. Sleep 5 minutes. `libc::kill(pid, libc::SIGCONT)`. (This literally freezes the OS process).
    3. For `/kick`, use `libc::kill(pid, libc::SIGKILL)`. Deallocate the `AgentPty` struct.

---

## MODULE 4: THE TIME-TRAVEL VFS (`crates/core/src/vfs/`)

### 4.1 `snapshot.rs`: High-Speed Differential Hashing
**Objective:** Snapshot massive codebases in <50ms.
*   **Dependencies:** `ignore = "0.4"`, `jwalk = "0.8"`, `blake3 = "1.5"`
*   **Implementation Steps:**
    1. Use `jwalk::WalkDir` to traverse the directory in parallel, automatically respecting `.gitignore`.
    2. Hash file contents using `blake3`. 
    3. Instead of physically copying all files, maintain an SQLite table: `CREATE TABLE snapshot (id TEXT, path TEXT, hash TEXT)`. Only copy files into `.agenthub_shadow/objects/{hash}` if that hash doesn't already exist. (This is a content-addressable storage model identical to git, but hyper-optimized for local agents).

### 4.2 `revert.rs`: The Undo Trigger
**Objective:** Instant rollback on `Ctrl+Z`.
*   **Implementation Steps:**
    1. Query the last snapshot ID from SQLite.
    2. Iterate the snapshot records. 
    3. Use `std::fs::hard_link` to instantly link the objects from `.agenthub_shadow/objects/` back into the working directory, overwriting modified files. (Hard linking ensures the revert takes <10ms regardless of project size).

---

## MODULE 5: ZERO-API RAG (AST INJECTION) (`crates/core/src/context/`)

### 5.1 `ast.rs`: S-Expression Extraction
**Objective:** Read the codebase and extract logic signatures without reading function bodies.
*   **Dependencies:** `tree-sitter = "0.20"`, `tree-sitter-rust`, `tree-sitter-python`
*   **Implementation Steps:**
    1. Instantiate a `tree_sitter::Parser` and set the language based on file extension.
    2. Parse the file into a `Tree`.
    3. Compile an S-expression Query. For Rust:
       ```scm
       (function_item name: (identifier) @fn_name parameters: (parameters) @params return_type: (_) @ret)
       (struct_item name: (type_identifier) @struct_name)
       ```
    4. Execute the query using `QueryCursor`. Iterate matches, extract the exact byte ranges from the source string, and format them: `fn my_func(a: i32) -> i32;`
    5. Return the concatenated signatures.

### 5.2 `injector.rs`: Stealth Modification
**Objective:** Augment the user's prompt invisibly.
*   **Implementation Steps:**
    1. Scan user input for file paths (regex: `[\w\-/\\]+\.(rs|py|ts|js|go)`).
    2. For each detected file, trigger `ast.rs`.
    3. Construct the stealth payload: 
       `[SYSTEM CONTEXT: The file {file} contains these signatures: {signatures}. End Context.]\n{user_input}`
    4. Write this payload via the `io.rs` PTY writer.

---

## MODULE 6: THE RATATUI COMMAND CENTER (`crates/ui/src/`)

### 6.1 `layout.rs`: Terminal Constraints
**Objective:** Render the complex Discord-style interface.
*   **Dependencies:** `ratatui = "0.26"`, `crossterm = "0.27"`
*   **Implementation Steps:**
    1. Set up a `Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(0), Constraint::Length(3)])`.
    2. Split the main area horizontally: `[Constraint::Percentage(20), Constraint::Percentage(80)]`.
    3. Left pane: `Sidebar` (renders Active Agents, their Roles, and Subagent hierarchical trees).
    4. Right pane: `GroupChat` (renders `List` of `ListItem`s formatted as `[AgentName]: text`).

### 6.2 `racing.rs`: Multiplexed LLM UI
**Objective:** The visual implementation of Hook 1.
*   **Implementation Steps:**
    1. When `input.rs` detects multiple tags (`@gemini @claude code`), trigger a layout shift.
    2. Divide the `GroupChat` pane into N equal vertical columns: `Constraint::Ratio(1, N)`.
    3. Render a `Paragraph` widget in each column. Subscribe to the real-time VTE `output_buffer` of each respective agent. The UI must refresh at 60Hz via a `crossterm::event::poll` loop.

---

## THE ABSOLUTE DEFINITION OF DONE (DoD)
This manual is complete only when:
1. `agenthub-core` achieves >90% code coverage via `cargo tarpaulin`.
2. The VTE ANSI stripper successfully parses and cleanses a raw output dump of the `cursor-cli` without missing a single byte.
3. The RAG AST Injector successfully queries a 5,000-line Rust file and injects only the signatures in under 50ms.
4. The Time-Travel VFS successfully hashes a `node_modules` structure (if not ignored) and performs a hard-link revert without panic.

**BEGIN IMPLEMENTATION. NO DEVIATIONS.**