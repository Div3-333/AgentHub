//! Workspace checkpointing (VFS snapshots) — blueprint §12.1.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use jwalk::WalkDir;
use rayon::prelude::*;
use sqlx::SqlitePool;
use tokio::sync::broadcast;
use tracing::warn;
use uuid::Uuid;

use crate::bus::BusEvent;
use crate::config::AgentHubConfig;
use crate::db::DbClient;
use crate::error::{AgentHubError, Result};
use crate::vfs::{is_excluded_rel, rel_path_string};

pub const MAX_SNAPSHOTS: usize = 20;

/// Why a snapshot was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotTrigger {
    Manual,
    Pipeline,
    Sparring,
    Racing,
    AgentEdit,
}

impl SnapshotTrigger {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Pipeline => "pipeline",
            Self::Sparring => "sparring",
            Self::Racing => "racing",
            Self::AgentEdit => "agent_edit",
        }
    }
}

/// Metadata returned after a successful snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotInfo {
    pub id: Uuid,
    pub file_count: usize,
    pub size_bytes: u64,
}

struct WalkEntry {
    rel_path: String,
    abs_path: PathBuf,
    hash: String,
    size: u64,
}

struct ManifestEntry {
    rel_path: String,
    hash: String,
    status: &'static str,
    size: u64,
}

/// Resolves `config.shadow_dir` relative to `cwd` when needed.
#[must_use]
pub fn resolve_shadow_dir(config: &AgentHubConfig, cwd: &Path) -> PathBuf {
    if config.shadow_dir.is_absolute() {
        config.shadow_dir.clone()
    } else {
        cwd.join(&config.shadow_dir)
    }
}

/// Creates a snapshot using paths from [`AgentHubConfig`].
pub async fn create_snapshot_with_config(
    db: &DbClient,
    config: &AgentHubConfig,
    cwd: &Path,
    session_id: Uuid,
    trigger: SnapshotTrigger,
    bus_tx: Option<&broadcast::Sender<BusEvent>>,
) -> Result<SnapshotInfo> {
    let shadow_dir = resolve_shadow_dir(config, cwd);
    std::fs::create_dir_all(&shadow_dir).map_err(|e| {
        AgentHubError::Snapshot(format!(
            "failed to create shadow dir {}: {e}",
            shadow_dir.display()
        ))
    })?;
    create_snapshot(&db.pool, cwd, &shadow_dir, session_id, trigger, bus_tx).await
}

/// True when `input` is the manual `/snapshot` slash command.
#[must_use]
pub fn is_snapshot_command(input: &str) -> bool {
    matches!(input.trim(), "/snapshot" | "/SNAPSHOT")
        || input.trim().eq_ignore_ascii_case("/snapshot")
}

/// Creates a snapshot of `cwd` and records it in SQLite.
pub async fn create_snapshot(
    pool: &SqlitePool,
    cwd: &Path,
    shadow_dir: &Path,
    session_id: Uuid,
    trigger: SnapshotTrigger,
    bus_tx: Option<&broadcast::Sender<BusEvent>>,
) -> Result<SnapshotInfo> {
    let snapshot_id = Uuid::new_v4();
    let snapshot_dir = shadow_dir.join(snapshot_id.to_string());
    std::fs::create_dir_all(&snapshot_dir).map_err(|e| {
        AgentHubError::Snapshot(format!(
            "failed to create snapshot dir {}: {e}",
            snapshot_dir.display()
        ))
    })?;

    let entries = collect_workspace_files(cwd)?;
    let cwd_str = cwd.to_string_lossy().to_string();
    let manifest = build_manifest(pool, &cwd_str, &entries, &snapshot_dir).await?;

    let file_count = manifest.len();
    let size_bytes: u64 = manifest.iter().map(|e| e.size).sum();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| AgentHubError::Snapshot(format!("system clock before UNIX epoch: {e}")))?
        .as_millis() as i64;

    let mut tx = pool.begin().await?;

    sqlx::query(
        r"
        INSERT INTO snapshots (id, session_id, timestamp, file_count, size_bytes, cwd, trigger)
        VALUES (?, ?, ?, ?, ?, ?, ?)
        ",
    )
    .bind(snapshot_id.to_string())
    .bind(session_id.to_string())
    .bind(timestamp)
    .bind(i64::try_from(file_count).unwrap_or(i64::MAX))
    .bind(i64::try_from(size_bytes).unwrap_or(i64::MAX))
    .bind(cwd.to_string_lossy().as_ref())
    .bind(trigger.as_str())
    .execute(&mut *tx)
    .await?;

    for entry in &manifest {
        sqlx::query(
            r"
            INSERT INTO snapshot_files (id, snapshot_id, rel_path, blake3_hash, status)
            VALUES (?, ?, ?, ?, ?)
            ",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(snapshot_id.to_string())
        .bind(&entry.rel_path)
        .bind(&entry.hash)
        .bind(entry.status)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    prune_old_snapshots(pool, cwd, shadow_dir).await?;

    if let Some(tx) = bus_tx {
        let _ = tx.send(BusEvent::SnapshotCreated {
            snapshot_id,
            file_count,
        });
    }

    Ok(SnapshotInfo {
        id: snapshot_id,
        file_count,
        size_bytes,
    })
}

fn collect_workspace_files(cwd: &Path) -> Result<Vec<WalkEntry>> {
    let cwd_buf = cwd.to_path_buf();
    let entries: Vec<WalkEntry> = WalkDir::new(cwd)
        .skip_hidden(false)
        .follow_links(false)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| {
            let abs_path = entry.path();
            let rel_path = match rel_path_string(&cwd_buf, &abs_path) {
                Ok(p) => p,
                Err(e) => {
                    warn!(path = %abs_path.display(), error = %e, "skipping path outside cwd");
                    return None;
                }
            };
            if is_excluded_rel(&rel_path) {
                return None;
            }
            let hash = match hash_file(&abs_path) {
                Ok(h) => h,
                Err(e) => {
                    warn!(path = %abs_path.display(), error = %e, "skipping unreadable file");
                    return None;
                }
            };
            let size = entry.metadata().ok()?.len();
            Some(WalkEntry {
                rel_path,
                abs_path: abs_path.to_path_buf(),
                hash,
                size,
            })
        })
        .collect();

    Ok(entries)
}

async fn build_manifest(
    pool: &SqlitePool,
    cwd: &str,
    entries: &[WalkEntry],
    snapshot_dir: &Path,
) -> Result<Vec<ManifestEntry>> {
    let mut manifest = Vec::with_capacity(entries.len());
    let mut copies: Vec<(PathBuf, PathBuf, ManifestEntry)> = Vec::new();

    for entry in entries {
        let deduped = find_dedup_source(pool, &entry.rel_path, &entry.hash, cwd).await?;
        if deduped.is_some() {
            manifest.push(ManifestEntry {
                rel_path: entry.rel_path.clone(),
                hash: entry.hash.clone(),
                status: "unchanged",
                size: entry.size,
            });
            continue;
        }

        let dest = snapshot_dir.join(entry.rel_path.replace('/', std::path::MAIN_SEPARATOR_STR));
        copies.push((
            entry.abs_path.clone(),
            dest,
            ManifestEntry {
                rel_path: entry.rel_path.clone(),
                hash: entry.hash.clone(),
                status: "copied",
                size: entry.size,
            },
        ));
    }

    copies.par_iter().try_for_each(|(src, dest, _)| {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                AgentHubError::Snapshot(format!(
                    "failed to create parent {}: {e}",
                    parent.display()
                ))
            })?;
        }
        std::fs::copy(src, dest).map_err(|e| {
            AgentHubError::Snapshot(format!(
                "failed to copy {} -> {}: {e}",
                src.display(),
                dest.display()
            ))
        })?;
        Ok::<(), AgentHubError>(())
    })?;

    manifest.extend(copies.into_iter().map(|(_, _, entry)| entry));
    Ok(manifest)
}

async fn find_dedup_source(
    pool: &SqlitePool,
    rel_path: &str,
    hash: &str,
    cwd: &str,
) -> Result<Option<Uuid>> {
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
    .bind(cwd)
    .fetch_optional(pool)
    .await?;

    row.map(|(id,)| Uuid::parse_str(&id))
        .transpose()
        .map_err(|e| AgentHubError::Snapshot(format!("invalid snapshot uuid in db: {e}")))
}

pub(crate) fn hash_file(path: &Path) -> Result<String> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)
        .map_err(|e| AgentHubError::Snapshot(format!("failed to open {}: {e}", path.display())))?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(|e| {
            AgentHubError::Snapshot(format!("failed to read {}: {e}", path.display()))
        })?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

async fn prune_old_snapshots(pool: &SqlitePool, cwd: &Path, shadow_dir: &Path) -> Result<()> {
    let cwd_str = cwd.to_string_lossy();
    let rows: Vec<(String,)> = sqlx::query_as(
        r"
        SELECT id FROM snapshots
        WHERE cwd = ?
        ORDER BY timestamp ASC
        ",
    )
    .bind(cwd_str.as_ref())
    .fetch_all(pool)
    .await?;

    if rows.len() <= MAX_SNAPSHOTS {
        return Ok(());
    }

    let to_remove = rows.len() - MAX_SNAPSHOTS;
    for (id,) in rows.into_iter().take(to_remove) {
        rehome_orphaned_blobs(pool, shadow_dir, &id, cwd_str.as_ref()).await?;
        delete_snapshot(pool, shadow_dir, &id).await?;
    }

    Ok(())
}

/// Copies blob files into newer snapshots before pruning, so deduped `unchanged`
/// manifest rows still have on-disk backing after the oldest `copied` snapshot is removed.
async fn rehome_orphaned_blobs(
    pool: &SqlitePool,
    shadow_dir: &Path,
    deleting_id: &str,
    cwd: &str,
) -> Result<()> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        r"
        SELECT old.rel_path, old.blake3_hash, u.snapshot_id
        FROM snapshot_files old
        INNER JOIN snapshots so ON so.id = old.snapshot_id
        INNER JOIN snapshot_files u
            ON u.rel_path = old.rel_path AND u.blake3_hash = old.blake3_hash
        INNER JOIN snapshots su ON su.id = u.snapshot_id
        WHERE old.snapshot_id = ?
          AND old.status = 'copied'
          AND u.status = 'unchanged'
          AND so.cwd = ?
          AND su.cwd = ?
          AND su.timestamp > so.timestamp
          AND NOT EXISTS (
            SELECT 1
            FROM snapshot_files c
            INNER JOIN snapshots sc ON sc.id = c.snapshot_id
            WHERE c.rel_path = old.rel_path
              AND c.blake3_hash = old.blake3_hash
              AND c.status = 'copied'
              AND sc.cwd = ?
              AND sc.id != ?
              AND sc.timestamp > so.timestamp
          )
        ORDER BY su.timestamp DESC
        ",
    )
    .bind(deleting_id)
    .bind(cwd)
    .bind(cwd)
    .bind(cwd)
    .bind(deleting_id)
    .fetch_all(pool)
    .await?;

    let mut seen = std::collections::HashSet::new();
    let deleting_uuid = Uuid::parse_str(deleting_id)
        .map_err(|e| AgentHubError::Snapshot(format!("invalid snapshot id for rehome: {e}")))?;

    for (rel_path, _hash, target_id) in rows {
        if !seen.insert(rel_path.clone()) {
            continue;
        }

        let target_uuid = Uuid::parse_str(&target_id).map_err(|e| {
            AgentHubError::Snapshot(format!("invalid rehome target snapshot id: {e}"))
        })?;

        let src = crate::vfs::snapshot_file_path(shadow_dir, deleting_uuid, &rel_path);
        let dest = crate::vfs::snapshot_file_path(shadow_dir, target_uuid, &rel_path);
        if !src.is_file() {
            warn!(
                path = %rel_path,
                src = %src.display(),
                "rehome skipped: source blob missing"
            );
            continue;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                AgentHubError::Snapshot(format!(
                    "failed to create rehome parent {}: {e}",
                    parent.display()
                ))
            })?;
        }
        std::fs::copy(&src, &dest).map_err(|e| {
            AgentHubError::Snapshot(format!(
                "failed to rehome {} -> {}: {e}",
                src.display(),
                dest.display()
            ))
        })?;

        sqlx::query(
            r"
            UPDATE snapshot_files
            SET status = 'copied'
            WHERE snapshot_id = ? AND rel_path = ?
            ",
        )
        .bind(&target_id)
        .bind(&rel_path)
        .execute(pool)
        .await?;
    }

    Ok(())
}

pub(crate) async fn delete_snapshot(
    pool: &SqlitePool,
    shadow_dir: &Path,
    snapshot_id: &str,
) -> Result<()> {
    sqlx::query("DELETE FROM snapshot_files WHERE snapshot_id = ?")
        .bind(snapshot_id)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM snapshots WHERE id = ?")
        .bind(snapshot_id)
        .execute(pool)
        .await?;

    let dir = shadow_dir.join(snapshot_id);
    if dir.is_dir() {
        std::fs::remove_dir_all(&dir).map_err(|e| {
            AgentHubError::Snapshot(format!(
                "failed to remove snapshot dir {}: {e}",
                dir.display()
            ))
        })?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::BUS_CAPACITY;
    use crate::vfs::ensure_session;
    use std::collections::HashMap;
    use std::fs;
    use tempfile::TempDir;

    async fn test_db() -> (TempDir, DbClient) {
        let dir = TempDir::new().expect("tempdir");
        let url = format!("sqlite:{}", dir.path().join("test.db").display());
        let db = DbClient::init_pool(&url).await.expect("pool");
        db.run_migrations().await.expect("migrate");
        (dir, db)
    }

    async fn file_statuses(pool: &SqlitePool, snapshot_id: Uuid) -> HashMap<String, String> {
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT rel_path, status FROM snapshot_files WHERE snapshot_id = ?")
                .bind(snapshot_id.to_string())
                .fetch_all(pool)
                .await
                .expect("query files");
        rows.into_iter().collect()
    }

    #[test]
    fn hash_file_is_deterministic() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("a.txt");
        std::fs::write(&path, b"hello").expect("write");
        let h1 = hash_file(&path).expect("hash");
        let h2 = hash_file(&path).expect("hash");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
    }

    #[test]
    fn is_snapshot_command_recognizes_variants() {
        assert!(is_snapshot_command("/snapshot"));
        assert!(is_snapshot_command("  /SNAPSHOT  "));
        assert!(!is_snapshot_command("/undo"));
    }

    #[tokio::test]
    async fn create_snapshot_copies_files_and_writes_manifest() {
        let (_db_dir, db) = test_db().await;
        let workspace = TempDir::new().expect("workspace");
        let cwd = workspace.path();
        fs::write(cwd.join("hello.txt"), b"hello").expect("write");

        let session_id = Uuid::new_v4();
        ensure_session(&db.pool, session_id, cwd)
            .await
            .expect("session");

        let shadow = cwd.join(".agenthub_shadow");
        let info = create_snapshot(
            &db.pool,
            cwd,
            &shadow,
            session_id,
            SnapshotTrigger::Manual,
            None,
        )
        .await
        .expect("snapshot");

        assert_eq!(info.file_count, 1);
        assert!(info.size_bytes > 0);
        assert!(shadow.join(info.id.to_string()).join("hello.txt").is_file());

        let statuses = file_statuses(&db.pool, info.id).await;
        assert_eq!(
            statuses.get("hello.txt").map(String::as_str),
            Some("copied")
        );
    }

    #[tokio::test]
    async fn second_snapshot_dedups_unchanged_files() {
        let (_db_dir, db) = test_db().await;
        let workspace = TempDir::new().expect("workspace");
        let cwd = workspace.path();
        fs::write(cwd.join("stable.txt"), b"same").expect("write");

        let session_id = Uuid::new_v4();
        ensure_session(&db.pool, session_id, cwd)
            .await
            .expect("session");

        let shadow = cwd.join(".agenthub_shadow");

        create_snapshot(
            &db.pool,
            cwd,
            &shadow,
            session_id,
            SnapshotTrigger::Pipeline,
            None,
        )
        .await
        .expect("first");

        let second = create_snapshot(
            &db.pool,
            cwd,
            &shadow,
            session_id,
            SnapshotTrigger::Manual,
            None,
        )
        .await
        .expect("second");

        let statuses = file_statuses(&db.pool, second.id).await;
        assert_eq!(
            statuses.get("stable.txt").map(String::as_str),
            Some("unchanged")
        );
        assert!(!shadow
            .join(second.id.to_string())
            .join("stable.txt")
            .exists());
    }

    #[tokio::test]
    async fn prune_rehomes_dedup_blobs_before_deleting_oldest() {
        let (_db_dir, db) = test_db().await;
        let workspace = TempDir::new().expect("workspace");
        let cwd = workspace.path();
        fs::write(cwd.join("persist.txt"), b"v1").expect("write");

        let session_id = Uuid::new_v4();
        ensure_session(&db.pool, session_id, cwd)
            .await
            .expect("session");

        let shadow = cwd.join(".agenthub_shadow");
        let first = create_snapshot(
            &db.pool,
            cwd,
            &shadow,
            session_id,
            SnapshotTrigger::Manual,
            None,
        )
        .await
        .expect("first");

        for _ in 0..MAX_SNAPSHOTS {
            create_snapshot(
                &db.pool,
                cwd,
                &shadow,
                session_id,
                SnapshotTrigger::Pipeline,
                None,
            )
            .await
            .expect("dedup snapshot");
        }

        let oldest: (String,) =
            sqlx::query_as("SELECT id FROM snapshots ORDER BY timestamp ASC LIMIT 1")
                .fetch_one(&db.pool)
                .await
                .expect("oldest");
        assert_ne!(oldest.0, first.id.to_string());

        let latest: (String,) =
            sqlx::query_as("SELECT id FROM snapshots ORDER BY timestamp DESC LIMIT 1")
                .fetch_one(&db.pool)
                .await
                .expect("latest");

        let status: (String,) = sqlx::query_as(
            "SELECT status FROM snapshot_files WHERE snapshot_id = ? AND rel_path = 'persist.txt'",
        )
        .bind(&latest.0)
        .fetch_one(&db.pool)
        .await
        .expect("status");
        assert_eq!(
            status.0, "copied",
            "latest snapshot should own rehomed blob after prune"
        );
        assert!(
            shadow.join(&latest.0).join("persist.txt").is_file(),
            "rehomed blob must exist on disk"
        );
    }

    #[tokio::test]
    async fn rotation_keeps_at_most_twenty_snapshots() {
        let (_db_dir, db) = test_db().await;
        let workspace = TempDir::new().expect("workspace");
        let cwd = workspace.path();
        fs::write(cwd.join("f.txt"), b"x").expect("write");

        let session_id = Uuid::new_v4();
        ensure_session(&db.pool, session_id, cwd)
            .await
            .expect("session");

        let shadow = cwd.join(".agenthub_shadow");

        for _ in 0..=MAX_SNAPSHOTS {
            create_snapshot(
                &db.pool,
                cwd,
                &shadow,
                session_id,
                SnapshotTrigger::Manual,
                None,
            )
            .await
            .expect("snapshot");
        }

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM snapshots")
            .fetch_one(&db.pool)
            .await
            .expect("count");
        assert_eq!(count.0, MAX_SNAPSHOTS as i64);
    }

    #[tokio::test]
    async fn slash_command_emits_bus_event() {
        let (_db_dir, db) = test_db().await;
        let workspace = TempDir::new().expect("workspace");
        let cwd = workspace.path();
        fs::write(cwd.join("a.txt"), b"a").expect("write");

        let session_id = Uuid::new_v4();
        ensure_session(&db.pool, session_id, cwd)
            .await
            .expect("session");

        let mut config = AgentHubConfig::default();
        config.shadow_dir = PathBuf::from(".agenthub_shadow");

        let (bus_tx, mut bus_rx) = broadcast::channel(BUS_CAPACITY);

        let msg = crate::vfs::handle_slash_command(
            "/snapshot",
            &db,
            &config,
            cwd,
            session_id,
            Some(&bus_tx),
            None,
        )
        .await
        .expect("handle")
        .expect("response");

        assert!(msg.contains("[VFS]"));
        assert!(matches!(
            bus_rx.try_recv().expect("event"),
            BusEvent::SnapshotCreated { .. }
        ));
    }
}
