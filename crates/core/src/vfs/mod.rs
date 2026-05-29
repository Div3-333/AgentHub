//! Time-travel VFS: workspace snapshots and revert (blueprint Part 12).

pub mod revert;
pub mod snapshot;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::broadcast;
use uuid::Uuid;

use crate::bus::BusEvent;
use crate::db::DbClient;
use crate::error::{AgentHubError, Result};

pub use revert::{freeze_agent_pids, resume_agent_pids, RevertOptions, RevertResult};
pub use snapshot::{
    create_snapshot, create_snapshot_with_config, handle_slash_command, is_snapshot_command,
    resolve_shadow_dir, SnapshotInfo, SnapshotTrigger, MAX_SNAPSHOTS,
};

/// Engine coordinating snapshot creation and revert for one workspace.
#[derive(Clone)]
pub struct VfsEngine {
    cwd: PathBuf,
    shadow_dir: PathBuf,
    db: Arc<DbClient>,
    session_id: Uuid,
    bus_tx: Option<broadcast::Sender<BusEvent>>,
}

impl VfsEngine {
    #[must_use]
    pub fn new(
        cwd: impl Into<PathBuf>,
        shadow_dir: impl Into<PathBuf>,
        db: Arc<DbClient>,
        session_id: Uuid,
        bus_tx: Option<broadcast::Sender<BusEvent>>,
    ) -> Self {
        Self {
            cwd: cwd.into(),
            shadow_dir: shadow_dir.into(),
            db,
            session_id,
            bus_tx,
        }
    }

    #[must_use]
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    #[must_use]
    pub fn shadow_dir(&self) -> &Path {
        &self.shadow_dir
    }

    #[must_use]
    pub fn session_id(&self) -> Uuid {
        self.session_id
    }

    /// Creates a workspace snapshot under [`Self::shadow_dir`].
    pub async fn snapshot(&self, trigger: SnapshotTrigger) -> Result<SnapshotInfo> {
        create_snapshot(
            &self.db.pool,
            &self.cwd,
            &self.shadow_dir,
            self.session_id,
            trigger,
            self.bus_tx.as_ref(),
        )
        .await
    }

    /// Reverts the workspace to the most recent snapshot.
    pub async fn revert_latest(
        &self,
        opts: RevertOptions,
        agent_pids: &[u32],
    ) -> Result<RevertResult> {
        revert::revert_latest(
            &self.db.pool,
            &self.cwd,
            &self.shadow_dir,
            opts,
            agent_pids,
            self.bus_tx.as_ref(),
        )
        .await
    }
}

/// Ensures a `sessions` row exists (required by the snapshots FK).
pub async fn ensure_session(pool: &sqlx::SqlitePool, session_id: Uuid, cwd: &Path) -> Result<()> {
    let id = session_id.to_string();
    let cwd_str = cwd.to_string_lossy();
    let now = chrono::Utc::now().timestamp();

    sqlx::query(
        r"
        INSERT INTO sessions (id, started_at, mode, cwd)
        VALUES (?, ?, 'integration_test', ?)
        ON CONFLICT(id) DO NOTHING
        ",
    )
    .bind(&id)
    .bind(now)
    .bind(cwd_str.as_ref())
    .execute(pool)
    .await?;

    Ok(())
}

/// Returns true when `rel` is under an excluded top-level directory.
#[must_use]
pub fn is_excluded_rel(rel: &str) -> bool {
    let first = rel.split(['/', '\\']).next().unwrap_or(rel);
    matches!(
        first,
        ".agenthub_shadow" | ".git" | "node_modules" | "target" | "dist" | "__pycache__"
    )
}

/// Relative path from `cwd` using `/` separators for manifest storage.
pub fn rel_path_string(cwd: &Path, path: &Path) -> Result<String> {
    let rel = path.strip_prefix(cwd).map_err(|_| {
        AgentHubError::Snapshot(format!(
            "path {} is not under cwd {}",
            path.display(),
            cwd.display()
        ))
    })?;
    Ok(rel.to_string_lossy().replace('\\', "/"))
}

/// On-disk location of a copied file inside a snapshot directory.
#[must_use]
pub fn snapshot_file_path(shadow_dir: &Path, snapshot_id: Uuid, rel_path: &str) -> PathBuf {
    shadow_dir
        .join(snapshot_id.to_string())
        .join(rel_path.replace('/', std::path::MAIN_SEPARATOR_STR))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_excluded_rel_matches_blueprint_dirs() {
        assert!(is_excluded_rel(".git/config"));
        assert!(is_excluded_rel(".agenthub_shadow/foo"));
        assert!(is_excluded_rel("target/debug/foo"));
        assert!(!is_excluded_rel("src/main.rs"));
    }

    #[test]
    fn rel_path_string_normalizes_separators() {
        let cwd = PathBuf::from("/workspace");
        let file = PathBuf::from("/workspace/src/lib.rs");
        let rel = rel_path_string(&cwd, &file).expect("rel");
        assert_eq!(rel, "src/lib.rs");
    }
}
