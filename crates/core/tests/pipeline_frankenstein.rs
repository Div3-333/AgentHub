// Integration: Frankenstein pipeline + sparring (blueprint Phase 7 / Part 10).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use agenthub_core::bus::{BusEvent, BUS_CAPACITY};
use agenthub_core::config::{AgentHubConfig, DriverProfile};
use agenthub_core::db::DbClient;
use agenthub_core::error::AgentHubError;
use agenthub_core::pipeline::pty_bridge::spawn_agent_message_bridge;
use agenthub_core::pipeline::{
    parse, parse_spar_command, PipelineExecutor, SparEngine, SPAR_ABORT,
};
use agenthub_core::pty::{pty_tests_enabled, spawn_agent, SpawnOptions};
use agenthub_core::server::ServerState;
use agenthub_core::vfs::SnapshotTrigger;
use tokio::sync::broadcast;

include!("common/mock_cli_exe.rs");

fn write_mock_driver(
    drivers_dir: &std::path::Path,
    executable: &std::path::Path,
    echo_input: bool,
) {
    let mut env = HashMap::from([
        ("NO_COLOR".to_string(), "1".to_string()),
        ("TERM".to_string(), "dumb".to_string()),
    ]);
    if echo_input {
        env.insert("MOCK_CLI_ECHO_INPUT".to_string(), "1".to_string());
    }
    let profile = DriverProfile {
        name: "mock_cli".to_string(),
        display_name: "Mock CLI".to_string(),
        executable: executable.to_string_lossy().into_owned(),
        args: vec![],
        env,
        prompt_regex: "^>\\s*$".to_string(),
        silence_timeout_ms: 200,
        init_sequence: vec![],
        rate_limit_patterns: vec![],
        auto_reply_patterns: HashMap::new(),
        supports_multi_instance: true,
        max_instances: 0,
    };
    let json = serde_json::to_string_pretty(&profile).expect("serialize driver");
    std::fs::write(drivers_dir.join("mock_cli.json"), json).expect("write driver json");
}

async fn spawn_mock_with_sanitizer(
    state: Arc<ServerState>,
    bus_tx: broadcast::Sender<BusEvent>,
    config: &AgentHubConfig,
) -> (uuid::Uuid, Arc<agenthub_core::pty::AgentPty>, DriverProfile) {
    let id = spawn_agent(
        "mock_cli",
        config,
        Arc::clone(&state),
        bus_tx.clone(),
        None,
        SpawnOptions::default(),
    )
    .await
    .expect("spawn_agent");
    let agent = state.agents.get(&id).expect("agent").value().clone();
    let driver =
        agenthub_core::config::load_driver_profile_from_dir(&config.drivers_dir, "mock_cli")
            .expect("driver");
    agenthub_core::pipeline::executor::ensure_agent_rbac(&state, &agent);
    tokio::spawn(spawn_agent_message_bridge(
        agent.clone(),
        driver.clone(),
        bus_tx,
    ));
    // Wait for initial prompt.
    tokio::time::sleep(Duration::from_millis(300)).await;
    (id, agent, driver)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pipeline_frankenstein_three_stage_execution() {
    if !pty_tests_enabled() {
        eprintln!("skip: PTY integration tests disabled (AGENTHUB_SKIP_PTY or Windows without AGENTHUB_FORCE_PTY)");
        return;
    }

    tokio::time::timeout(Duration::from_secs(90), async {
        let dir = tempfile::tempdir().expect("tempdir");
        let drivers_dir = dir.path().join("drivers");
        std::fs::create_dir_all(&drivers_dir).expect("mkdir drivers");
        write_mock_driver(&drivers_dir, &mock_cli_executable(), true);

        let config = AgentHubConfig {
            drivers_dir,
            max_agents: 8,
            ..AgentHubConfig::default()
        };
        let state = Arc::new(ServerState::new());
        let (bus_tx, _bus_rx) = broadcast::channel(BUS_CAPACITY);

        let (_id1, _a1, _) =
            spawn_mock_with_sanitizer(Arc::clone(&state), bus_tx.clone(), &config).await;
        let (_id2, _a2, _) =
            spawn_mock_with_sanitizer(Arc::clone(&state), bus_tx.clone(), &config).await;

        let definition = "@mock_cli-1 hello | > echo world | @mock_cli-2 repeat";
        let stages = parse(definition).expect("parse");
        assert_eq!(stages.len(), 3);

        let exec = PipelineExecutor::new(
            Arc::clone(&state),
            bus_tx.clone(),
            dir.path(),
            uuid::Uuid::new_v4(),
            config,
            None,
        );

        let result = exec.execute(definition).await.expect("pipeline execute");
        assert!(!result.final_output.is_empty());
    })
    .await
    .expect("pipeline_frankenstein_three_stage_execution timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pipeline_unix_failure_stops_pipeline() {
    if !pty_tests_enabled() {
        eprintln!("skip: PTY integration tests disabled (AGENTHUB_SKIP_PTY or Windows without AGENTHUB_FORCE_PTY)");
        return;
    }

    tokio::time::timeout(Duration::from_secs(60), async {
        let dir = tempfile::tempdir().expect("tempdir");
        let drivers_dir = dir.path().join("drivers");
        std::fs::create_dir_all(&drivers_dir).expect("mkdir drivers");
        write_mock_driver(&drivers_dir, &mock_cli_executable(), true);

        let config = AgentHubConfig {
            drivers_dir,
            max_agents: 4,
            ..AgentHubConfig::default()
        };
        let state = Arc::new(ServerState::new());
        let (bus_tx, _bus_rx) = broadcast::channel(BUS_CAPACITY);

        let (_id, _agent, _) =
            spawn_mock_with_sanitizer(Arc::clone(&state), bus_tx.clone(), &config).await;

        let definition = "@mock_cli-1 hello | > exit 1";
        let exec = PipelineExecutor::new(
            Arc::clone(&state),
            bus_tx,
            dir.path(),
            uuid::Uuid::new_v4(),
            config,
            None,
        );

        let err = exec
            .execute(definition)
            .await
            .expect_err("pipeline should fail");
        assert!(matches!(
            err,
            agenthub_core::error::AgentHubError::PipelineExecution { .. }
        ));
    })
    .await
    .expect("pipeline_unix_failure_stops_pipeline timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn spar_completes_three_turns() {
    if !pty_tests_enabled() {
        eprintln!("skip: PTY integration tests disabled (AGENTHUB_SKIP_PTY or Windows without AGENTHUB_FORCE_PTY)");
        return;
    }

    tokio::time::timeout(Duration::from_secs(120), async {
        let dir = tempfile::tempdir().expect("tempdir");
        let drivers_dir = dir.path().join("drivers");
        std::fs::create_dir_all(&drivers_dir).expect("mkdir drivers");
        write_mock_driver(&drivers_dir, &mock_cli_executable(), true);

        let config = AgentHubConfig {
            drivers_dir,
            max_agents: 8,
            ..AgentHubConfig::default()
        };
        let state = Arc::new(ServerState::new());
        let (bus_tx, _bus_rx) = broadcast::channel(BUS_CAPACITY);

        spawn_mock_with_sanitizer(Arc::clone(&state), bus_tx.clone(), &config).await;
        spawn_mock_with_sanitizer(Arc::clone(&state), bus_tx.clone(), &config).await;

        let spar_cfg = parse_spar_command(
            "/spar @mock_cli-1 as Coder vs @mock_cli-2 as Reviewer --turns 3 --goal \"ping\"",
        )
        .expect("parse spar");

        let engine = SparEngine::new(
            Arc::clone(&state),
            bus_tx,
            dir.path(),
            uuid::Uuid::new_v4(),
            config,
            None,
        );

        let result = engine.run(&spar_cfg).await.expect("spar run");
        assert_eq!(result.turns_completed, 3);
        assert!(!result.aborted);
        assert!(!result.stagnation);
    })
    .await
    .expect("spar_completes_three_turns timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spar_escape_aborts_within_half_second() {
    SPAR_ABORT.store(false, std::sync::atomic::Ordering::SeqCst);

    let state = Arc::new(ServerState::new());
    let (bus_tx, mut bus_rx) = broadcast::channel(BUS_CAPACITY);

    let spar_cfg =
        parse_spar_command("/spar @mock_cli-1 as A vs @mock_cli-2 as B --turns 5 --goal test")
            .expect("parse");

    let engine = SparEngine::new(
        Arc::clone(&state),
        bus_tx,
        std::env::temp_dir(),
        uuid::Uuid::new_v4(),
        AgentHubConfig::default(),
        None,
    );

    let handle = tokio::spawn(async move { engine.run(&spar_cfg).await });

    tokio::time::sleep(Duration::from_millis(50)).await;
    SPAR_ABORT.store(true, std::sync::atomic::Ordering::SeqCst);

    let result = tokio::time::timeout(Duration::from_millis(500), handle)
        .await
        .expect("abort should finish within 500ms")
        .expect("join")
        .expect("spar result");

    assert!(result.aborted);

    let mut saw_abort_msg = false;
    while let Ok(event) = bus_rx.try_recv() {
        if let BusEvent::SystemMessage { content, .. } = event {
            if content.contains("Manually aborted") {
                saw_abort_msg = true;
            }
        }
    }
    assert!(saw_abort_msg);
}

async fn test_db() -> (tempfile::TempDir, DbClient) {
    let dir = tempfile::tempdir().expect("tempdir");
    let url = format!("sqlite:{}", dir.path().join("test.db").display());
    let db = DbClient::init_pool(&url).await.expect("pool");
    db.run_migrations().await.expect("migrate");
    (dir, db)
}

#[test]
fn frankenstein_parser_edge_cases() {
    let stages = parse("@gemini a|b").expect("inline pipe stays in prompt");
    assert_eq!(stages.len(), 1);

    let err = parse("@gemini ok | @ | > echo").unwrap_err();
    assert!(matches!(err, AgentHubError::PipelineParse { .. }));

    let broadcast = parse("kick off | > echo done").expect("broadcast");
    assert!(matches!(
        &broadcast[0],
        agenthub_core::pipeline::PipelineStage::Agent(a) if a.tag.is_none()
    ));
}

#[tokio::test]
async fn pipeline_vfs_snapshot_before_stages() {
    let workspace = tempfile::tempdir().expect("workspace");
    let (_db_dir, db) = test_db().await;
    let db = Arc::new(db);

    let (bus_tx, mut bus_rx) = broadcast::channel(BUS_CAPACITY);
    let session_id = uuid::Uuid::new_v4();
    let exec = PipelineExecutor::new(
        Arc::new(ServerState::new()),
        bus_tx,
        workspace.path(),
        session_id,
        AgentHubConfig::default(),
        Some(Arc::clone(&db)),
    );

    let result = exec
        .execute("> echo snapshot_ok")
        .await
        .expect("unix-only pipeline");
    let snapshot_id = result.snapshot_id.expect("§10.2 snapshot before stages");

    let mut saw_pipeline_started = false;
    let mut saw_snapshot = false;
    while let Ok(event) = bus_rx.try_recv() {
        match event {
            BusEvent::PipelineStarted { .. } => saw_pipeline_started = true,
            BusEvent::SnapshotCreated {
                snapshot_id: id, ..
            } => {
                assert_eq!(id, snapshot_id);
                saw_snapshot = true;
            }
            _ => {}
        }
    }
    assert!(saw_pipeline_started);
    assert!(saw_snapshot);

    let pipeline = db
        .get_pipeline(result.pipeline_id)
        .await
        .expect("pipeline row")
        .expect("pipeline exists");
    assert_eq!(pipeline.snapshot_id, Some(snapshot_id));

    let snapshot = db
        .get_snapshot(snapshot_id)
        .await
        .expect("snapshot row")
        .expect("snapshot exists");
    assert_eq!(snapshot.trigger, SnapshotTrigger::Pipeline.as_str());
}

#[test]
fn spar_stagnation_detects_identical_responses() {
    use agenthub_core::pipeline::loop_engine::{check_stagnation, similarity_ratio};

    let mut hist = Vec::new();
    assert!((similarity_ratio("same", "same") - 1.0).abs() < f64::EPSILON);
    assert!(!check_stagnation(&mut hist, "repeat"));
    assert!(check_stagnation(&mut hist, "repeat"));
}
