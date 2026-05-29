// VFS snapshot + revert integration (blueprint Phase 9 / §12).
//
// Run: `cargo test -p agenthub-core vfs`

use std::fs;
use std::sync::Arc;

use agenthub_core::bus::{BusEvent, BUS_CAPACITY};
use agenthub_core::db::DbClient;
use agenthub_core::vfs::snapshot::SnapshotTrigger;
use agenthub_core::vfs::{ensure_session, revert, RevertOptions, VfsEngine};
use tempfile::TempDir;
use tokio::sync::broadcast;
use uuid::Uuid;

async fn test_db() -> (TempDir, DbClient) {
    let dir = TempDir::new().expect("tempdir");
    let url = format!("sqlite:{}", dir.path().join("test.db").display());
    let db = DbClient::init_pool(&url).await.expect("pool");
    db.run_migrations().await.expect("migrate");
    (dir, db)
}

#[tokio::test]
async fn vfs_snapshot_then_revert_restores_workspace() {
    let (_db_dir, db) = test_db().await;
    let workspace = TempDir::new().expect("workspace");
    let cwd = workspace.path();
    let tracked = cwd.join("tracked.txt");
    fs::write(&tracked, b"before agents").expect("write");

    let session_id = Uuid::new_v4();
    ensure_session(&db.pool, session_id, cwd)
        .await
        .expect("session");

    let shadow = cwd.join(".agenthub_shadow");
    let (bus_tx, mut bus_rx) = broadcast::channel(BUS_CAPACITY);
    let engine = VfsEngine::new(cwd, &shadow, Arc::new(db), session_id, Some(bus_tx));

    let snap = engine
        .snapshot(SnapshotTrigger::Manual)
        .await
        .expect("snapshot");
    assert!(matches!(
        bus_rx.try_recv().expect("event"),
        BusEvent::SnapshotCreated { .. }
    ));

    fs::write(&tracked, b"after agents").expect("modify");
    fs::write(cwd.join("agent_created.txt"), b"rogue").expect("new file");

    let revert_result = engine
        .revert_latest(
            RevertOptions {
                delete_new_files: true,
                dry_run: false,
            },
            &[],
        )
        .await
        .expect("revert");

    assert_eq!(fs::read_to_string(&tracked).expect("read"), "before agents");
    assert!(!cwd.join("agent_created.txt").exists());
    assert!(revert_result.files_restored >= 1);
    assert_eq!(revert_result.new_files_deleted, 1);
    assert_eq!(revert_result.snapshot_id, snap.id);

    assert!(matches!(
        bus_rx.try_recv().expect("initiated"),
        BusEvent::RevertInitiated { .. }
    ));
    assert!(matches!(
        bus_rx.try_recv().expect("complete"),
        BusEvent::RevertComplete { .. }
    ));
}

#[tokio::test]
async fn vfs_revert_restores_deduped_unchanged_file() {
    let (_db_dir, db) = test_db().await;
    let workspace = TempDir::new().expect("workspace");
    let cwd = workspace.path();
    let tracked = cwd.join("dedup.txt");
    fs::write(&tracked, b"original").expect("write");

    let session_id = Uuid::new_v4();
    ensure_session(&db.pool, session_id, cwd)
        .await
        .expect("session");

    let shadow = cwd.join(".agenthub_shadow");
    let engine = VfsEngine::new(cwd, &shadow, Arc::new(db), session_id, None);

    engine
        .snapshot(SnapshotTrigger::Pipeline)
        .await
        .expect("first");
    engine
        .snapshot(SnapshotTrigger::Manual)
        .await
        .expect("second dedup");

    fs::write(&tracked, b"mutated").expect("modify");

    let result = engine
        .revert_latest(RevertOptions::default(), &[])
        .await
        .expect("revert");

    assert_eq!(fs::read_to_string(&tracked).expect("read"), "original");
    assert!(result.files_restored >= 1);
}

#[test]
fn vfs_revert_prompt_helpers() {
    let id = Uuid::new_v4();
    assert!(revert::revert_confirmation_message(id, 4, "30s").contains("4"));
    assert!(revert::delete_new_files_message(2).contains("2"));
    assert!(revert::is_undo_command("/undo"));
}
