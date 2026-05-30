//! Time-travel VFS: workspace snapshots and revert (blueprint Part 12).

pub mod revert;
pub mod snapshot;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::broadcast;
use uuid::Uuid;

use crate::bus::BusEvent;
use crate::config::AgentHubConfig;
use crate::db::DbClient;
use crate::error::{AgentHubError, Result};
use crate::server::ServerState;

pub use revert::{
    delete_new_files_message, freeze_agent_pids, is_undo_command, preview_revert,
    resume_agent_pids, revert_confirmation_message, revert_success_message, RevertOptions,
    RevertPreview, RevertResult,
};
pub use snapshot::{
    create_snapshot, create_snapshot_with_config, is_snapshot_command, resolve_shadow_dir,
    SnapshotInfo, SnapshotTrigger, MAX_SNAPSHOTS,
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

/// Handles `/snapshot` and `/undo` slash commands.
pub async fn handle_slash_command(
    input: &str,
    db: &DbClient,
    config: &AgentHubConfig,
    cwd: &Path,
    session_id: Uuid,
    bus_tx: Option<&broadcast::Sender<BusEvent>>,
    state: Option<&ServerState>,
) -> Result<Option<String>> {
    if snapshot::is_snapshot_command(input) {
        let info = snapshot::create_snapshot_with_config(
            db,
            config,
            cwd,
            session_id,
            SnapshotTrigger::Manual,
            bus_tx,
        )
        .await?;
        return Ok(Some(format!(
            "[VFS]: Snapshot {} created ({} files, {} bytes).",
            info.id, info.file_count, info.size_bytes
        )));
    }

    if revert::is_undo_command(input) {
        if is_undo_yes_command(input) {
            return execute_revert(db, config, cwd, true, bus_tx, state)
                .await
                .map(Some);
        }
        return Ok(None);
    }

    Ok(None)
}

/// Runs revert with explicit options (TUI confirmation or `/undo --yes`).
pub async fn execute_revert(
    db: &DbClient,
    config: &AgentHubConfig,
    cwd: &Path,
    delete_new_files: bool,
    bus_tx: Option<&broadcast::Sender<BusEvent>>,
    state: Option<&ServerState>,
) -> Result<String> {
    let shadow = snapshot::resolve_shadow_dir(config, cwd);
    let pids = state.map(collect_agent_pids).unwrap_or_default();
    let result = revert::revert_latest(
        &db.pool,
        cwd,
        &shadow,
        RevertOptions {
            delete_new_files,
            dry_run: false,
        },
        &pids,
        bus_tx,
    )
    .await?;
    Ok(revert_success_message(result.files_restored))
}

fn is_undo_yes_command(input: &str) -> bool {
    let parts: Vec<&str> = input.split_whitespace().collect();
    parts
        .first()
        .is_some_and(|cmd| cmd.eq_ignore_ascii_case("/undo"))
        && parts.iter().any(|p| *p == "--yes" || *p == "-y")
}

#[cfg(any(feature = "full", feature = "bus-tests"))]
fn collect_agent_pids(state: &ServerState) -> Vec<u32> {
    state
        .agents
        .iter()
        .map(|entry| entry.value().pid)
        .filter(|&pid| pid != 0)
        .collect()
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
