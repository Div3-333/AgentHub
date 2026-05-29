// Workspace integration entry — Phase 7 Frankenstein pipeline (see `crates/core/tests/pipeline_frankenstein.rs`).
//
// Run: `cargo test -p agenthub-core pipeline_frankenstein` or `cargo test -p agenthub-integration pipeline_frankenstein`.
// Set `AGENTHUB_SKIP_PTY=1` on hosts without pseudo-terminal support.

include!("../../crates/core/tests/pipeline_frankenstein.rs");
