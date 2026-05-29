// Workspace integration entry — Phase 2 PTY lifecycle (see `crates/core/tests/pty_lifecycle.rs`).
//
// Run: `cargo test -p agenthub-core pty_lifecycle` or `cargo test -p agenthub-integration pty_lifecycle`.
// Set `AGENTHUB_SKIP_PTY=1` on hosts without pseudo-terminal support.

include!("../../crates/core/tests/pty_lifecycle.rs");
