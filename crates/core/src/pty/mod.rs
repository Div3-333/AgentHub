//! PTY engine (blueprint §5).
//!
//! ## CI / PTY integration tests
//!
//! Set `AGENTHUB_SKIP_PTY=1` on runners that cannot allocate pseudo-terminals.
//! Unit tests for [`PtyStatus`] and spawn limits still run without a real PTY.
//!
//! Ring-buffer I/O is in [`io`] (§5.3 — no heap allocation on the read/write hot path).

pub mod debug_log;
pub mod io;
pub mod manager;
#[cfg(feature = "full")]
pub mod spawn_cmd;
#[cfg(feature = "full")]
pub mod subagent;
#[cfg(feature = "full")]
pub mod trace;

pub use crate::db::{compress_pty_bytes, decompress_pty_bytes};
pub use debug_log::{rotate_all as rotate_pty_debug, PtyDebugSink};
pub use io::{pty_reader_task, PtyRingBuffer, RING_CAPACITY};
pub use manager::{
    escalate_kill, freeze_agent_pids, kill_agent, pty_tests_enabled, resume_agent_pids, AgentPty,
    PtyStatus, SpawnOptions,
};
#[cfg(feature = "full")]
pub use manager::{mock_agent_for_tests, mock_agent_with_capture, pty_skip_mode, spawn_agent};
#[cfg(feature = "full")]
pub use spawn_cmd::{format_resolved_command, resolve_spawn_command, ResolvedCommand};
#[cfg(feature = "full")]
pub use subagent::{
    ensure_subagent_watcher, format_subagent_tag, match_child_processes, on_subagent_exec,
    poll_new_children, subagent_announcement, subagent_backend, subagent_watcher_task,
    SubagentBackend, SubagentExecEvent, POLL_INTERVAL_MS, SUBAGENT_CAPTURE_PENDING, SUBAGENT_ROLE,
};
#[cfg(feature = "full")]
pub use trace::{emit_pty_io_trace, emit_spawn_trace, preview_pty_bytes};
