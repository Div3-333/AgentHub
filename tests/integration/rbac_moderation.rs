// Workspace integration entry — RBAC + moderation (see `crates/core/tests/rbac_moderation.rs`).
//
// Run: `cargo test -p agenthub-integration rbac_moderation --features full`
// Set `AGENTHUB_SKIP_PTY=1` on hosts without pseudo-terminal support.

include!("../../crates/core/tests/rbac_moderation.rs");
