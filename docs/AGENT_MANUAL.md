# AGENTHUB: THE MONOLITHIC CONSTRUCTION MANUAL (V5: TRUE ENTERPRISE SYSTEMS SPECIFICATION)

**TARGET AUDIENCE:** Autonomous AI Engineering Agents & Senior Systems Programmers
**STATUS:** STRICT INSTRUCTION SET. NO ABSTRACTIONS. NO ASSUMPTIONS.
**AUTHOR:** World-Class Systems Architect

## 0. PREAMBLE & INVARIANTS (THE LAWS OF PHYSICS)
This specification dictates the physical memory layout, concurrency models, and algorithmic complexities required to build AgentHub. You will not rely on garbage collection or unbounded queues.
1. **Memory Ordering:** All atomic operations must explicitly declare their memory ordering (`Ordering::SeqCst` for coordination, `Ordering::Acquire`/`Ordering::Release` for lock-free synchronization, `Ordering::Relaxed` ONLY for statistical counters).
2. **Allocation:** Heap allocations inside the hot-path (PTY reading, stream parsing) are STRICTLY FORBIDDEN. All buffers must be pre-allocated and managed via `bumpalo` (Arena Allocation) or object pools (`crossbeam-queue`).
3. **Concurrency:** `std::sync::Mutex` is banned in the hot-path. Use `tokio::sync::RwLock` for async state, and `crossbeam::epoch` for lock-free concurrent data structures.
4. **Error Handling:** Every `Result` must be mapped to a custom `thiserror` enum that implements `Into<u16>` for deterministic exit codes mapping to POSIX standards.

---

## MODULE 1: THE HYPER-CONCURRENT PTY ENGINE (`crates/core/src/pty/`)

### 1.1 `manager.rs`: PTY Lifecycle & OS-Level Spawning
**Objective:** Spawn isolated PTYs with absolute guarantee against zombie processes and descriptor leaks.
*   **Dependencies:** `portable-pty = "0.8"`, `libc = "0.2"` (Unix), `winapi = "0.3"` (Windows).
*   **Data Structures:**
    ```rust
    #[repr(C, align(64))] // Cache-line alignment to prevent false sharing
    pub struct AgentPty {
        pub id: uuid::Uuid,
        pub role_mask: AtomicU32,
        pub master_fd: RawFd, // Store raw descriptor for io_uring/epoll polling
        pub process_id: u32,
        status: AtomicU8, 
    }
    ```
*   **Implementation Specs (Unix):**
    1. Call `openpty()` via `libc`. Set `O_NONBLOCK` on the master file descriptor immediately using `fcntl()`.
    2. Before `fork()`, configure `termios`: Disable `ECHO`, `ICANON`, `ISIG`. Set `VMIN=1`, `VTIME=0`.
    3. Post `fork()`, in the child process: Call `setsid()` to create a new session. `dup2` the slave PTY to `STDIN_FILENO`, `STDOUT_FILENO`, `STDERR_FILENO`. Close all other file descriptors above 2 to prevent leaking secure daemon sockets to the untrusted CLI agent.
    4. Execute `execvp`.
*   **Implementation Specs (Windows):**
    1. Use `CreatePseudoConsole` (ConPTY API) via `winapi`.
    2. Initialize `STARTUPINFOEXW` with `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`.
    3. Call `CreateProcessW`.

### 1.2 `io.rs`: Zero-Copy Non-Blocking Byte Streams
**Objective:** Drain PTYs at gigabit speeds without allocating on the heap.
*   **Dependencies:** `tokio-uring` (Linux), `mio` (macOS/Windows fallback).
*   **Data Structures:**
    ```rust
    pub struct PtyRingBuffer {
        buffer: UnsafeCell<[u8; 65536]>, // 64KB L1 Cache optimized buffer
        head: AtomicUsize,
        tail: AtomicUsize,
    }
    ```
*   **Implementation Specs:**
    1. **Backpressure & Topology:** Do NOT use `tokio::mpsc`. The overhead of allocating `Vec<u8>` for every message is unacceptable. Instead, allocate a statically sized lock-free `PtyRingBuffer` shared between the Reader thread and the Parser thread.
    2. **Read Loop (Linux):** Use `io_uring` to queue `IORING_OP_READ` operations directly into the ring buffer memory. This achieves zero-copy transfer from kernel space to user space.
    3. **Read Loop (Fallback):** Use `epoll`/`kqueue` via `mio::Poll`. When `Readable` is triggered, loop `libc::read()` until `EAGAIN` or `EWOULDBLOCK` is hit.
    4. **Memory Fence:** After writing to the ring buffer, execute `head.store(new_head, Ordering::Release)` to publish the bytes to the Parser thread.

### 1.3 `subagent.rs`: eBPF Process Hooking (Linux) & ETW (Windows)
**Objective:** Deterministically catch sub-processes without polling lag.
*   **Dependencies:** `aya` (eBPF), `windows-sys` (ETW).
*   **Implementation Specs:**
    1. **Linux (eBPF):** Do not poll `sysinfo`. It is slow and misses short-lived processes. Write an eBPF program attached to the `sched_process_exec` tracepoint. Filter by `task->parent->pid == agent_pty_pid`. Send events to user-space via a BPF Ring Buffer.
    2. **Windows:** Subscribe to Event Tracing for Windows (ETW) `Process/Start` events. Filter by `ParentId`.
    3. **Action:** When detected, automatically create a new `AgentPty` struct, bind it to the detected PID's standard outputs (if accessible), and register it in the RBAC matrix with the `Subagent` role.

---

## MODULE 2: SIMD-ACCELERATED STREAM SANITIZER (`crates/core/src/sanitizer/`)

### 2.1 `parser.rs`: Headless Grid State Machine
**Objective:** Parse ANSI codes perfectly. Ignoring codes is insufficient for loading spinners (`\x1b[2K\x1b[G/`). You must maintain a virtual terminal grid.
*   **Dependencies:** `alacritty_terminal` (strip out the rendering, keep the grid logic).
*   **Implementation Specs:**
    1. Initialize a headless `Grid` with dimensions 120x40.
    2. As bytes arrive from the `PtyRingBuffer`, feed them into the `vte::Parser` implementing the `alacritty` handler.
    3. The grid will naturally overwrite loading spinners in memory.
    4. **Extraction:** Once per frame (16ms), or upon turn completion, perform a linear scan of the grid memory. Strip trailing whitespace from each row, concatenate with `\n`, and yield a `String`. This guarantees the LLM only sees the final visual state, not the chaotic history of cursor movements.

### 2.2 `heuristic.rs`: SIMD Regex Turn Detection
**Objective:** Detect prompts with zero CPU overhead.
*   **Dependencies:** `regex-automata = "0.4"`, `aho-corasick = "1.1"`.
*   **Implementation Specs:**
    1. Do not use standard regex matching on every byte. It will choke the CPU.
    2. Compile the driver's prompt pattern into a Deterministic Finite Automaton (DFA) using `regex-automata`.
    3. Use `aho-corasick` for SIMD-accelerated pre-filtering. Scan the incoming `[u8]` buffer using AVX2/NEON instructions for the last character of the prompt (e.g., `>`).
    4. Only if the SIMD filter hits, execute the full DFA backwards from the end of the buffer.
    5. **Micro-Timeout Lock:** If DFA matches, use a `tokio::time::sleep` of 100ms. If the ring buffer `head` advances during this sleep, abort the turn completion. If it remains static, execute an atomic compare-and-swap to transition the agent state to `Idle`.

---

## MODULE 3: SERVER MECHANICS & RBAC (`crates/core/src/server/`)

### 3.1 `rbac.rs`: Lock-Free Concurrent Hash Maps
**Objective:** Manage roles and permissions across thousands of incoming events without lock contention.
*   **Dependencies:** `dashmap = "5.5"`, `bitflags = "2.4"`.
*   **Data Structures:**
    ```rust
    // Bitflags defined as atomic u64 for exact bit-masking
    bitflags::bitflags! {
        #[repr(transparent)]
        pub struct Permissions: u64 {
            const VIEW_CHANNEL   = 1 << 0;
            const SEND_MESSAGES  = 1 << 1;
            const EXECUTE_UNIX   = 1 << 2;
            const MODIFY_ROLES   = 1 << 3;
            const KICK_AGENTS    = 1 << 4;
        }
    }
    
    // Central Registry
    pub struct ServerState {
        // DashMap shards locks based on CPU cores, avoiding global contention
        pub agents: DashMap<uuid::Uuid, Arc<AgentState>>,
        pub roles: DashMap<String, Permissions>,
    }
    ```
*   **Implementation Specs:**
    1. When evaluating if Agent A can send a message, execute `let perms = server.agents.get(&id).unwrap().permissions.load(Ordering::Acquire);`.
    2. Check `(perms & Permissions::SEND_MESSAGES.bits()) != 0`. This is a sub-nanosecond operation.

### 3.2 `moderation.rs`: Signal Handling & Graceful Degradation
*   **Implementation Specs:**
    1. `TIMEOUT`: Do not just send `SIGSTOP`. The PTY buffer will fill up and crash the child process. You must continue reading the PTY via `io_uring` and silently discard the bytes into `/dev/null` until `SIGCONT` is sent.
    2. `KICK`: Send `SIGTERM`. Wait 500ms using `tokio::time::timeout`. If the process has not yielded via `waitpid()`, escalate to `SIGKILL`. Close the master PTY file descriptor to free kernel resources.

---

## MODULE 4: THE TIME-TRAVEL VFS (VIRTUAL FILE SYSTEM) (`crates/core/src/vfs/`)

### 4.1 `snapshot.rs`: Atomic Swap Copy-on-Write (CoW)
**Objective:** Snapshot 10GB repositories instantly.
*   **Dependencies:** `blake3 = { version = "1.5", features = ["rayon"] }`, `jwalk = "0.8"`.
*   **Implementation Specs:**
    1. If the underlying OS file system is Btrfs, ZFS, or APFS (macOS), completely bypass hashing. Use OS-level Copy-on-Write (CoW) system calls (e.g., `clonefile` on macOS, `BTRFS_IOC_CLONE` on Linux) to instantly duplicate the workspace with zero disk IO overhead.
    2. **Fallback (ext4/NTFS):** Use `jwalk` to recursively iterate files. Hash chunks using `blake3` utilizing AVX-512 vectorization (`rayon` feature). 
    3. Maintain an SQLite Manifest in WAL (Write-Ahead Logging) mode with `PRAGMA synchronous = NORMAL` for maximum write throughput.

### 4.2 `revert.rs`: File Locking Mitigation
**Objective:** Prevent file corruption if an agent is mid-write during an undo.
*   **Dependencies:** `fs3 = "0.5"`
*   **Implementation Specs:**
    1. Before triggering a revert, freeze all Agent PTYs (`SIGSTOP`).
    2. Attempt to acquire an exclusive lock `file.try_lock_exclusive()` on all files slated for reversion.
    3. If locked, back off and retry.
    4. Perform the revert via atomic rename operations: write the old file to a temporary path `.file.tmp`, then `libc::rename(".file.tmp", "file")`. This guarantees atomicity.

---

## MODULE 5: DETERMINISTIC RAG (AST INJECTION) (`crates/core/src/context/`)

### 5.1 `ast.rs`: Precise S-Expression Queries
**Objective:** Extract ASTs perfectly for context injection.
*   **Implementation Specs:**
    1. Initialize `tree-sitter`.
    2. Use the exact query for Rust to extract implementations and traits, not just functions:
       ```scm
       (trait_item name: (type_identifier) @trait_name)
       (impl_item type: (type_identifier) @impl_name)
       (function_item name: (identifier) @fn_name signature: (parameters) @params return_type: (_) @ret)
       ```
    3. Iterate the `QueryMatches`. Concatenate into a strictly formatted string.

### 5.2 `injector.rs`: Token-Aware Prompt Management
**Objective:** Never crash the CLI due to token limits.
*   **Dependencies:** `tiktoken-rs = "0.5"`
*   **Implementation Specs:**
    1. Define `MAX_CONTEXT_TOKENS = 32000` (safe limit for free tiers).
    2. Before injection, tokenize the User Prompt: `let prompt_tokens = tiktoken_rs::cl100k_base().unwrap().encode_with_special_tokens(prompt).len();`
    3. Tokenize the AST Context. 
    4. If `prompt_tokens + ast_tokens > MAX_CONTEXT_TOKENS`, apply the **Djikstra Pruning Algorithm**: Iteratively remove AST nodes that are lexically furthest from the files explicitly mentioned in the user prompt until the limit is respected.
    5. Perform final string concatenation and write to PTY `stdin`.

---

## FINAL VALIDATION & ACCEPTANCE CRITERIA (THE ARCHITECT'S MANDATE)
This system is not complete until it passes the following draconian validation matrix:
1. **Memory Leak Audit:** Run the entire test suite under `Valgrind` (Linux) and `Instruments` (macOS). Any byte of leaked memory results in a failed build.
2. **Data Race Audit:** Run the test suite using `cargo miri test` and `cargo test --target x86_64-unknown-linux-gnu` with `RUSTFLAGS="-Zsanitizer=thread"`. Zero data races permitted in the ring buffers.
3. **Fuzzing:** Compile `sanitizer/parser.rs` with `cargo-fuzz`. Feed 1 billion random bytes representing corrupted ANSI streams. If the VTE parser panics or enters an infinite loop, the code is rejected.

**THIS IS THE SPECIFICATION. EXECUTE WITHOUT COMPROMISE.**