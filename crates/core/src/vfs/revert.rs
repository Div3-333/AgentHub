//! Undo / time-travel revert logic (blueprint §12.2).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use fs3::FileExt;
use jwalk::WalkDir;
use sqlx::SqlitePool;
use tokio::sync::broadcast;
use tracing::warn;
use uuid::Uuid;

use crate::bus::BusEvent;
use crate::error::{AgentHubError, Result};
use crate::vfs::snapshot::{delete_snapshot, hash_file};
use crate::vfs::{is_excluded_rel, rel_path_string, snapshot_file_path};

const LOCK_RETRIES: u32 = 5;
const LOCK_WAIT_MS: u64 = 100;

/// Options for workspace revert.
#[derive(Debug, Clone, Default)]
pub struct RevertOptions {
    /// When true, delete files created after the snapshot that are not in the manifest.
    pub delete_new_files: bool,
    /// When true, report actions without modifying the workspace.
    pub dry_run: bool,
}

/// Result of a successful revert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevertResult {
    pub snapshot_id: Uuid,
    pub files_restored: usize,
    pub new_files_deleted: usize,
}

struct SnapshotRow {
    id: Uuid,
}

#[derive(sqlx::FromRow)]
struct ManifestRow {
    rel_path: String,
    blake3_hash: String,
    status: String,
}

/// True when `input` is the manual `/undo` slash command.
#[must_use]
pub fn is_undo_command(input: &str) -> bool {
    matches!(input.trim(), "/undo" | "/UNDO") || input.trim().eq_ignore_ascii_case("/undo")
}

/// User-facing confirmation before revert (blueprint §12.2 step 2).
#[must_use]
pub fn revert_confirmation_message(snapshot_id: Uuid, file_count: usize, elapsed: &str) -> String {
    format!(
        "[VFS]: Revert to snapshot {snapshot_id} taken {elapsed} ago? \
         This will overwrite {file_count} files. [Y/n]"
    )
}

/// User-facing prompt when new files exist since the snapshot.
#[must_use]
pub fn delete_new_files_message(count: usize) -> String {
    format!("[VFS]: {count} new files were created since the snapshot. Delete them? [Y/n]")
}

/// Success line after revert (blueprint §12.2 step 3g).
#[must_use]
pub fn revert_success_message(files_restored: usize) -> String {
    format!("[VFS]: ✅ Workspace reverted. {files_restored} files restored.")
}

/// Count workspace files not present in the latest snapshot manifest.
pub async fn count_new_files_for_latest(pool: &SqlitePool, cwd: &Path) -> Result<usize> {
    let snapshot = load_latest_snapshot(pool, cwd).await?;
    let manifest = load_manifest(pool, snapshot.id).await?;
    let manifest_paths: HashSet<String> = manifest.iter().map(|r| r.rel_path.clone()).collect();
    Ok(list_new_files(cwd, &manifest_paths)?.len())
}

/// Suspend agent processes during revert (delegates to [`crate::pty::freeze_agent_pids`]).
pub fn freeze_agent_pids(pids: &[u32]) {
    crate::pty::freeze_agent_pids(pids);
}

/// Resume agent processes after revert (delegates to [`crate::pty::resume_agent_pids`]).
pub fn resume_agent_pids(pids: &[u32]) {
    crate::pty::resume_agent_pids(pids);
}

/// Reverts the workspace to the latest snapshot for `cwd`.
pub async fn revert_latest(
    pool: &SqlitePool,
    cwd: &Path,
    shadow_dir: &Path,
    opts: RevertOptions,
    agent_pids: &[u32],
    bus_tx: Option<&broadcast::Sender<BusEvent>>,
) -> Result<RevertResult> {
    let snapshot = load_latest_snapshot(pool, cwd).await?;
    let manifest = load_manifest(pool, snapshot.id).await?;

    if let Some(tx) = bus_tx {
        let _ = tx.send(BusEvent::RevertInitiated {
            snapshot_id: snapshot.id,
        });
    }

    freeze_agent_pids(agent_pids);
    struct ResumeOnDrop<'a> {
        pids: &'a [u32],
    }
    impl Drop for ResumeOnDrop<'_> {
        fn drop(&mut self) {
            resume_agent_pids(self.pids);
        }
    }
    let _resume_guard = ResumeOnDrop { pids: agent_pids };

    let revert_outcome = {
        let manifest_paths: HashSet<String> = manifest.iter().map(|r| r.rel_path.clone()).collect();
        let mut files_restored = 0usize;

        for row in &manifest {
            if restore_manifest_entry(pool, cwd, shadow_dir, snapshot.id, row, opts.dry_run).await?
            {
                files_restored += 1;
            }
        }

        let new_files = list_new_files(cwd, &manifest_paths)?;
        let mut new_files_deleted = 0usize;
        if opts.delete_new_files {
            for path in new_files {
                if opts.dry_run {
                    new_files_deleted += 1;
                    continue;
                }
                if path.is_file() {
                    std::fs::remove_file(&path).map_err(|e| {
                        AgentHubError::Revert(format!(
                            "failed to delete new file {}: {e}",
                            path.display()
                        ))
                    })?;
                } else if path.is_dir() {
                    std::fs::remove_dir_all(&path).map_err(|e| {
                        AgentHubError::Revert(format!(
                            "failed to delete new path {}: {e}",
                            path.display()
                        ))
                    })?;
                }
                new_files_deleted += 1;
            }
        }

        Ok::<_, AgentHubError>(RevertResult {
            snapshot_id: snapshot.id,
            files_restored,
            new_files_deleted,
        })
    };

    let result = revert_outcome?;

    if !opts.dry_run {
        delete_snapshot(pool, shadow_dir, &snapshot.id.to_string()).await?;
    }

    if let Some(tx) = bus_tx {
        let _ = tx.send(BusEvent::RevertComplete {
            snapshot_id: snapshot.id,
        });
    }

    Ok(result)
}

async fn load_latest_snapshot(pool: &SqlitePool, cwd: &Path) -> Result<SnapshotRow> {
    let cwd_str = cwd.to_string_lossy();
    let row: Option<(String,)> = sqlx::query_as(
        r"
        SELECT id FROM snapshots
        WHERE cwd = ?
        ORDER BY timestamp DESC
        LIMIT 1
        ",
    )
    .bind(cwd_str.as_ref())
    .fetch_optional(pool)
    .await?;

    let (id,) =
        row.ok_or_else(|| AgentHubError::Revert("no snapshot found for workspace".into()))?;

    let id = Uuid::parse_str(&id)
        .map_err(|e| AgentHubError::Revert(format!("invalid snapshot id in db: {e}")))?;

    Ok(SnapshotRow { id })
}

async fn load_manifest(pool: &SqlitePool, snapshot_id: Uuid) -> Result<Vec<ManifestRow>> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        r"
        SELECT rel_path, blake3_hash, status
        FROM snapshot_files
        WHERE snapshot_id = ?
        ",
    )
    .bind(snapshot_id.to_string())
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(rel_path, blake3_hash, status)| ManifestRow {
            rel_path,
            blake3_hash,
            status,
        })
        .collect())
}

async fn restore_manifest_entry(
    pool: &SqlitePool,
    cwd: &Path,
    shadow_dir: &Path,
    snapshot_id: Uuid,
    row: &ManifestRow,
    dry_run: bool,
) -> Result<bool> {
    let dest = cwd.join(row.rel_path.replace('/', std::path::MAIN_SEPARATOR_STR));

    let needs_restore = match row.status.as_str() {
        "copied" => true,
        "unchanged" => {
            let current = if dest.is_file() {
                hash_file(&dest).ok()
            } else {
                None
            };
            current.as_deref() != Some(row.blake3_hash.as_str())
        }
        other => {
            warn!(status = other, path = %row.rel_path, "unknown snapshot_files status");
            true
        }
    };

    if !needs_restore {
        return Ok(false);
    }

    let src = if row.status == "copied" {
        snapshot_file_path(shadow_dir, snapshot_id, &row.rel_path)
    } else {
        let prior = find_copied_snapshot(pool, &row.rel_path, &row.blake3_hash, cwd).await?;
        snapshot_file_path(shadow_dir, prior, &row.rel_path)
    };

    if !src.is_file() {
        return Err(AgentHubError::Revert(format!(
            "snapshot copy missing for {} at {}",
            row.rel_path,
            src.display()
        )));
    }

    if dry_run {
        return Ok(true);
    }

    restore_file_atomic(&src, &dest)?;
    Ok(true)
}

async fn find_copied_snapshot(
    pool: &SqlitePool,
    rel_path: &str,
    hash: &str,
    cwd: &Path,
) -> Result<Uuid> {
    let cwd_str = cwd.to_string_lossy();
    let row: Option<(String,)> = sqlx::query_as(
        r"
        SELECT sf.snapshot_id
        FROM snapshot_files sf
        INNER JOIN snapshots s ON s.id = sf.snapshot_id
        WHERE sf.rel_path = ?
          AND sf.blake3_hash = ?
          AND sf.status = 'copied'
          AND s.cwd = ?
        ORDER BY s.timestamp DESC
        LIMIT 1
        ",
    )
    .bind(rel_path)
    .bind(hash)
    .bind(cwd_str.as_ref())
    .fetch_optional(pool)
    .await?;

    let (id,) = row.ok_or_else(|| {
        AgentHubError::Revert(format!(
            "no copied snapshot blob for {rel_path} with matching hash"
        ))
    })?;

    Uuid::parse_str(&id).map_err(|e| AgentHubError::Revert(format!("invalid snapshot uuid: {e}")))
}

fn restore_file_atomic(src: &Path, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }

    if try_exclusive_lock(dest) {
        let tmp = temp_path(dest);
        std::fs::copy(src, &tmp).map_err(|e| {
            AgentHubError::Revert(format!(
                "failed to copy {} -> {}: {e}",
                src.display(),
                tmp.display()
            ))
        })?;
        std::fs::rename(&tmp, dest).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            AgentHubError::Revert(format!(
                "atomic rename {} -> {}: {e}",
                tmp.display(),
                dest.display()
            ))
        })?;
        Ok(())
    } else {
        warn!(
            path = %dest.display(),
            "skipped restore: could not acquire exclusive lock"
        );
        Ok(())
    }
}

fn temp_path(dest: &Path) -> PathBuf {
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    dest.with_file_name(format!("{name}.agenthub_tmp"))
}

fn try_exclusive_lock(path: &Path) -> bool {
    if !path.exists() {
        return true;
    }

    let Ok(file) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
    else {
        return true;
    };

    for attempt in 0..LOCK_RETRIES {
        if file.try_lock_exclusive().is_ok() {
            return true;
        }
        if attempt + 1 < LOCK_RETRIES {
            thread::sleep(Duration::from_millis(LOCK_WAIT_MS));
        }
    }
    false
}

fn list_new_files(cwd: &Path, manifest_paths: &HashSet<String>) -> Result<Vec<PathBuf>> {
    let cwd_buf = cwd.to_path_buf();
    let mut new_files = Vec::new();

    for entry in WalkDir::new(cwd)
        .skip_hidden(false)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let abs = entry.path();
        let rel = rel_path_string(&cwd_buf, &abs)?;
        if is_excluded_rel(&rel) {
            continue;
        }
        if !manifest_paths.contains(&rel) {
            new_files.push(abs.to_path_buf());
        }
    }

    Ok(new_files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_command_variants() {
        assert!(is_undo_command("/undo"));
        assert!(is_undo_command("  /UNDO  "));
        assert!(!is_undo_command("/snapshot"));
    }

    #[test]
    fn prompt_messages_include_counts() {
        let id = Uuid::new_v4();
        assert!(revert_confirmation_message(id, 4, "30s").contains('4'));
        assert!(delete_new_files_message(2).contains('2'));
    }
}
