//! Database layer (blueprint Part 14 — sealed).
//!
//! SQLite schema in [`migrations/001_initial_schema.sql`]; persistence via [`DbClient`].

pub mod pty_debug_codec;
pub mod schema;
pub mod sqlite;

pub use pty_debug_codec::{compress_pty_bytes, decompress_pty_bytes};

pub use sqlite::{
    AgentRow, CustomRoleRow, DbClient, MessageRow, NewAgent, NewCustomRole, NewMessage,
    NewPipeline, NewPipelineStage, NewPtyDebugEntry, NewSession, NewSnapshot, NewSnapshotFile,
    PipelineRow, PipelineStageRow, PtyDebugRow, SessionRow, SnapshotFileRow, SnapshotRow,
};
