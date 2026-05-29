//! Opt-in PTY raw-byte capture (blueprint Part 19 #9).
//!
//! When [`crate::config::AgentHubConfig::pty_debug_log`] is enabled, the PTY reader
//! forwards read chunks here. Data is zstd-compressed before persistence.

use std::path::Path;
use std::sync::Arc;

use chrono::Utc;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::config::agenthub_debug_dir;
use crate::db::{DbClient, NewPtyDebugEntry};
use crate::error::Result;

use crate::db::compress_pty_bytes;

const ROTATION_SECS: i64 = 48 * 3600;

/// Async sink wired into [`super::io::pty_reader_task`]; drops are a no-op (channel drains).
pub struct PtyDebugSink {
    tx: mpsc::UnboundedSender<Vec<u8>>,
}

impl PtyDebugSink {
    /// Spawns a background task that persists compressed chunks for `agent_id`.
    #[must_use]
    pub fn spawn(db: Option<Arc<DbClient>>, agent_id: Uuid) -> Arc<Self> {
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
        tokio::spawn(async move {
            while let Some(raw) = rx.recv().await {
                if raw.is_empty() {
                    continue;
                }
                let timestamp = Utc::now().timestamp();
                let entry_id = Uuid::new_v4();
                let compressed = match compress_pty_bytes(&raw) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!(%agent_id, %e, "pty debug compress failed");
                        continue;
                    }
                };
                if let Some(ref db) = db {
                    let entry = NewPtyDebugEntry {
                        id: entry_id,
                        agent_id,
                        timestamp,
                        raw_bytes: compressed,
                    };
                    if let Err(e) = db.insert_pty_debug(&entry).await {
                        tracing::warn!(%agent_id, %e, "pty debug db insert failed");
                    }
                } else if let Err(e) = write_debug_file(agent_id, entry_id, timestamp, &compressed)
                {
                    tracing::warn!(%agent_id, %e, "pty debug file write failed");
                }
            }
        });
        Arc::new(Self { tx })
    }

    /// Enqueue one raw PTY read chunk (non-blocking).
    pub fn record(&self, raw: &[u8]) {
        if !raw.is_empty() {
            let _ = self.tx.send(raw.to_vec());
        }
    }
}

fn write_debug_file(
    agent_id: Uuid,
    entry_id: Uuid,
    timestamp: i64,
    compressed: &[u8],
) -> Result<()> {
    let dir = agenthub_debug_dir().join(agent_id.to_string());
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{timestamp}_{entry_id}.zst"));
    std::fs::write(path, compressed)?;
    Ok(())
}

/// Delete debug files older than 48 hours under [`agenthub_debug_dir`].
pub fn rotate_pty_debug_files() -> Result<u64> {
    let root = agenthub_debug_dir();
    if !root.is_dir() {
        return Ok(0);
    }
    let cutoff = Utc::now().timestamp() - ROTATION_SECS;
    let mut deleted = 0u64;
    for agent_entry in std::fs::read_dir(&root)? {
        let agent_entry = agent_entry?;
        if !agent_entry.file_type()?.is_dir() {
            continue;
        }
        for file_entry in std::fs::read_dir(agent_entry.path())? {
            let file_entry = file_entry?;
            let path = file_entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(ts) = parse_debug_file_timestamp(&path) else {
                continue;
            };
            if ts < cutoff {
                std::fs::remove_file(&path)?;
                deleted += 1;
            }
        }
    }
    Ok(deleted)
}

fn parse_debug_file_timestamp(path: &Path) -> Option<i64> {
    let name = path.file_name()?.to_str()?;
    name.split('_').next()?.parse().ok()
}

/// Run DB and filesystem rotation (call on startup when debug logging is enabled).
pub async fn rotate_all(db: &DbClient) -> Result<(u64, u64)> {
    let db_deleted = db.rotate_pty_debug_log().await?;
    let files_deleted = rotate_pty_debug_files()?;
    Ok((db_deleted, files_deleted))
}

#[cfg(all(test, feature = "full"))]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    use crate::db::DbClient;
    use tempfile::TempDir;

    #[test]
    fn parse_debug_file_timestamp_from_name() {
        let path = PathBuf::from("1706000000_550e8400-e29b-41d4-a716-446655440000.zst");
        assert_eq!(parse_debug_file_timestamp(&path), Some(1_706_000_000));
    }

    #[test]
    fn rotate_pty_debug_files_deletes_stale_entries() {
        let dir = TempDir::new().expect("tempdir");
        std::env::set_var("AGENTHUB_CONFIG_DIR", dir.path());
        let agent_id = Uuid::new_v4();
        let debug_root = dir.path().join("debug").join(agent_id.to_string());
        std::fs::create_dir_all(&debug_root).expect("mkdir");

        let old_ts = Utc::now().timestamp() - 49 * 3600;
        let new_ts = Utc::now().timestamp();
        let old_path = debug_root.join(format!("{old_ts}_{}.zst", Uuid::new_v4()));
        let new_path = debug_root.join(format!("{new_ts}_{}.zst", Uuid::new_v4()));
        std::fs::write(&old_path, b"stale").expect("write old");
        std::fs::write(&new_path, b"fresh").expect("write new");

        let deleted = rotate_pty_debug_files().expect("rotate");
        assert_eq!(deleted, 1);
        assert!(!old_path.exists());
        assert!(new_path.exists());

        std::env::remove_var("AGENTHUB_CONFIG_DIR");
    }

    #[tokio::test]
    async fn sink_persists_compressed_row() {
        let dir = TempDir::new().expect("tempdir");
        let url = format!("sqlite://{}", dir.path().join("test.db").display());
        let db = Arc::new(DbClient::init_pool(&url).await.expect("pool"));
        db.run_migrations().await.expect("migrate");
        let agent_id = Uuid::new_v4();
        let sink = PtyDebugSink::spawn(Some(Arc::clone(&db)), agent_id);
        sink.record(b"\x1b[0mPTY chunk");
        tokio::time::sleep(Duration::from_millis(200)).await;
        let rows = db.list_pty_debug(agent_id, None).await.expect("list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].raw_bytes, b"\x1b[0mPTY chunk");
    }
}
