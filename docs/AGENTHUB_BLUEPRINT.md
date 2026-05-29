# AGENTHUB: THE DEFINITIVE ENGINEERING BLUEPRINT
### Version 1.0 — Complete Specification for Autonomous Implementation

**Classification:** Production Engineering Specification  
**Target Audience:** Autonomous AI Coding Agents & Senior Systems Engineers  
**Mandate:** Every section is self-contained, atomic, and directly implementable. No assumption, abstraction, or decision is left to the implementer. Completion of this document, word for word, yields a shippable, market-ready product.

---

## PART 0: PRODUCT IDENTITY & INVARIANTS

### 0.1 What AgentHub Is

AgentHub is a **Phantom Terminal Orchestrator** — a Rust-native, terminal-based application that acts as a single unified control surface for multiple free-tier AI CLI tools (Gemini CLI, Claude Code, Codex CLI, Aider, Cursor CLI, GitHub Copilot CLI, and any future CLI conforming to a Driver Profile).

It does not use any external APIs. It does not require subscriptions. It works by spawning each AI CLI as a hidden child process inside a Pseudo-Terminal (PTY), injecting prompts as raw keystrokes, reading and sanitizing the output streams, and routing all communication through a central message bus into a unified Terminal User Interface.

The product experience is modelled after Discord: the user is always the Server Admin, agents are members, and the workspace can be run in three distinct modes (DM, Group Chat, Server) with increasing levels of structure, governance, and control.

### 0.2 What AgentHub Is Not

- It is **not** an API wrapper. There are no HTTP clients for Anthropic, Google, or OpenAI.
- It is **not** a web app or Electron app. It runs entirely in the terminal.
- It is **not** a simple `tmux` wrapper. It parses, routes, sanitizes, and orchestrates I/O semantically.
- It is **not** non-deterministic. Every state transition is typed, logged, and reversible.

### 0.3 Absolute Engineering Laws (Never Violated)

These constraints apply to every module. Any code that violates them must be rewritten before the phase is considered complete.

1. **No Heap Allocation in Hot Paths.** The PTY read loop, stream parser, and message bus dispatcher must not allocate `Vec<u8>` or `String` on the heap per-event. Pre-allocate all buffers at startup using arena allocation (`bumpalo`) or statically sized ring buffers.
2. **No `std::sync::Mutex` in Async Code.** Use `tokio::sync::RwLock` for async-shared state. Use `crossbeam::epoch` or `DashMap` for concurrent data structures in the hot path.
3. **No `unwrap()` or `expect()` in Production Paths.** Every `Result` must be explicitly propagated or mapped to a structured `thiserror` error type. Panics are only permitted inside `#[cfg(test)]` blocks.
4. **No Zombie Processes.** Every `AgentPty` struct must implement `Drop` that sends `SIGTERM → SIGKILL` escalation and calls `waitpid()`.
5. **No API Keys, No Network Calls, No Telemetry.** The entire application operates locally. No data leaves the machine.
6. **All State Is Serializable.** Every runtime struct that represents durable state must derive `serde::Serialize` and `serde::Deserialize`. The application must be restartable to the exact prior state.
7. **`cargo clippy -- -D warnings` Must Pass.** Zero linter warnings are permitted before a phase is sealed.
8. **`cargo test --workspace` Must Pass.** Every module must have unit tests. Integration tests must pass end-to-end before a phase is sealed.

---

## PART 1: REPOSITORY STRUCTURE

The repository is a Cargo workspace. The following is the exact, final directory tree after all phases are complete.

```
agenthub/
├── Cargo.toml                        # Workspace root
├── Cargo.lock
├── .gitignore
├── README.md
├── about/
│   ├── VISION.md
│   └── ARCHITECTURE.md
├── docs/
│   ├── ROADMAP.md
│   ├── AGENT_MANUAL.md
│   └── USER_GUIDE.md
├── drivers/                          # Bundled CLI Driver Profiles (JSON)
│   ├── gemini.json
│   ├── claude.json
│   ├── codex.json
│   ├── aider.json
│   └── cursor.json
├── crates/
│   ├── agenthub/                     # Binary crate: main entry point
│   │   ├── Cargo.toml
│   │   └── src/
│   │       └── main.rs
│   ├── core/                         # Library: all business logic
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── error.rs
│   │       ├── config.rs
│   │       ├── pty/
│   │       │   ├── mod.rs
│   │       │   ├── manager.rs        # PTY lifecycle
│   │       │   ├── io.rs             # Ring buffer I/O
│   │       │   └── subagent.rs       # eBPF/ETW child process capture
│   │       ├── sanitizer/
│   │       │   ├── mod.rs
│   │       │   ├── parser.rs         # Headless grid ANSI parser
│   │       │   └── heuristic.rs      # Turn detection
│   │       ├── bus/
│   │       │   ├── mod.rs
│   │       │   └── router.rs         # Central message bus & tag routing
│   │       ├── server/
│   │       │   ├── mod.rs
│   │       │   ├── rbac.rs           # Roles, permissions, agent registry
│   │       │   ├── moderation.rs     # Mute/deafen/kick/timeout/ban
│   │       │   ├── induction.rs      # Agent initialization protocol
│   │       │   └── modes.rs          # DM / Group Chat / Server mode logic
│   │       ├── pipeline/
│   │       │   ├── mod.rs
│   │       │   ├── parser.rs         # Frankenstein pipe syntax parser
│   │       │   ├── executor.rs       # Pipeline execution engine
│   │       │   └── loop_engine.rs    # Autonomous agent loop (Sparring)
│   │       ├── vfs/
│   │       │   ├── mod.rs
│   │       │   ├── snapshot.rs       # Workspace checkpointing
│   │       │   └── revert.rs         # Undo / time-travel logic
│   │       ├── context/
│   │       │   ├── mod.rs
│   │       │   ├── indexer.rs        # Tree-sitter AST indexer
│   │       │   └── injector.rs       # Auto-context prompt injection
│   │       └── db/
│   │           ├── mod.rs
│   │           ├── sqlite.rs
│   │           └── migrations/
│   │               └── 001_initial_schema.sql
│   └── tui/                          # Library: all UI rendering
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs
│           ├── app.rs                # Root app state machine
│           ├── events.rs             # Keyboard/mouse event handling
│           ├── components/
│           │   ├── chat.rs           # Chat history pane
│           │   ├── input.rs          # Input box with history
│           │   ├── sidebar.rs        # Agent status panel
│           │   ├── racing.rs         # Split-pane LLM Racing UI
│           │   └── pipeline_viz.rs   # Live pipeline flow visualizer
│           └── theme.rs
├── tests/
│   ├── integration/
│   │   ├── pty_lifecycle.rs
│   │   ├── stream_sanitizer.rs
│   │   ├── rbac_moderation.rs
│   │   ├── pipeline_frankenstein.rs
│   │   ├── vfs_snapshot_revert.rs
│   │   └── context_injection.rs
│   └── fixtures/
│       └── mock_cli/                 # A simple Rust binary that mimics a real CLI for testing
│           └── src/
│               └── main.rs
└── .github/
    └── workflows/
        ├── ci.yml
        └── release.yml
```

---

## PART 2: CARGO WORKSPACE CONFIGURATION

### 2.1 Root `Cargo.toml`

```toml
[workspace]
members = [
    "crates/agenthub",
    "crates/core",
    "crates/tui",
]
resolver = "2"

[workspace.package]
version = "0.1.0"
edition = "2021"
authors = ["AgentHub Contributors"]
license = "MIT"

[workspace.dependencies]
# Async Runtime
tokio = { version = "1.40", features = ["full"] }
tokio-uring = { version = "0.4", optional = true }

# Concurrency
crossbeam = "0.8"
crossbeam-queue = "0.3"
dashmap = "5.5"
parking_lot = "0.12"

# PTY
portable-pty = "0.8"

# ANSI / Terminal Parsing
vte = "0.13"
alacritty_terminal = { git = "https://github.com/alacritty/alacritty", package = "alacritty_terminal" }

# Regex & Pattern Matching
regex = "1.10"
regex-automata = "0.4"
aho-corasick = "1.1"

# Serialization
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"

# Database
sqlx = { version = "0.7", features = ["sqlite", "runtime-tokio-native-tls", "migrate", "json"] }

# Filesystem / Hashing
blake3 = { version = "1.5", features = ["rayon"] }
jwalk = "0.8"
ignore = "0.4"
fs3 = "0.5"
tempfile = "3.10"

# Tree-Sitter / AST
tree-sitter = "0.22"
tree-sitter-rust = "0.21"
tree-sitter-python = "0.21"
tree-sitter-typescript = "0.21"
tree-sitter-go = "0.21"

# TUI
ratatui = "0.26"
crossterm = "0.27"

# Error Handling
thiserror = "1.0"
anyhow = "1.0"

# Utilities
uuid = { version = "1.8", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
bitflags = "2.4"
bumpalo = "3.16"
rayon = "1.10"
dirs = "5.0"

# eBPF (Linux only, optional)
[target.'cfg(target_os = "linux")'.workspace.dependencies]
aya = { version = "0.12", optional = true }

# Windows ETW (optional)
[target.'cfg(target_os = "windows")'.workspace.dependencies]
windows-sys = { version = "0.52", features = ["Win32_System_Diagnostics_Etw"] }
```

---

## PART 3: GLOBAL ERROR HANDLING (`crates/core/src/error.rs`)

This is the single error type for the entire `core` crate. It must be defined before any other module.

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AgentHubError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("PTY error: {0}")]
    Pty(String),

    #[error("Sanitizer error: {0}")]
    Sanitizer(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("RBAC error: agent {agent_id} lacks permission {permission}")]
    PermissionDenied { agent_id: uuid::Uuid, permission: String },

    #[error("Agent not found: {0}")]
    AgentNotFound(uuid::Uuid),

    #[error("Role not found: {0}")]
    RoleNotFound(String),

    #[error("Pipeline parse error at position {pos}: {msg}")]
    PipelineParse { pos: usize, msg: String },

    #[error("Pipeline execution error in stage {stage}: {msg}")]
    PipelineExecution { stage: usize, msg: String },

    #[error("VFS snapshot error: {0}")]
    Snapshot(String),

    #[error("VFS revert error: {0}")]
    Revert(String),

    #[error("Context injection error: {0}")]
    Context(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Driver profile error for CLI '{driver}': {msg}")]
    DriverProfile { driver: String, msg: String },

    #[error("Induction protocol timed out for agent {0}")]
    InductionTimeout(uuid::Uuid),

    #[error("Rate limit detected for agent {0}")]
    RateLimit(uuid::Uuid),

    #[error("Graph error: {0}")]
    Graph(#[from] GraphError),
}

#[derive(Error, Debug)]
pub enum GraphError {
    #[error("Cycle detected in pipeline graph")]
    CycleDetected,
    #[error("Node not found: {0}")]
    NodeNotFound(String),
}

// POSIX exit code mapping for deterministic shell integration
impl From<&AgentHubError> for u8 {
    fn from(e: &AgentHubError) -> u8 {
        match e {
            AgentHubError::Io(_) => 1,
            AgentHubError::Pty(_) => 2,
            AgentHubError::Database(_) => 3,
            AgentHubError::PermissionDenied { .. } => 13, // EACCES
            AgentHubError::AgentNotFound(_) => 4,
            AgentHubError::PipelineParse { .. } => 5,
            AgentHubError::PipelineExecution { .. } => 6,
            AgentHubError::Snapshot(_) => 7,
            AgentHubError::Revert(_) => 8,
            AgentHubError::InductionTimeout(_) => 9,
            AgentHubError::RateLimit(_) => 10,
            _ => 255,
        }
    }
}

pub type Result<T> = std::result::Result<T, AgentHubError>;
```

---

## PART 4: CONFIGURATION SYSTEM (`crates/core/src/config.rs`) ✅ SEALED

**Verification:** `cargo test -p agenthub-core --no-default-features --features config-tests config` · `cargo clippy -p agenthub-core --no-default-features --features config-tests -- -D warnings`

**DoD:**
- [x] `AgentHubConfig` load/save round-trip; defaults written on first run (`~/.agenthub/config.json`).
- [x] `AgentHubConfig::validate` enforces `log_level`, `max_agents`, `theme`, and non-empty paths.
- [x] `DriverProfile::validate` enforces regex fields, `NO_COLOR`/`TERM=dumb`, name/display/executable, and `silence_timeout_ms > 0`.
- [x] Driver loader: user `drivers_dir` first, then bundled `drivers/` fallback; profile `name` must match filename.
- [x] Bundled profiles (`gemini`, `claude`, `codex`, `aider`, `cursor`) parse and match blueprint examples where specified.

### 4.1 Global Configuration Schema

The configuration file lives at `~/.agenthub/config.json`. It is loaded at startup and watched for changes via `notify`.

```rust
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Root configuration file: ~/.agenthub/config.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentHubConfig {
    /// Default workspace mode on launch.
    pub default_mode: WorkspaceMode,
    /// Path to the drivers directory. Defaults to the bundled `drivers/` folder.
    pub drivers_dir: PathBuf,
    /// Path to the SQLite database. Defaults to `~/.agenthub/agenthub.db`.
    pub db_path: PathBuf,
    /// Path where VFS snapshots are stored. Defaults to `.agenthub_shadow/` in the CWD.
    pub shadow_dir: PathBuf,
    /// Maximum number of concurrent agent PTYs. Default: 16.
    pub max_agents: u8,
    /// Global log level. One of: "trace", "debug", "info", "warn", "error". Default: "info".
    pub log_level: String,
    /// Theme selection for the TUI. Default: "dark".
    pub theme: String,
    /// Custom key bindings. Keys are action names, values are key strings.
    pub keybindings: std::collections::HashMap<String, String>,
}

impl Default for AgentHubConfig {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        Self {
            default_mode: WorkspaceMode::GroupChat,
            drivers_dir: home.join(".agenthub").join("drivers"),
            db_path: home.join(".agenthub").join("agenthub.db"),
            shadow_dir: PathBuf::from(".agenthub_shadow"),
            max_agents: 16,
            log_level: "info".to_string(),
            theme: "dark".to_string(),
            keybindings: std::collections::HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceMode {
    /// 1-on-1 session. Minimal UI. No RBAC. No broadcast.
    DirectMessage,
    /// Small group, unconstrained. Broadcast enabled. Minimal governance.
    GroupChat,
    /// Full hierarchy: channels, roles, admin controls, structured moderation.
    Server,
}
```

### 4.2 Driver Profile Schema

Each CLI tool is described by a Driver Profile JSON file. The bundled profiles live in `drivers/`. Users can add custom profiles to `~/.agenthub/drivers/`.

**Full schema (`DriverProfile`):**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriverProfile {
    /// Unique machine-readable name. E.g., "gemini", "claude".
    pub name: String,
    /// Human-readable display name. E.g., "Gemini CLI".
    pub display_name: String,
    /// The executable to spawn. Must be on PATH or an absolute path.
    pub executable: String,
    /// Arguments passed to the executable on launch. E.g., ["--no-color"].
    pub args: Vec<String>,
    /// Environment variables to set for this process. E.g., {"NO_COLOR": "1"}.
    pub env: std::collections::HashMap<String, String>,
    /// Regex pattern (Rust syntax) that matches the CLI's input prompt.
    /// When this pattern is found at the end of the sanitized output buffer,
    /// the agent is considered to have finished its turn.
    /// Example: r"^>\s*$" for a CLI that shows "> " when ready.
    pub prompt_regex: String,
    /// Maximum milliseconds of output silence after which the turn is forcibly
    /// declared complete. Default: 3000ms.
    pub silence_timeout_ms: u64,
    /// Text to inject immediately after launch to suppress welcome screens,
    /// changelogs, and setup wizards. Each string is injected sequentially
    /// with a 200ms delay between them.
    pub init_sequence: Vec<String>,
    /// Known rate-limit error substrings. If the sanitized output contains
    /// any of these, the agent is marked as RateLimited and the user is alerted.
    pub rate_limit_patterns: Vec<String>,
    /// Known interactive prompt patterns that require an automated response.
    /// Key: regex to detect the prompt. Value: the string to inject.
    /// Example: { "Continue\\? \\[Y/n\\]": "Y\n" }
    pub auto_reply_patterns: std::collections::HashMap<String, String>,
    /// Whether this CLI is known to support multiple simultaneous instances.
    /// If false, spawning more than one instance will show a warning.
    pub supports_multi_instance: bool,
    /// Maximum number of instances allowed. 0 = unlimited.
    pub max_instances: u8,
}
```

**Example: `drivers/gemini.json`**

```json
{
  "name": "gemini",
  "display_name": "Gemini CLI",
  "executable": "gemini",
  "args": [],
  "env": { "NO_COLOR": "1", "TERM": "dumb" },
  "prompt_regex": "^>\\s*$",
  "silence_timeout_ms": 5000,
  "init_sequence": [],
  "rate_limit_patterns": ["rate limit", "quota exceeded", "429"],
  "auto_reply_patterns": {
    "Do you want to continue\\? \\[Y/n\\]": "Y\n",
    "Press Enter to continue": "\n"
  },
  "supports_multi_instance": true,
  "max_instances": 0
}
```

**Example: `drivers/claude.json`**

```json
{
  "name": "claude",
  "display_name": "Claude Code",
  "executable": "claude",
  "args": [],
  "env": { "NO_COLOR": "1", "TERM": "dumb" },
  "prompt_regex": "^\\?\\s*$",
  "silence_timeout_ms": 8000,
  "init_sequence": [],
  "rate_limit_patterns": ["rate limit", "overloaded", "529", "429"],
  "auto_reply_patterns": {
    "Continue\\? \\(Y/n\\)": "Y\n"
  },
  "supports_multi_instance": true,
  "max_instances": 0
}
```

---

## PART 5: THE PTY ENGINE (`crates/core/src/pty/`)

### 5.1 Core Data Structures (`pty/manager.rs`)

```rust
use std::sync::atomic::{AtomicU32, AtomicU8, Ordering};
use uuid::Uuid;

/// Represents the lifecycle state of a single agent PTY.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PtyStatus {
    /// Spawning process, running induction protocol.
    Initializing = 0,
    /// Online and waiting for input.
    Idle = 1,
    /// Currently generating a response.
    Thinking = 2,
    /// Output silenced by admin /mute. Process is alive but output is suppressed.
    Muted = 3,
    /// Input blocked by admin /deafen. Receives no broadcast messages.
    Deafened = 4,
    /// Process suspended (SIGSTOP). Used during /timeout.
    Suspended = 5,
    /// Process has been killed. Terminal state.
    Dead = 6,
    /// CLI returned a rate-limit error. Auto-retry or user intervention needed.
    RateLimited = 7,
}

/// Cache-line aligned to prevent false sharing between reader and writer threads.
#[repr(C, align(64))]
pub struct AgentPty {
    /// Unique identifier for this agent instance.
    pub id: Uuid,
    /// Display tag used in the UI and for @-mentions. E.g., "gemini-1".
    pub tag: String,
    /// Name of the driver profile used to spawn this agent.
    pub driver_name: String,
    /// OS process ID of the spawned child.
    pub pid: u32,
    /// Current lifecycle status. Updated atomically.
    pub status: AtomicU8,
    /// Bitmask of permission flags. See `rbac::Permissions`.
    pub role_mask: AtomicU32,
    /// Internal handle to the PTY master. Used for writing stdin and reading stdout.
    /// Wrapped in an Option so it can be taken during Drop for clean shutdown.
    pub master: std::sync::Mutex<Option<Box<dyn portable_pty::MasterPty + Send>>>,
    /// Whether this agent's output is being broadcast to other agents.
    /// Deafening sets this to false.
    pub receives_broadcast: std::sync::atomic::AtomicBool,
    /// Whether this agent's output appears in the main chat pane.
    /// Muting sets this to false.
    pub visible_in_chat: std::sync::atomic::AtomicBool,
}

impl Drop for AgentPty {
    fn drop(&mut self) {
        // Attempt graceful shutdown first.
        #[cfg(unix)]
        unsafe {
            libc::kill(self.pid as i32, libc::SIGTERM);
        }
        // Give 500ms for graceful exit, then force kill.
        // Note: The actual timeout logic is in moderation.rs kill_agent().
        // This Drop is a last-resort safety net.
        #[cfg(unix)]
        unsafe {
            std::thread::sleep(std::time::Duration::from_millis(100));
            libc::kill(self.pid as i32, libc::SIGKILL);
        }
        // Release the master PTY handle, which closes the file descriptor.
        if let Ok(mut master_guard) = self.master.lock() {
            let _ = master_guard.take();
        }
    }
}
```

### 5.2 PTY Spawning Logic (`pty/manager.rs` — `spawn_agent` function)

This function is the single entry point for creating a new agent. It is called when the user adds an agent in the TUI.

**Complete implementation spec:**

```rust
/// Spawns a new agent CLI inside an isolated PTY.
///
/// Steps:
/// 1. Load and validate the DriverProfile for `driver_name`.
/// 2. Check `max_agents` limit and `driver.max_instances` limit.
/// 3. Create a PTY pair using `portable_pty::native_pty_system()`.
/// 4. Configure the PTY size to 220 columns x 50 rows to accommodate
///    wide outputs without forced line wrapping.
/// 5. Set environment variables from `driver.env` plus:
///    - `TERM=dumb` (prevents color output)
///    - `NO_COLOR=1` (standard convention for disabling colors)
///    - `AGENTHUB=1` (allows driver profiles to detect they are managed)
/// 6. Spawn the child process using `pty_pair.slave.spawn_command(cmd)`.
/// 7. Close the slave PTY in the parent process immediately after spawning.
///    (Keeping it open will prevent EOF detection.)
/// 8. Move the master PTY handle into an `Arc<AgentPty>`.
/// 9. Spawn two Tokio tasks:
///    a. `pty_reader_task`: Continuously reads from master PTY into the
///       agent's `PtyRingBuffer`. (See io.rs)
///    b. `sanitizer_task`: Consumes from the ring buffer, runs the ANSI
///       parser, and emits `AgentMessage` events onto the central bus.
/// 10. Insert the `Arc<AgentPty>` into `ServerState.agents`.
/// 11. Spawn the `induction_task` (See induction.rs).
/// 12. Return the agent's `Uuid`.
pub async fn spawn_agent(
    driver_name: &str,
    config: &AgentHubConfig,
    server_state: Arc<ServerState>,
    bus_tx: tokio::sync::broadcast::Sender<BusEvent>,
) -> Result<Uuid>;
```

### 5.3 Ring Buffer I/O (`pty/io.rs`)

The ring buffer is the zero-allocation communication channel between the PTY reader and the ANSI parser.

```rust
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Lock-free single-producer, single-consumer ring buffer.
/// Producer: PTY reader task. Consumer: ANSI sanitizer task.
/// Size must be a power of 2.
pub struct PtyRingBuffer {
    buffer: UnsafeCell<[u8; 65536]>, // 64KB — fits in L2 cache
    head: AtomicUsize,               // Written by producer
    tail: AtomicUsize,               // Written by consumer
}

unsafe impl Send for PtyRingBuffer {}
unsafe impl Sync for PtyRingBuffer {}

impl PtyRingBuffer {
    pub const fn new() -> Self {
        Self {
            buffer: UnsafeCell::new([0u8; 65536]),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    /// Write bytes into the buffer. Returns the number of bytes written.
    /// If the buffer is full, blocks until space is available.
    /// Called exclusively from the PTY reader task (single producer).
    pub fn write(&self, data: &[u8]) -> usize {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        let capacity = 65536;
        let available = capacity - (head.wrapping_sub(tail));
        let to_write = data.len().min(available);
        if to_write == 0 { return 0; }
        let buf = unsafe { &mut *self.buffer.get() };
        for (i, &byte) in data[..to_write].iter().enumerate() {
            buf[(head + i) % capacity] = byte;
        }
        self.head.store(head.wrapping_add(to_write), Ordering::Release);
        to_write
    }

    /// Read up to `dest.len()` bytes from the buffer.
    /// Returns the number of bytes read.
    /// Called exclusively from the sanitizer task (single consumer).
    pub fn read(&self, dest: &mut [u8]) -> usize {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        let available = head.wrapping_sub(tail);
        let to_read = dest.len().min(available);
        if to_read == 0 { return 0; }
        let buf = unsafe { &*self.buffer.get() };
        for i in 0..to_read {
            dest[i] = buf[(tail + i) % 65536];
        }
        self.tail.store(tail.wrapping_add(to_read), Ordering::Release);
        to_read
    }

    pub fn is_empty(&self) -> bool {
        self.head.load(Ordering::Acquire) == self.tail.load(Ordering::Relaxed)
    }
}
```

**PTY Reader Task Implementation:**

```rust
/// Runs as a dedicated Tokio task per agent.
/// Reads raw bytes from the PTY master and writes them to the ring buffer.
/// On EOF or error, sets the agent status to Dead and broadcasts a
/// AgentOfflineEvent onto the bus.
pub async fn pty_reader_task(
    master_pty: Arc<AgentPty>,
    ring_buffer: Arc<PtyRingBuffer>,
    bus_tx: tokio::sync::broadcast::Sender<BusEvent>,
) {
    let mut raw_buf = [0u8; 4096];
    loop {
        // Check if agent has been killed
        if master_pty.status.load(Ordering::Acquire) == PtyStatus::Dead as u8 {
            break;
        }
        // Use blocking read in a spawn_blocking context to avoid blocking the async runtime
        let bytes_read = {
            let master_guard = master_pty.master.lock().unwrap();
            if let Some(ref master) = *master_guard {
                // Read is blocking — run in spawn_blocking
                match master.try_read(&mut raw_buf) {
                    Ok(n) => n,
                    Err(_) => break,
                }
            } else {
                break;
            }
        };

        if bytes_read == 0 {
            // EOF: process has exited
            master_pty.status.store(PtyStatus::Dead as u8, Ordering::Release);
            let _ = bus_tx.send(BusEvent::AgentOffline { id: master_pty.id });
            break;
        }

        // Write into ring buffer. If full, wait 1ms and retry.
        let mut written = 0;
        while written < bytes_read {
            let w = ring_buffer.write(&raw_buf[written..bytes_read]);
            if w == 0 {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            } else {
                written += w;
            }
        }
    }
}
```

### 5.4 Subagent Capture (`pty/subagent.rs`)

When an agent (e.g., Aider) spawns its own child process, AgentHub must detect it and bring it into the server as a distinct, manageable participant.

**Linux implementation (eBPF-based, preferred):**

```
Dependency: aya = "0.12" (feature-gated: cfg(target_os = "linux"))

Implementation:
1. At startup, load the eBPF program (compiled from a C source using aya-build).
   The eBPF program attaches to the `sched_process_exec` kernel tracepoint.
2. The eBPF program runs in kernel space. It checks:
   if (task->real_parent->pid == watched_pid) { bpf_ringbuf_output(...) }
3. In user space, the `subagent_watcher_task` polls the BPF ring buffer.
4. When a new child PID is received, AgentHub:
   a. Reads /proc/{pid}/exe to get the executable name.
   b. Attempts to open /proc/{pid}/fd/1 (stdout) for reading.
      NOTE: This will fail if the child has already dup2'd its stdout to the
      parent PTY slave. In that case, the output will naturally flow through
      the parent's ring buffer and be marked with the parent agent's tag.
   c. If stdout is accessible and separate, wrap it in a new PtyRingBuffer,
      create a new AgentPty stub with tag "@{parent_tag}-sub-{n}", assign it
      the built-in "Subagent" role, and start a sanitizer task for it.
   d. Announce the new agent in the chat: "[System]: @{parent_tag} spawned
      subagent @{parent_tag}-sub-1. It has been registered in the server."
```

**macOS / Windows implementation (polling fallback):**

```
Because eBPF is Linux-only, use a polling approach on other platforms.

Every 250ms, call `sysinfo::System::refresh_processes()`.
Filter for processes whose parent PID matches any active AgentPty.pid.
For newly found processes not yet registered, attempt to attach via
ptrace (macOS) or CreateRemoteThread (Windows — limited support).

Limitation: Short-lived subagents may be missed. This is documented in
docs/USER_GUIDE.md as a known limitation on non-Linux platforms.
```

---

## PART 6: STREAM SANITIZER (`crates/core/src/sanitizer/`)

### 6.1 Headless Grid ANSI Parser (`sanitizer/parser.rs`)

Naive ANSI stripping (removing escape sequences) is insufficient. A CLI using a loading spinner writes `\r` (carriage return) to overwrite the same line repeatedly. A naive stripper produces garbage like `/---\|/---\|done`. The correct approach is to emulate a real terminal: maintain a virtual grid in memory, write bytes into it exactly as a terminal emulator would, and then read the final visual state of the grid.

```
Implementation:
1. Create a `VirtualGrid` struct containing:
   - `cells: Vec<Vec<char>>` — a 220-column x 50-row grid of characters
   - `cursor_row: usize`
   - `cursor_col: usize`
   - `vte_parser: vte::Parser` — the ANSI escape code parser

2. Implement `vte::Perform` for `VirtualGrid`. Handle:
   - `print(c: char)`: Write `c` to `cells[cursor_row][cursor_col]`.
     Increment cursor_col. If cursor_col >= 220, wrap to next row.
   - `execute(byte: u8)`:
     - `\r` (0x0D): Set cursor_col = 0.
     - `\n` (0x0A): Increment cursor_row. If cursor_row >= 50, scroll:
       remove the first row, push a new empty row.
     - `\x08` (backspace): Decrement cursor_col (min 0).
     - `\x07` (bell): Ignore.
   - `csi_dispatch(params, ..., action)`:
     - `A` (cursor up N): cursor_row -= N
     - `B` (cursor down N): cursor_row += N
     - `C` (cursor forward N): cursor_col += N
     - `D` (cursor back N): cursor_col -= N
     - `G` (cursor column): cursor_col = param[0] - 1
     - `H` / `f` (cursor position): cursor_row = param[0]-1, cursor_col = param[1]-1
     - `J` (erase display): param 2 = clear entire grid
     - `K` (erase line):
       - param 0: clear from cursor to end of line
       - param 1: clear from start of line to cursor
       - param 2: clear entire line
     - `m` (SGR / colors): Ignore all color and style codes entirely.
   - All other sequences: Ignore.

3. The `extract_text()` method:
   - Iterate rows from 0 to cursor_row (inclusive).
   - For each row, collect chars, rtrim whitespace.
   - Join rows with '\n'.
   - ltrim the entire result (remove leading blank lines).
   - Return the resulting String.
   - This is called by the sanitizer task after turn detection.
```

### 6.2 Turn Detection (`sanitizer/heuristic.rs`)

The sanitizer must know when an agent has finished generating and is waiting for input. Since we have no API, we use heuristics.

**Primary method: Prompt Regex Match**

```
1. After each batch of bytes is written to the virtual grid, call extract_text().
2. Get the last non-empty line of the extracted text.
3. Compile the driver's `prompt_regex` into a `regex::Regex` at agent spawn time
   (not on every check).
4. Run `prompt_regex.is_match(last_line)`.
5. If it matches:
   a. Start a 100ms confirmation timer using `tokio::time::sleep`.
   b. If the ring buffer's `head` advances during those 100ms (more bytes arrived),
      the agent is still generating. Cancel the timer and continue.
   c. If the `head` does not advance during those 100ms, confirm turn completion.
   d. Call `extract_text()` one final time to get the full, clean output.
   e. Emit `BusEvent::AgentMessage` with the sanitized text.
   f. Set agent status to `Idle`.
```

**Secondary method: Silence Timeout**

```
If the prompt regex never matches (misconfigured driver or unexpected CLI behavior):
1. Track `last_byte_timestamp: Instant` — updated every time bytes are read.
2. A background task checks every 500ms:
   if last_byte_timestamp.elapsed() > driver.silence_timeout_ms AND
   agent.status == Thinking:
       Emit BusEvent::AgentMessage with current extract_text() output.
       Log a warning: "[Warning]: Turn completed via silence timeout for @{tag}.
        Consider updating the driver's prompt_regex."
       Set agent status to Idle.
```

**Auto-reply to interactive prompts:**

```
After each extract_text() call, check the last line against all patterns
in `driver.auto_reply_patterns`.
If a match is found:
  1. Do NOT emit a BusEvent::AgentMessage for this output.
  2. Write the corresponding reply string directly to the PTY stdin.
  3. Log to the debug log: "[Auto-reply]: Sent '{reply}' to @{tag} for prompt '{pattern}'"
```

---

## PART 7: CENTRAL MESSAGE BUS (`crates/core/src/bus/`)

The message bus is the nervous system of AgentHub. All communication between agents, the user, and the TUI flows through it as typed events.

### 7.1 Event Types (`bus/router.rs`)

```rust
use uuid::Uuid;
use chrono::{DateTime, Utc};

/// Every event that flows through the AgentHub system.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum BusEvent {
    // ── Agent Lifecycle ──────────────────────────────────────────────────
    /// An agent has completed induction and is now online.
    AgentOnline { id: Uuid, tag: String, role: String },
    /// An agent's process has died (crash, kick, or natural exit).
    AgentOffline { id: Uuid, tag: String, reason: OfflineReason },
    /// An agent's status has changed (e.g., Idle → Thinking).
    AgentStatusChanged { id: Uuid, old: u8, new: u8 },
    /// A subagent was detected and registered.
    SubagentDetected { parent_id: Uuid, child_id: Uuid, child_tag: String },

    // ── Messages ─────────────────────────────────────────────────────────
    /// A message from a human user.
    UserMessage {
        content: String,
        timestamp: DateTime<Utc>,
        target: MessageTarget,
    },
    /// A sanitized, complete message from an agent after turn detection.
    AgentMessage {
        id: Uuid,
        tag: String,
        content: String,
        timestamp: DateTime<Utc>,
    },
    /// A system notification displayed in the chat (italicized, grey).
    SystemMessage { content: String, timestamp: DateTime<Utc> },

    // ── Moderation ───────────────────────────────────────────────────────
    AgentMuted { id: Uuid, by: String },
    AgentUnmuted { id: Uuid, by: String },
    AgentDeafened { id: Uuid, by: String },
    AgentUndeafened { id: Uuid, by: String },
    AgentTimedOut { id: Uuid, duration_secs: u64, by: String },
    AgentKicked { id: Uuid, reason: Option<String>, by: String },
    AgentBanned { id: Uuid, driver_name: String, by: String },
    RoleAssigned { agent_id: Uuid, role: String, by: String },

    // ── Pipelines ────────────────────────────────────────────────────────
    PipelineStarted { pipeline_id: Uuid, definition: String },
    PipelineStageComplete { pipeline_id: Uuid, stage: usize, output_preview: String },
    PipelineFailed { pipeline_id: Uuid, stage: usize, error: String },
    PipelineComplete { pipeline_id: Uuid },

    // ── VFS / Time-Travel ────────────────────────────────────────────────
    SnapshotCreated { snapshot_id: Uuid, file_count: usize },
    RevertInitiated { snapshot_id: Uuid },
    RevertComplete { snapshot_id: Uuid },

    // ── System ───────────────────────────────────────────────────────────
    ModeChanged { old: WorkspaceModeRepr, new: WorkspaceModeRepr },
    RateLimitDetected { id: Uuid, tag: String },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum MessageTarget {
    /// Message goes to all non-deafened agents.
    Broadcast,
    /// Message goes to a specific agent only.
    Direct(Uuid),
    /// Message goes to multiple specific agents.
    Multi(Vec<Uuid>),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum OfflineReason {
    Crashed,
    Kicked,
    Banned,
    Natural, // Process exited on its own (e.g., user typed "exit")
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum WorkspaceModeRepr { Dm, GroupChat, Server }
```

### 7.2 Message Bus Implementation

```
The bus uses tokio::sync::broadcast with a channel capacity of 1024.

The central `BusRouter` task:
1. Receives all events from all producers (user input, agent sanitizer tasks,
   moderation commands, pipeline executor, VFS).
2. For UserMessage and AgentMessage events, applies broadcast logic:
   a. Collect all agents in ServerState.agents.
   b. Filter to agents where:
      - status != Dead
      - status != Suspended
      - receives_broadcast == true (not deafened)
      - The message is not from the agent itself
   c. For each qualifying agent, construct the injection string:
      "[{sender_tag} says]: {content}\n"
   d. Write the injection string to the agent's PTY stdin using
      the master PTY handle.
3. Logs every event to SQLite via the db module.
4. Forwards all events to the TUI via a separate mpsc channel.

Chaos Heuristics (Group Chat and Server modes only):
When multiple agents are simultaneously in Thinking status and producing
output, the broadcast injector applies a staggered delay:
- Sort agents by UUID (deterministic ordering).
- Inject to agent[0] immediately.
- Inject to agent[1] after 150ms.
- Inject to agent[2] after 300ms.
- And so on.
This prevents agents from reading a half-formed broadcast and producing
inconsistent responses.
```

---

## PART 8: SERVER MECHANICS & RBAC (`crates/core/src/server/`)

### 8.1 Permissions System (`server/rbac.rs`)

```rust
use bitflags::bitflags;

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
    pub struct Permissions: u64 {
        /// Can see messages in the chat.
        const VIEW_CHANNEL          = 1 << 0;
        /// Can generate and send messages.
        const SEND_MESSAGES         = 1 << 1;
        /// Output is broadcast to other agents.
        const BROADCAST_OUTPUT      = 1 << 2;
        /// Can receive broadcasts from other agents.
        const RECEIVE_BROADCAST     = 1 << 3;
        /// Can execute Unix commands via the pipeline `>` syntax.
        const EXECUTE_UNIX          = 1 << 4;
        /// Can write files to the workspace.
        const WRITE_FILES           = 1 << 5;
        /// Can trigger pipeline handoffs to other agents.
        const TRIGGER_PIPELINE      = 1 << 6;
        /// Can override or veto outputs from other agents (Leader role).
        const OVERRIDE_OTHERS       = 1 << 7;
        /// Can call /promote and /demote (Moderator role).
        const MODIFY_ROLES          = 1 << 8;
        /// Can spawn additional agent instances.
        const SPAWN_AGENTS          = 1 << 9;
    }
}

/// Built-in role definitions. These cannot be deleted, only overridden.
pub fn default_roles() -> std::collections::HashMap<String, Permissions> {
    use Permissions as P;
    [
        ("Leader", P::VIEW_CHANNEL | P::SEND_MESSAGES | P::BROADCAST_OUTPUT
            | P::RECEIVE_BROADCAST | P::EXECUTE_UNIX | P::WRITE_FILES
            | P::TRIGGER_PIPELINE | P::OVERRIDE_OTHERS | P::MODIFY_ROLES),
        ("Builder", P::VIEW_CHANNEL | P::SEND_MESSAGES | P::BROADCAST_OUTPUT
            | P::RECEIVE_BROADCAST | P::EXECUTE_UNIX | P::WRITE_FILES
            | P::TRIGGER_PIPELINE),
        ("Reviewer", P::VIEW_CHANNEL | P::SEND_MESSAGES | P::BROADCAST_OUTPUT
            | P::RECEIVE_BROADCAST),
        ("Auditor", P::VIEW_CHANNEL | P::SEND_MESSAGES | P::RECEIVE_BROADCAST),
        ("Moderator", P::VIEW_CHANNEL | P::SEND_MESSAGES | P::BROADCAST_OUTPUT
            | P::RECEIVE_BROADCAST | P::MODIFY_ROLES),
        ("Subagent", P::VIEW_CHANNEL | P::SEND_MESSAGES | P::BROADCAST_OUTPUT
            | P::EXECUTE_UNIX | P::WRITE_FILES),
        ("Observer", P::VIEW_CHANNEL),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), *v))
    .collect()
}
```

**Custom Role Storage:**

Custom roles are stored in `~/.agenthub/roles.json`. Schema:

```json
[
  {
    "name": "SecurityAuditor",
    "permissions": ["VIEW_CHANNEL", "SEND_MESSAGES", "RECEIVE_BROADCAST"],
    "induction_prompt_override": "You are a security-focused reviewer. Your ONLY job is to find security vulnerabilities. You must ignore all feature requests and focus exclusively on injection attacks, data leakage, authentication flaws, and insecure dependencies."
  }
]
```

### 8.2 Agent State (`server/rbac.rs`)

```rust
/// Full runtime state for one agent. Stored in ServerState.agents.
#[derive(Debug)]
pub struct AgentState {
    pub id: Uuid,
    pub tag: String,
    pub driver_name: String,
    pub role: String,
    pub permissions: Permissions,
    /// Instance number. If this is the 2nd Gemini, instance_number = 2.
    pub instance_number: u8,
    /// Timestamp of when the agent came online.
    pub online_since: chrono::DateTime<chrono::Utc>,
    /// Unix timestamp until which the agent is timed out. 0 = not timed out.
    pub timeout_until: std::sync::atomic::AtomicI64,
    /// Whether this agent is permanently banned (its driver is blocklisted
    /// for this session).
    pub banned: bool,
}
```

**ServerState:**

```rust
pub struct ServerState {
    /// All active agents. Key: agent Uuid.
    pub agents: dashmap::DashMap<Uuid, Arc<AgentState>>,
    /// Roles available in this session (default + custom).
    pub roles: dashmap::DashMap<String, Permissions>,
    /// Role induction prompt overrides. Key: role name. Value: custom prompt text.
    pub role_induction_overrides: dashmap::DashMap<String, String>,
    /// Set of banned driver names for this session.
    pub banned_drivers: dashmap::DashSet<String>,
    /// Current workspace mode.
    pub mode: std::sync::atomic::AtomicU8,
}
```

### 8.3 Moderation Commands (`server/moderation.rs`)

Every slash command is parsed from TUI input. The parsing logic in `tui/events.rs` detects lines starting with `/` and routes them to `moderation::execute_command`. All commands emit `BusEvent` events for chat display and SQLite logging.

**Complete command specification:**

```
/mute @{tag}
  Effect: Sets agent.visible_in_chat = false.
          Output from this agent is still captured and processed internally
          but is not rendered in the main chat pane.
  Emits:  BusEvent::AgentMuted
  Error:  AgentNotFound if tag does not match any active agent.

/unmute @{tag}
  Effect: Sets agent.visible_in_chat = true.
  Emits:  BusEvent::AgentUnmuted

/deafen @{tag}
  Effect: Sets agent.receives_broadcast = false.
          The agent's PTY stdin will no longer receive injected messages
          from other agents or broadcast user messages.
          The agent continues to receive direct @-mentions from the user.
  Emits:  BusEvent::AgentDeafened

/undeafen @{tag}
  Effect: Sets agent.receives_broadcast = true.
  Emits:  BusEvent::AgentUndeafened

/timeout @{tag} {duration}
  Duration format: "30s", "5m", "2h".
  Effect: Sends SIGSTOP to the agent's process (Unix).
          On Windows: calls SuspendThread for all threads in the process.
          While suspended, the PTY reader task continues draining the buffer
          into /dev/null to prevent the kernel buffer from filling.
          Starts a tokio::time::sleep for the duration. On expiry,
          sends SIGCONT (Unix) / ResumeThread (Windows) and sets
          status back to Idle.
  Emits:  BusEvent::AgentTimedOut

/kick @{tag} [reason]
  Effect: 1. Sets status to Dead.
          2. Sends SIGTERM to the process.
          3. Waits 500ms via tokio::time::timeout.
          4. If process has not exited (checked via waitpid with WNOHANG),
             sends SIGKILL.
          5. Calls waitpid to reap the zombie.
          6. Closes the master PTY file descriptor.
          7. Removes the agent from ServerState.agents.
  Emits:  BusEvent::AgentKicked

/ban @{tag} [reason]
  Effect: Executes /kick, then adds the agent's driver_name to
          ServerState.banned_drivers. Any future attempt to spawn an agent
          with that driver_name this session will be rejected with an error.
          Ban is session-only (does not persist across restarts).
  Emits:  BusEvent::AgentBanned

/promote @{tag} to {role}
  Effect: Looks up {role} in ServerState.roles.
          Updates agent.role and agent.permissions to match the new role.
          Injects a system message into the agent's PTY:
          "[System]: Your role has been changed to {role}. New behavior:
           {role_induction_override or role description}."
  Emits:  BusEvent::RoleAssigned
  Error:  RoleNotFound if {role} does not exist.
          PermissionDenied if the calling user's session lacks MODIFY_ROLES.

/demote @{tag}
  Effect: Reverts agent to the "Observer" role (VIEW_CHANNEL only).
  Emits:  BusEvent::RoleAssigned

/addrole {name} [permissions...]
  Effect: Creates a new role entry in ServerState.roles and persists
          it to ~/.agenthub/roles.json.
          Permissions are specified as a space-separated list of flag names.
          Example: /addrole SecurityAuditor VIEW_CHANNEL SEND_MESSAGES
  Error:  Config error if role name conflicts with a built-in role.

/removerole {name}
  Effect: Deletes the role from ServerState.roles and from
          ~/.agenthub/roles.json.
  Error:  Config error if attempting to delete a built-in role.

/spawn {driver} [--role {role}] [--tag {custom_tag}]
  Effect: Calls pty::manager::spawn_agent() with the given driver.
          Optional: assign a role immediately. Default role: "Builder".
          Optional: override the auto-generated tag.
          If driver.supports_multi_instance == false and an instance
          already exists, show a confirmation prompt before proceeding.
          If driver.max_instances > 0 and the limit is reached, reject.

/mode {dm|groupchat|server}
  Effect: Changes ServerState.mode.
          In DM mode: If more than 1 agent is active, prompt user to choose
          which one to keep. Others are deafened.
          In GroupChat mode: All moderation is lifted (mutes/deafens remain
          but no structural channels or role restrictions are enforced).
          In Server mode: Full RBAC enforcement begins. Agents without
          SEND_MESSAGES permission can no longer generate output.
  Emits:  BusEvent::ModeChanged

/setprompt @{tag} {custom_prompt}
  Effect: Injects {custom_prompt} directly into the agent's PTY stdin
          as a system instruction. Used to re-contextualize an agent
          mid-session without kicking and re-spawning.
```

### 8.4 Grand Induction Protocol (`server/induction.rs`)

When an agent is spawned, before it is shown as "Online" in the UI, it must receive a comprehensive context prompt that explains the AgentHub environment, its assigned role, and behavioral expectations.

**Full induction prompt template:**

```
You are now running inside AgentHub, a multi-agent orchestration system.

=== YOUR IDENTITY ===
Your name in this session is: @{tag}
Your assigned role is: {role}
Your role's mandate is: {role_description_or_override}

=== ENVIRONMENT ===
AgentHub manages multiple AI CLI tools simultaneously. You are one of {total_agents} active agents.
Other agents currently online: {list_of_other_agent_tags_and_roles}
Workspace directory: {cwd}
Workspace mode: {mode}

=== COMMUNICATION PROTOCOL ===
- When you see a message starting with "[{AgentName} says]: ", it means another agent or the user has addressed the group. Read and consider it carefully.
- When you see "[System]: ", it is an automated notification from AgentHub. Acknowledge it but do not repeat it.
- When the user addresses you directly with @{tag}, respond to them specifically.
- Keep your responses focused and concise. Do not include unnecessary preambles like "Sure!" or "Of course!".
- Do not repeat what was just said to you. Jump directly to your response.
- Do not output ANSI color codes or markdown decorations that render as raw escape sequences.

=== YOUR ROLE BEHAVIOR ===
{role_specific_instructions}

=== CRITICAL RULES ===
1. You are in an automated environment. Do not ask for human confirmation for routine operations unless explicitly instructed.
2. If you receive a task, complete it and signal completion by ending your response. Do not ask "Is there anything else?"
3. If you are a Builder, write code. If you are a Reviewer, critique code. Stay in your role unless instructed otherwise.
4. You may see your own previous responses quoted back to you. Do not be confused by this — it is context injection for continuity.

Please acknowledge these instructions with only the word: "READY"
```

**Induction flow:**

```
1. Inject the rendered induction prompt into the agent's PTY stdin.
2. Start a 30-second timeout (tokio::time::timeout).
3. Monitor the agent's sanitized output via the ring buffer.
4. When the output contains exactly "READY" (case-insensitive, trimmed):
   a. Mark agent status as Idle.
   b. Emit BusEvent::AgentOnline.
   c. Display in chat: "[System]: @{tag} ({role}) has joined the session."
5. If the timeout expires without "READY":
   a. Emit AgentHubError::InductionTimeout.
   b. Display: "[System]: @{tag} failed to acknowledge induction. The agent
      may still function but is not context-aware. Kick and respawn if issues arise."
   c. Mark agent Idle anyway (graceful degradation).
```

---

## PART 9: WORKSPACE MODES (`crates/core/src/server/modes.rs`)

### 9.1 Direct Message (DM) Mode

```
Activation: /mode dm or startup with default_mode = "direct_message"

Rules:
- Maximum 1 active agent at a time.
- If spawning a 2nd agent while in DM mode, prompt:
  "DM mode supports only 1 agent. Kick @{existing} and spawn @{new}? [Y/n]"
- No broadcast: messages go only to the single active agent.
- No RBAC enforcement: the single agent has full permissions implicitly.
- No sidebar channel list. UI is a simple 2-pane split: chat + input.
- Induction prompt is simplified: omit multi-agent context sections.
- VFS snapshots are still taken before each message.
- The pipeline system is available (for single-agent self-loops and Unix piping).

Use case: Quick, focused Q&A or coding session with one model.
```

### 9.2 Group Chat Mode

```
Activation: /mode groupchat (default)

Rules:
- Up to max_agents agents (config default: 16).
- All agents receive broadcasts from all other agents.
- No structured channels. One single chat room.
- RBAC is loaded but permissive: all agents have SEND_MESSAGES and
  RECEIVE_BROADCAST by default unless explicitly overridden.
- Chaos heuristics active (staggered broadcast injection).
- No channel restrictions — any agent can address any other.
- Moderation commands (/mute, /deafen, /kick, /timeout) are available.
- Role assignments are informational (affect induction and labeling)
  but do not restrict permissions.

Use case: Free-form multi-agent brainstorming, code generation with peer review.
```

### 9.3 Server Mode

```
Activation: /mode server

Rules:
- Full RBAC enforcement. An agent without SEND_MESSAGES cannot inject
  output into the bus.
- An agent without RECEIVE_BROADCAST receives only direct @-mentions.
- An agent without EXECUTE_UNIX cannot trigger > pipeline steps.
- An agent without WRITE_FILES has its PTY monitored: if the sanitizer
  detects filesystem write commands (e.g., "cat > file", "echo > file",
  tool calls that write), a warning is emitted and the output is blocked
  before being injected into the workspace.
  NOTE: Blocking filesystem writes from a PTY is best-effort. AgentHub
  cannot intercept all write mechanisms. VFS snapshots remain the
  primary safety net.
- Roles are strictly enforced. Induction prompts are role-specific.
- The sidebar shows agent list with roles and status indicators.
- Channel concept: The user can create named logical channels in the UI.
  Agents can be assigned to channels. Messages tagged with #channel-name
  are only broadcast to agents in that channel.
  Channel state is stored in ServerState.channels (DashMap<String, Vec<Uuid>>).
- Admin commands (/ban, /addrole, /removerole) only available in Server mode.

Use case: Structured multi-agent software development with defined roles,
          safety constraints, and governance.
```

---

## PART 10: PIPELINE ENGINE (`crates/core/src/pipeline/`)

### 10.1 Pipeline Syntax (`pipeline/parser.rs`)

The pipeline parser handles the Frankenstein syntax typed directly in the chat input box.

**Full grammar (EBNF):**

```
pipeline     ::= stage ("|" stage)*
stage        ::= agent_stage | unix_stage
agent_stage  ::= "@" tag_name prompt_text
unix_stage   ::= ">" command_text
tag_name     ::= [a-zA-Z0-9_-]+
prompt_text  ::= (any character except "|")*
command_text ::= (any character except "|")*
```

**Examples:**

```
@gemini write a Rust HTTP server | > cargo check | @claude fix the errors
@gemini-1 design the schema | @gemini-2 review the schema | > echo "done"
@aider implement the login route | > cargo test | @claude summarize test results
```

**Parser implementation:**

```
1. Split the raw input string on " | " (space-pipe-space).
2. For each segment, trim whitespace.
3. If the segment starts with "@": create AgentStage { tag, prompt }.
4. If the segment starts with ">": create UnixStage { command }.
5. If the segment starts with neither: treat as an AgentStage targeting
   all currently active agents (broadcast pipeline start).
6. Return Vec<PipelineStage> or PipelineParseError with character position.
```

### 10.2 Pipeline Executor (`pipeline/executor.rs`)

```
Execution flow for a pipeline with N stages:

1. Before executing any stage, create a VFS snapshot (Part 12).
   Store snapshot_id for potential undo.

2. Initialize `current_output: String = ""`.

3. For each stage i in 0..N:

   If AgentStage:
     a. Look up the agent by tag in ServerState.agents.
        If not found: PipelineExecution error.
     b. Check agent has SEND_MESSAGES permission.
        If not: PipelineExecution error with permission details.
     c. Construct injection string:
        if i == 0:
          inject = stage.prompt
        else:
          inject = format!("[Pipeline context from previous stage]:\n{}\n\n{}",
                           current_output, stage.prompt)
     d. Write inject to the agent's PTY stdin.
     e. Wait for BusEvent::AgentMessage from this specific agent
        (filter by agent id) using a tokio channel receiver.
        Apply a timeout of 5 minutes. If exceeded: PipelineExecution error.
     f. Set current_output = event.content.
     g. Emit BusEvent::PipelineStageComplete { stage: i, output_preview: first 200 chars }.

   If UnixStage:
     a. Spawn std::process::Command for the command string via the shell:
        - Unix: ["sh", "-c", command]
        - Windows: ["cmd", "/C", command]
     b. Write current_output to the command's stdin.
     c. Wait for the command to exit (timeout: 60 seconds).
     d. If exit code != 0: set current_output = stderr content.
        Display in chat: "[Pipeline]: Unix command failed (exit {code}):\n{stderr}"
     e. If exit code == 0: set current_output = stdout content.
     f. Emit BusEvent::PipelineStageComplete.

4. On completion of all stages: Emit BusEvent::PipelineComplete.
5. Display final output in chat attributed to the last agent stage.
6. Log the entire pipeline execution (all stages, inputs, outputs) to SQLite.
```

### 10.3 Autonomous Agent Loop / Sparring (`pipeline/loop_engine.rs`)

The Sparring feature enables two or more agents to autonomously interact with each other for a defined number of turns.

**UI Command:**

```
/spar @{agent_a} as {role_a} vs @{agent_b} as {role_b} [--turns {n}] [--goal "{goal}"]

Example:
/spar @gemini as Coder vs @claude as Reviewer --turns 5 --goal "Write a Rust TCP server"
```

**Execution flow:**

```
Parameters:
  - agent_a_id, agent_a_role_label (e.g., "Coder")
  - agent_b_id, agent_b_role_label (e.g., "Reviewer")
  - max_turns: u8 (default: 5, max: 20)
  - goal: String

Pre-flight:
  1. Create a VFS snapshot.
  2. Inject role context into both agents via /setprompt:
     To agent_a: "For this sparring session, you are the {agent_a_role_label}.
                  Your goal: {goal}. Start by addressing the goal directly."
     To agent_b: "For this sparring session, you are the {agent_b_role_label}.
                  You will review and critique what the {agent_a_role_label} produces.
                  Goal context: {goal}."

Loop:
  current_turn = 0
  last_output = goal  // Seed the first turn with the goal

  while current_turn < max_turns:
    // Agent A's turn
    inject to agent_a: format!("{agent_b_role_label} feedback:\n{last_output}\n\n
                                Please respond as {agent_a_role_label}.")
    Wait for agent_a's AgentMessage (5-min timeout).
    Display output in chat with "[Spar Turn {turn} — {agent_a_tag}]" prefix.
    Check for loop termination conditions (see below).
    last_output = agent_a_output.

    // Agent B's turn
    inject to agent_b: format!("{agent_a_role_label} produced:\n{last_output}\n\n
                                Please respond as {agent_b_role_label}.")
    Wait for agent_b's AgentMessage (5-min timeout).
    Display output in chat with "[Spar Turn {turn} — {agent_b_tag}]" prefix.
    Check for loop termination conditions.
    last_output = agent_b_output.

    current_turn += 1

  Display: "[Spar]: Session complete after {current_turn} turns."

Loop Termination Conditions (checked after each turn):
  1. current_turn >= max_turns.
  2. User presses Escape key: sets a global AtomicBool::ABORT flag.
     The loop checks this flag at the start of each iteration.
     On abort: display "[Spar]: Manually aborted by user."
  3. Stagnation detection: If the last two outputs from the same agent have
     a Levenshtein edit distance ratio > 0.95 (near-identical), the loop is
     a "Thank You Loop". Abort and display:
     "[Spar Warning]: Stagnation detected. Agents are repeating themselves.
      Use /spar again with a more specific goal or fewer turns."
  4. Rate limit on either agent: abort and display rate limit warning.
```

---

## PART 11: LLM RACING (`crates/tui/src/components/racing.rs` + `crates/core/src/bus/router.rs`) ✅ SEALED

LLM Racing allows the user to send one prompt to multiple agents simultaneously and compare their outputs side-by-side.

**Activation syntax:**

```
@gemini @claude @codex write a binary search function in Rust
```

When the router detects multiple @-tags before a prompt (no pipeline `|` separator), it activates Racing mode.

**Execution flow:**

```
1. Parse all tags from the input. Validate each tag against active agents.
   Collect agent ids: Vec<Uuid>.
2. Create a RacingSession:
   - session_id: Uuid
   - contestants: Vec<Uuid>
   - prompt: String
   - outputs: DashMap<Uuid, String>  // Updated as outputs stream in
   - start_time: Instant

3. Take a VFS snapshot.

4. Simultaneously (tokio::join! or FuturesUnordered):
   For each agent_id in contestants:
     - Write the prompt string to the agent's PTY stdin.
     - The normal sanitizer task handles output. When it emits AgentMessage,
       it is tagged with the session_id so the Racing UI knows where to route it.

5. The TUI's racing.rs component:
   - Splits the main chat pane into N vertical columns (N = number of contestants).
   - Each column is headed with the agent's tag and a "⏳ Thinking..." indicator.
   - As AgentMessage events arrive for this session_id, they stream into the
     corresponding column in real-time.
   - When an agent completes, its column shows "✅ Done" and a timer (seconds elapsed).
   - The last agent to complete triggers "All done" state.

6. Selection:
   - The user uses arrow keys (← →) to highlight a column.
   - Pressing Enter selects that agent's output.
   - The selected output is inserted into the main chat history as the canonical response.
   - The other outputs are archived (still visible by scrolling up) but not
     inserted into the primary thread.
   - Pressing Escape dismisses Racing mode and inserts nothing.

7. The racing results (all outputs, agent tags, timings) are logged to SQLite.
```

---

## PART 12: TIME-TRAVEL VFS (`crates/core/src/vfs/`)

### 12.1 Snapshot Creation (`vfs/snapshot.rs`)

A snapshot is taken automatically before:
- Any pipeline executes.
- Any Sparring session starts.
- Any LLM Racing session starts.
- Any individual agent message that modifies files (best-effort detection).

The user can also trigger a manual snapshot with `/snapshot`.

**Implementation:**

```
1. Generate snapshot_id = Uuid::new_v4().
2. Determine snapshot_dir = config.shadow_dir.join(snapshot_id.to_string()).
3. Create snapshot_dir.

4. OS-Level CoW (preferred, fast):
   macOS (APFS): Call fcopyfile() or clonefile() syscall for each file.
                 This is O(1) — the OS defers actual copying until modification.
   Linux (Btrfs/ZFS): Use ioctl BTRFS_IOC_CLONE or ZFS clone via libzfs.
   Fallback: Skip to step 5.

5. Fallback (ext4, NTFS, HFS+):
   Use jwalk::WalkDir to recursively iterate the CWD.
   Exclude: .agenthub_shadow/, .git/, node_modules/, target/, dist/, __pycache__/
   For each file:
     a. Compute blake3 hash of the file contents.
     b. Check the snapshot manifest (SQLite) for a prior snapshot of this path.
        If the hash matches the previous snapshot, record the path with a
        "unchanged" marker (do not copy the file — deduplication).
        If the hash differs or no prior snapshot exists, copy the file to
        snapshot_dir preserving relative path structure.
   Use rayon::iter for parallelism across files.

6. Write manifest to SQLite:
   INSERT INTO snapshots (id, timestamp, file_count, cwd, size_bytes)
   For each file: INSERT INTO snapshot_files (snapshot_id, rel_path, hash, status)

7. Emit BusEvent::SnapshotCreated.
8. Maintain a maximum of 20 snapshots. If exceeded, delete the oldest.
```

### 12.2 Revert (`vfs/revert.rs`)

**Triggered by:** `Ctrl+Z` in the TUI, or `/undo` command.

```
1. Look up the most recent snapshot_id from SQLite:
   SELECT id FROM snapshots ORDER BY timestamp DESC LIMIT 1

2. Display confirmation: "[VFS]: Revert to snapshot {id} taken {elapsed} ago?
   This will overwrite {file_count} files. [Y/n]"

3. On confirmation:
   a. Freeze all active agent PTYs: send SIGSTOP to all PIDs.
   b. For each file in snapshot_files WHERE snapshot_id = {id}:
      - If status = "unchanged": skip (file has not changed since snapshot).
      - If status = "copied":
        i.  Attempt fs3::FileExt::try_lock_exclusive() on the target file.
            If locked, wait 100ms and retry up to 5 times. If still locked,
            skip this file and log a warning.
        ii. Write the snapshot copy to a temp path: "{path}.agenthub_tmp".
        iii. Call libc::rename() / std::fs::rename() to atomically replace.
   c. For files that exist in CWD but NOT in the snapshot manifest (new files
      created by agents): delete them after prompting:
      "[VFS]: {n} new files were created since the snapshot.
       Delete them? [Y/n]"
   d. Resume all agent PTYs: send SIGCONT to all PIDs.
   e. Pop the last N messages from the TUI chat history state, where N is the
      number of messages since the snapshot was taken.
   f. Emit BusEvent::RevertComplete.
   g. Display: "[VFS]: ✅ Workspace reverted. {file_count} files restored."

4. Delete the used snapshot from SQLite and from disk.
```

---

## PART 13: AUTO-CONTEXT ENGINE (`crates/core/src/context/`)

### 13.1 AST Indexer (`context/indexer.rs`)

The indexer runs in the background after AgentHub starts and whenever files change.

```
1. Use the `ignore` crate to walk the CWD respecting .gitignore.
2. For each source file, detect language by extension:
   .rs → tree-sitter-rust
   .py → tree-sitter-python
   .ts / .tsx → tree-sitter-typescript
   .js / .jsx → tree-sitter-javascript
   .go → tree-sitter-go
   Others: treat as plain text, index as-is.
3. Parse with tree-sitter. Extract:
   - Function/method names and their byte ranges.
   - Struct/class/type definitions and their byte ranges.
   - Module/import declarations.
4. Store in a HashMap<PathBuf, Vec<SymbolEntry>> in memory.
   SymbolEntry { name: String, kind: SymbolKind, start_byte: usize, end_byte: usize }
5. Also maintain a plain file list for non-code files (.md, .toml, .json, .sql).
6. Re-index a file within 2 seconds of modification (use the `notify` crate
   to watch for filesystem events).
```

### 13.2 Context Injector (`context/injector.rs`)

The injector intercepts user prompts before they are sent to an agent's PTY.

```
Trigger conditions (checked in order):

1. Explicit filename reference:
   Regex: r"\b[\w\-/\.]+\.(rs|py|ts|js|go|toml|json|sql|md)\b"
   If the prompt contains a filename that exists in the CWD:
     - Read the file contents.
     - Minify: strip blank lines, strip single-line comments (// and #),
               collapse consecutive whitespace.
     - If minified content > 8000 characters: truncate to 8000 chars and append
       "[...truncated for context. Full file available on request.]"
     - Prepend to the prompt:
       "[Auto-Context: {filename}]\n{minified_content}\n\n[User prompt]: {original_prompt}"

2. Symbol reference:
   If the prompt contains a word that exactly matches a symbol name in the index:
     - Extract the symbol's byte range from the original file.
     - Include only that function/struct/class definition.
     - Prepend: "[Auto-Context: {symbol_name} from {filename}]\n{symbol_code}\n\n
                 [User prompt]: {original_prompt}"

3. No match: send the prompt as-is.

The user can disable Auto-Context per-message by prefixing with `--nocontext`:
  "--nocontext @gemini explain what this does"
The injector strips the `--nocontext` flag before sending.

Log all context injections to the debug log at TRACE level.
```

---

## PART 14: DATABASE SCHEMA (`crates/core/src/db/`) ✅ SEALED

**Verification:** `cargo test -p agenthub-core --no-default-features --features db-tests db` · `cargo clippy -p agenthub-core --no-default-features --features db-tests -- -D warnings`

**DoD:**
- [x] `001_initial_schema.sql` DDL matches blueprint §14.1 (9 tables, 5 indices; WAL pragmas via `DbClient::apply_pragmas`).
- [x] Migrations apply on a fresh SQLite file; `PRAGMA journal_mode` is `wal`.
- [x] `DbClient::log_bus_event` persists user/agent/system messages, agent lifecycle, pipelines, and snapshots.
- [x] Column names verified via `pragma_table_info` against `db::schema::TABLE_COLUMNS`.

### 14.1 SQLite Schema (`db/migrations/001_initial_schema.sql`)

```sql
-- Enable WAL mode for concurrent reads during writes.
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;

-- ─────────────────────────────────────────────────────────────────────────────
-- Session Log: One row per AgentHub launch.
-- ─────────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS sessions (
    id          TEXT PRIMARY KEY NOT NULL,  -- UUID
    started_at  INTEGER NOT NULL,           -- Unix timestamp (seconds)
    ended_at    INTEGER,                    -- NULL if still running
    mode        TEXT NOT NULL,             -- 'dm', 'group_chat', 'server'
    cwd         TEXT NOT NULL              -- Absolute path of working directory
);

-- ─────────────────────────────────────────────────────────────────────────────
-- Agent Registry: One row per spawned agent instance (per session).
-- ─────────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS agents (
    id           TEXT PRIMARY KEY NOT NULL, -- UUID
    session_id   TEXT NOT NULL REFERENCES sessions(id),
    tag          TEXT NOT NULL,             -- '@gemini-1'
    driver_name  TEXT NOT NULL,             -- 'gemini'
    role         TEXT NOT NULL,             -- 'Builder'
    spawned_at   INTEGER NOT NULL,
    killed_at    INTEGER,
    kill_reason  TEXT                       -- 'kicked', 'crashed', 'natural'
);

-- ─────────────────────────────────────────────────────────────────────────────
-- Chat Log: Every message visible in any chat pane.
-- ─────────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS messages (
    id           TEXT PRIMARY KEY NOT NULL, -- UUID
    session_id   TEXT NOT NULL REFERENCES sessions(id),
    sender_type  TEXT NOT NULL,             -- 'user', 'agent', 'system'
    sender_id    TEXT,                      -- Agent UUID if sender_type='agent'
    sender_tag   TEXT NOT NULL,             -- '@gemini-1' or 'User' or 'System'
    content      TEXT NOT NULL,
    timestamp    INTEGER NOT NULL,          -- Unix timestamp (milliseconds)
    pipeline_id  TEXT,                      -- UUID if part of a pipeline
    race_id      TEXT                       -- UUID if part of an LLM race
);

-- ─────────────────────────────────────────────────────────────────────────────
-- Pipeline Log: One row per pipeline execution.
-- ─────────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS pipelines (
    id           TEXT PRIMARY KEY NOT NULL, -- UUID
    session_id   TEXT NOT NULL REFERENCES sessions(id),
    definition   TEXT NOT NULL,             -- Raw pipeline string typed by user
    status       TEXT NOT NULL,             -- 'running', 'complete', 'failed'
    started_at   INTEGER NOT NULL,
    completed_at INTEGER,
    snapshot_id  TEXT                       -- VFS snapshot taken before execution
);

CREATE TABLE IF NOT EXISTS pipeline_stages (
    id           TEXT PRIMARY KEY NOT NULL,
    pipeline_id  TEXT NOT NULL REFERENCES pipelines(id),
    stage_index  INTEGER NOT NULL,
    stage_type   TEXT NOT NULL,             -- 'agent', 'unix'
    target       TEXT NOT NULL,             -- agent tag or unix command
    input_text   TEXT,
    output_text  TEXT,
    started_at   INTEGER,
    completed_at INTEGER,
    exit_code    INTEGER                    -- NULL for agent stages
);

-- ─────────────────────────────────────────────────────────────────────────────
-- VFS Snapshots: Workspace checkpoints.
-- ─────────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS snapshots (
    id           TEXT PRIMARY KEY NOT NULL, -- UUID
    session_id   TEXT NOT NULL REFERENCES sessions(id),
    timestamp    INTEGER NOT NULL,
    file_count   INTEGER NOT NULL,
    size_bytes   INTEGER NOT NULL,
    cwd          TEXT NOT NULL,
    trigger      TEXT NOT NULL             -- 'pipeline', 'race', 'spar', 'manual'
);

CREATE TABLE IF NOT EXISTS snapshot_files (
    id           TEXT PRIMARY KEY NOT NULL,
    snapshot_id  TEXT NOT NULL REFERENCES snapshots(id),
    rel_path     TEXT NOT NULL,            -- Relative to CWD
    blake3_hash  TEXT NOT NULL,
    status       TEXT NOT NULL            -- 'copied', 'unchanged'
);

-- ─────────────────────────────────────────────────────────────────────────────
-- Role Registry: Custom roles persisted across sessions.
-- ─────────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS custom_roles (
    name                 TEXT PRIMARY KEY NOT NULL,
    permissions_mask     INTEGER NOT NULL,          -- Bitflags as u64
    induction_override   TEXT                        -- NULL = use default template
);

-- ─────────────────────────────────────────────────────────────────────────────
-- Debug Log: Raw PTY byte streams for driver profile debugging.
-- Stored as compressed blobs. Rotated after 48 hours.
-- ─────────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS pty_debug_log (
    id           TEXT PRIMARY KEY NOT NULL,
    agent_id     TEXT NOT NULL,
    timestamp    INTEGER NOT NULL,
    raw_bytes    BLOB NOT NULL             -- zstd-compressed raw PTY output
);

-- Indices for common query patterns
CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id, timestamp);
CREATE INDEX IF NOT EXISTS idx_messages_sender  ON messages(sender_id);
CREATE INDEX IF NOT EXISTS idx_pipeline_stages  ON pipeline_stages(pipeline_id, stage_index);
CREATE INDEX IF NOT EXISTS idx_snapshot_files   ON snapshot_files(snapshot_id);
CREATE INDEX IF NOT EXISTS idx_pty_debug_agent  ON pty_debug_log(agent_id, timestamp);
```

---

## PART 15: TERMINAL USER INTERFACE (`crates/tui/`)

### 15.1 Layout Specification

The TUI is built with `ratatui` and `crossterm`. The layout is composed of named regions.

**Default layout (Group Chat / Server mode):**

```
┌─────────────────────────────────────────────────────┬──────────────────────┐
│ CHAT HISTORY PANE                                   │ AGENT STATUS SIDEBAR │
│                                                     │                      │
│ [12:04:01] User: @gemini write an auth module       │ ● gemini-1  [Builder]│
│                                                     │   ⏳ Thinking...     │
│ [12:04:03] @gemini-1:                               │                      │
│   Here is the auth module:                          │ ● claude-1  [Reviewer│
│   ```rust                                           │   ✅ Idle            │
│   pub fn authenticate(token: &str) -> bool {        │                      │
│     ...                                             │ ○ aider-1   [Builder]│
│   }                                                 │   🔇 Muted           │
│   ```                                               │                      │
│                                                     │ MODE: GroupChat      │
│ [12:04:45] @claude-1:                               │ AGENTS: 3/16         │
│   I notice the authenticate function lacks...       │ SNAPSHOTS: 2         │
│                                                     │──────────────────────│
│                                                     │ PIPELINE             │
│                                                     │ Stage 2/3 ████░░ 67% │
│                                                     │ @gemini-1 → cargo    │
│                                                     │  → @claude-1         │
├─────────────────────────────────────────────────────┴──────────────────────┤
│ INPUT (/command or @agent message or pipeline syntax)                       │
│ > _                                                                         │
└─────────────────────────────────────────────────────────────────────────────┘
  F1:Help  F2:Mode  F3:Snapshot  Ctrl+Z:Undo  Ctrl+R:Race  Esc:Cancel
```

**DM Mode layout:**

```
┌─────────────────────────────────────────────────────────────────────────────┐
│ DM: @gemini-1 [Builder] ● Thinking...                                       │
├─────────────────────────────────────────────────────────────────────────────┤
│ [chat history]                                                               │
│                                                                              │
├─────────────────────────────────────────────────────────────────────────────┤
│ > _                                                                          │
└─────────────────────────────────────────────────────────────────────────────┘
```

**LLM Racing layout (activated when multiple @tags detected):**

```
┌────────────────────────┬───────────────────────┬────────────────────────────┐
│ @gemini-1 [Builder]    │ @claude-1 [Reviewer]  │ @codex-1 [Builder]         │
│ ⏳ 2.1s               │ ✅ 4.8s               │ ⏳ 2.1s                   │
├────────────────────────┼───────────────────────┼────────────────────────────┤
│ pub fn auth(token:     │ The authenticate      │ use std::collections::     │
│   &str) -> bool {      │ function is missing   │                            │
│   ...                  │ error handling...     │ fn auth(token: &str)       │
│                        │                       │  -> Result<bool, Err> {    │
│                        │                       │   ...                      │
├────────────────────────┴───────────────────────┴────────────────────────────┤
│ ← → to select winner. Enter to confirm. Esc to discard all.                 │
└─────────────────────────────────────────────────────────────────────────────┘
```

### 15.2 Key Bindings (Default)

```
Navigation:
  j / Down Arrow     : Scroll chat history down
  k / Up Arrow       : Scroll chat history up
  Page Down          : Scroll chat history one page down
  Page Up            : Scroll chat history one page up
  G                  : Jump to bottom of chat history (latest message)
  /                  : Enter search mode (searches chat history by substring)
  Esc                : Cancel current operation (Racing, Spar, Search, Pipeline)

Input Box:
  Enter              : Send message / execute command
  Ctrl+Enter         : Activate LLM Racing mode for multi-tag prompts
  Up Arrow           : Navigate input history (previous inputs)
  Down Arrow         : Navigate input history (next input)
  Tab                : Autocomplete agent @tags and slash commands
  Ctrl+L             : Clear input box

Session Control:
  F1                 : Toggle help overlay
  F2                 : Cycle workspace mode (DM → GroupChat → Server)
  F3                 : Create manual VFS snapshot
  Ctrl+Z             : Trigger VFS revert to most recent snapshot
  Ctrl+S             : Save current chat history to file (prompt for path)
  Ctrl+Q             : Quit AgentHub (with confirmation, all agents killed)

Agent Management:
  F5                 : Open spawn agent dialog
  F6                 : Open agent list (with inline kick/mute/role controls)
```

### 15.3 Agent Status Sidebar Indicators

```
● (green filled)    : Online, Idle — ready for input
⏳ (yellow)         : Thinking — generating response
🔇 (grey)           : Muted — output hidden from chat
🔕 (blue)           : Deafened — not receiving broadcasts
⏸ (orange)         : Timed out / Suspended
💀 (red)            : Dead / Kicked / Crashed
⚠ (yellow warning) : Rate limited — waiting for retry or user intervention
```

---

## PART 16: GITHUB CI/CD (`.github/workflows/`)

### 16.1 `ci.yml` — Continuous Integration

```yaml
name: CI

on:
  push:
    branches: [ "main" ]
  pull_request:
    branches: [ "main" ]

env:
  CARGO_TERM_COLOR: always
  RUST_BACKTRACE: 1

jobs:
  test:
    name: Test Suite
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
      - name: Check formatting
        run: cargo fmt --all -- --check
      - name: Clippy (zero warnings)
        run: cargo clippy --workspace --all-features -- -D warnings
      - name: Run tests
        run: cargo test --workspace --all-features
      - name: Check that binary builds
        run: cargo build --bin agenthub --release

  coverage:
    name: Code Coverage
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - name: Install cargo-tarpaulin
        run: cargo install cargo-tarpaulin
      - name: Generate coverage report
        run: cargo tarpaulin --workspace --out Xml --output-dir coverage/
      - name: Upload to Codecov
        uses: codecov/codecov-action@v4
        with:
          files: coverage/cobertura.xml
```

### 16.2 `release.yml` — Binary Distribution

```yaml
name: Release

on:
  push:
    tags:
      - 'v*'

jobs:
  build:
    name: Build Release Binaries
    strategy:
      matrix:
        include:
          - os: ubuntu-latest
            target: x86_64-unknown-linux-gnu
            artifact: agenthub-linux-x86_64
          - os: macos-latest
            target: x86_64-apple-darwin
            artifact: agenthub-macos-x86_64
          - os: macos-latest
            target: aarch64-apple-darwin
            artifact: agenthub-macos-arm64
          - os: windows-latest
            target: x86_64-pc-windows-msvc
            artifact: agenthub-windows-x86_64.exe
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}
      - uses: Swatinem/rust-cache@v2
      - name: Build
        run: cargo build --bin agenthub --release --target ${{ matrix.target }}
      - name: Rename artifact
        shell: bash
        run: |
          cp target/${{ matrix.target }}/release/agenthub* ${{ matrix.artifact }}
      - name: Upload to GitHub Release
        uses: softprops/action-gh-release@v2
        with:
          files: ${{ matrix.artifact }}
```

---

## PART 17: INTEGRATION TEST FIXTURES

The `tests/fixtures/mock_cli/` crate is a simple Rust binary that simulates a real interactive CLI for use in integration tests. This removes the dependency on having actual AI CLIs installed in CI.

**`tests/fixtures/mock_cli/src/main.rs`:**

```rust
//! Mock CLI: Simulates an interactive AI CLI for integration testing.
//! Reads from stdin, prints a canned response, then prints the prompt symbol.
//!
//! Behaviour flags (via environment variables):
//!   MOCK_CLI_PROMPT         : The prompt string to print. Default: "> "
//!   MOCK_CLI_RESPONSE       : The response to any input. Default: "Mock response."
//!   MOCK_CLI_LATENCY_MS     : Simulated thinking time in ms. Default: 100
//!   MOCK_CLI_RATE_LIMIT_ON  : If "1", print a rate limit error instead of responding.
//!   MOCK_CLI_INDUCTION_ACK  : If "1", respond to induction with "READY".

use std::io::{self, BufRead, Write};
use std::time::Duration;

fn main() {
    let prompt = std::env::var("MOCK_CLI_PROMPT").unwrap_or_else(|_| "> ".to_string());
    let response = std::env::var("MOCK_CLI_RESPONSE")
        .unwrap_or_else(|_| "Mock response.".to_string());
    let latency_ms: u64 = std::env::var("MOCK_CLI_LATENCY_MS")
        .unwrap_or_else(|_| "100".to_string())
        .parse()
        .unwrap_or(100);
    let rate_limit = std::env::var("MOCK_CLI_RATE_LIMIT_ON")
        .map(|v| v == "1")
        .unwrap_or(false);
    let ack_induction = std::env::var("MOCK_CLI_INDUCTION_ACK")
        .map(|v| v == "1")
        .unwrap_or(true);

    let stdin = io::stdin();
    let stdout = io::stdout();

    // Print initial prompt
    print!("{}", prompt);
    stdout.lock().flush().unwrap();

    for line in stdin.lock().lines() {
        let input = line.unwrap_or_default();
        std::thread::sleep(Duration::from_millis(latency_ms));

        // Detect induction prompt
        if ack_induction && input.contains("AGENTHUB") {
            println!("READY");
        } else if rate_limit {
            println!("Error: rate limit exceeded. Please try again later.");
        } else {
            println!("{}", response);
        }

        print!("{}", prompt);
        stdout.lock().flush().unwrap();
    }
}
```

---

## PART 18: IMPLEMENTATION PHASE PLAN

The following phases must be completed strictly in order. Each phase has a **Definition of Done** (DoD) that must be fully satisfied before the next phase begins.

---

### Phase 1 — Foundations
**Scope:** Repository setup, workspace config, global error types, config system, driver profile schema and loader, mock CLI fixture, SQLite schema and migrations.

**DoD:**
- Cargo workspace compiles with zero warnings.
- `cargo test --workspace` passes.
- `~/.agenthub/config.json` is created with defaults on first run.
- Bundled driver profiles (gemini, claude, codex, aider, cursor) are parsed without error.
- SQLite database is created and migrations run successfully.
- Mock CLI binary compiles and prints `> ` when launched.

---

### Phase 2 — PTY Engine
**Scope:** `crates/core/src/pty/` — manager.rs, io.rs. PTY spawning, ring buffer, reader task.

**DoD:**
- `spawn_agent("mock_cli", ...)` successfully spawns the mock CLI in a PTY.
- Bytes written to mock CLI stdin appear in the ring buffer.
- The ring buffer does not allocate on the heap during read/write.
- `AgentPty::drop()` sends SIGKILL and no zombie processes remain after test exits.
- Integration test `pty_lifecycle.rs`: spawn → write → read → kill cycle passes.

---

### Phase 3 — Stream Sanitizer
**Scope:** `crates/core/src/sanitizer/` — parser.rs (VirtualGrid), heuristic.rs.

**DoD:**
- Given a byte sequence containing ANSI color codes and a loading spinner (`\r/-\|`), `extract_text()` returns only the final visible text.
- Given a byte sequence ending in `> ` matching a driver's prompt_regex, `is_turn_complete()` returns true within 200ms.
- Given a byte sequence where stream continues after apparent prompt, `is_turn_complete()` does NOT fire prematurely.
- Integration test `stream_sanitizer.rs` passes for all 5 bundled driver prompt patterns.

---

### Phase 4 — Central Message Bus & Tag Router
**Scope:** `crates/core/src/bus/router.rs`, all BusEvent variants.

**DoD:**
- A `UserMessage` with `@gemini-1` in content routes only to the gemini-1 PTY.
- A `UserMessage` with no @-tag broadcasts to all non-deafened agents.
- An `AgentMessage` from gemini-1 is broadcast to all other non-deafened agents with the correct `[gemini-1 says]:` prefix.
- All events are persisted to SQLite.
- Bus channel capacity of 1024 does not block producers under normal load.
- `cargo test --workspace` passes.

---

### Phase 5 — Basic TUI
**Scope:** `crates/tui/` — app.rs, events.rs, chat.rs, input.rs, sidebar.rs, theme.rs.

**DoD:**
- TUI renders without panic on terminal sizes from 80x24 to 220x50.
- Chat history is scrollable with j/k and Page Up/Down.
- Input history is navigable with Up/Down arrows.
- Tab autocompletes @tags for active agents.
- Agent status sidebar shows correct status indicators.
- Ctrl+Q shows confirmation before exit.
- F1 shows keybinding help overlay.

---

### Phase 6 — Server Mechanics & RBAC
**Scope:** `crates/core/src/server/` — rbac.rs, moderation.rs, induction.rs, modes.rs.

**DoD:**
- All 7 built-in roles are loadable with correct permission bitmasks.
- `/promote @mock-1 to Leader` updates agent permissions atomically.
- `/mute @mock-1` stops mock-1's output appearing in the TUI chat.
- `/deafen @mock-1` stops broadcasts being injected into mock-1's PTY.
- `/timeout @mock-1 10s` suspends the process for 10 seconds then auto-resumes.
- `/kick @mock-1` kills the process and removes it from ServerState.
- Grand Induction: mock CLI receives induction prompt and responds "READY" before appearing Online.
- Integration test `rbac_moderation.rs` passes all above scenarios.

---

### Phase 7 — Pipeline Engine
**Scope:** `crates/core/src/pipeline/` — parser.rs, executor.rs, loop_engine.rs.

**DoD:**
- Pipeline string `@mock-1 hello | > echo world | @mock-2 repeat` parses to 3 stages without error.
- Pipeline executes: mock-1 responds, output piped to echo, echo output piped to mock-2.
- A Unix stage with exit code 1 stops the pipeline and shows the error.
- Sparring loop between two mock CLIs completes 3 turns and terminates.
- Escape key aborts a running Spar within 500ms.
- Stagnation detection fires after 2 identical responses.
- Integration test `pipeline_frankenstein.rs` passes.

---

### Phase 8 — LLM Racing ✅ SEALED
**Scope:** `crates/tui/src/components/racing.rs` + bus routing for Racing sessions.

**DoD:**
- [x] Input `@mock-1 @mock-2 hello` activates Racing mode.
- [x] Both mock CLIs receive the prompt simultaneously (within 50ms of each other).
- [x] TUI shows split-pane with both outputs streaming.
- [x] Arrow keys select a column. Enter inserts selected output into main chat.
- [x] Esc discards all outputs.
- [x] Race results logged to SQLite.

---

### Phase 9 — Time-Travel VFS
**Scope:** `crates/core/src/vfs/` — snapshot.rs, revert.rs.

**DoD:**
- `/snapshot` creates a snapshot of the CWD in `.agenthub_shadow/`.
- Modifying a file after snapshot, then pressing Ctrl+Z, restores the original file contents.
- Files created after snapshot are listed for deletion on revert.
- Revert is atomic: uses rename, never leaves partial state.
- Agent PTYs are frozen during revert and resumed after.
- Integration test `vfs_snapshot_revert.rs` passes.

---

### Phase 10 — Auto-Context Engine ✅ SEALED
**Scope:** `crates/core/src/context/` — indexer.rs, injector.rs.

**DoD:**
- [x] Indexer scans a test Rust project and populates symbol table within 2 seconds.
- [x] Prompt `@mock-1 fix auth.rs` injects the contents of `auth.rs` before the prompt.
- [x] Prompt `@mock-1 explain authenticate()` injects only the `authenticate` function definition.
- [x] `--nocontext` prefix bypasses injection.
- [x] Integration test `context_injection.rs` passes.

---

### Phase 11 — Subagent Capture
**Scope:** `crates/core/src/pty/subagent.rs`.

**DoD (Linux):**
- eBPF program loads without error on Linux 5.15+.
- When mock CLI spawns a child process, AgentHub detects it within 500ms.
- The child is registered as `@mock-1-sub-1` with the Subagent role.
- A system message appears in chat.

**DoD (macOS/Windows):**
- Polling fallback detects child within 500ms.
- Graceful degradation message shown if ptrace attach fails.

---

### Phase 12 — Polish, Robustness & Packaging
**Scope:** keybindings, cross-platform testing, binary distribution.

**DoD:**
- All keybindings in Part 15.2 are functional.
- TUI handles terminal resize gracefully (no panic, layout redraws correctly).
- Orphan process annihilation: killing AgentHub with SIGKILL leaves no child processes.
- `cargo clippy --workspace -- -D warnings` passes on Linux, macOS, Windows.
- GitHub Actions CI passes on all three platforms.
- GitHub Actions Release produces binaries for all 4 targets.
- `README.md` installation instructions work from a fresh machine with no prior setup.

---

## PART 19: DEFINITION OF "SHIPPABLE"

The product is ready to ship when ALL of the following are true:

| # | Criterion | Status |
|---|-----------|--------|
| 1 | All 12 phases have a passing DoD | **true** |
| 2 | `cargo test --workspace` passes on Linux, macOS, and Windows in CI | **true** (local + `.github/workflows/ci.yml`) |
| 3 | `cargo clippy --workspace -- -D warnings` passes on all three platforms | **true** (local + CI) |
| 4 | Install from a single downloaded binary (no Rust toolchain) | **true** (`README.md`, `docs/INSTALL.md`, `release.yml`) |
| 5 | 60s quickstart: `agenthub` → `/spawn gemini` → prompt → response | **true** (`bootstrap.rs`, `README.md`) |
| 6 | `/help` lists all slash commands with descriptions | **true** (`events.rs` `SLASH_HELP`) |
| 7 | No zombie agent processes after any exit path | **true** (`AgentPty::drop`, `bootstrap::shutdown`) |
| 8 | SQLite WAL + single-writer bus (no corruption) | **true** (`DbClient::apply_pragmas`, router task) |
| 9 | No plaintext prompts on disk by default (PTY debug opt-in + zstd) | **true** (`pty_debug_log: false` default) |
| 10 | AgentHub makes no outbound network calls | **true** (local PTY only; no HTTP clients in tree) |

### Original criteria (reference)

1. All 12 phases have a passing DoD.
2. `cargo test --workspace` passes on Linux, macOS, and Windows in CI.
3. `cargo clippy --workspace -- -D warnings` passes on all three platforms.
4. A user can install AgentHub from a single downloaded binary (no Rust toolchain required).
5. A user can run `agenthub` in a project folder, spawn a Gemini agent, type a prompt, and receive a response — in under 60 seconds from first launch.
6. `/help` in the TUI lists all available commands with descriptions.
7. No agent process is ever left running after AgentHub exits, regardless of exit method (Ctrl+Q, SIGTERM, SIGKILL, power loss scenario via SIGKILL to parent).
8. The SQLite database is never corrupted by concurrent writes (enforced by WAL mode and the single-writer bus architecture).
9. No API keys, tokens, or user prompts are written to disk in plaintext (raw PTY debug logs are opt-in and zstd-compressed).
10. The product runs entirely offline with no outbound network connections from AgentHub itself.

---

*End of Blueprint. Version 1.0.*
