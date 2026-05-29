// Phase 6 DoD: RBAC, moderation slash commands, and grand induction (blueprint §8).

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use agenthub_core::bus::{BusEvent, OfflineReason, BUS_CAPACITY};
use agenthub_core::config::{AgentHubConfig, DriverProfile};
use agenthub_core::pty::mock_agent_with_capture;
use agenthub_core::pty::{kill_agent, pty_tests_enabled, spawn_agent, PtyStatus, SpawnOptions};
use agenthub_core::server::induction::{output_contains_ready, run_induction_for_test};
use agenthub_core::server::moderation::{execute_command, ModerationContext};
use agenthub_core::server::rbac::{
    default_roles, load_custom_roles, permissions_for_role, roles_json_path, Permissions,
    BUILTIN_ROLES,
};
use agenthub_core::server::ServerState;
use agenthub_core::server::{set_workspace_mode, WorkspaceModeId};
use tokio::sync::broadcast;
use uuid::Uuid;

include!("common/mock_cli_exe.rs");

fn write_mock_driver(drivers_dir: &std::path::Path, executable: &std::path::Path) {
    let profile = DriverProfile {
        name: "mock_cli".to_string(),
        display_name: "Mock CLI".to_string(),
        executable: executable.to_string_lossy().into_owned(),
        args: vec![],
        env: HashMap::from([
            ("NO_COLOR".to_string(), "1".to_string()),
            ("TERM".to_string(), "dumb".to_string()),
        ]),
        prompt_regex: "^>\\s*$".to_string(),
        silence_timeout_ms: 3000,
        init_sequence: vec![],
        rate_limit_patterns: vec![],
        auto_reply_patterns: HashMap::new(),
        supports_multi_instance: true,
        max_instances: 0,
    };
    let json = serde_json::to_string_pretty(&profile).expect("serialize driver");
    std::fs::write(drivers_dir.join("mock_cli.json"), json).expect("write driver json");
}

fn server_mode_state() -> Arc<ServerState> {
    let state = Arc::new(ServerState::new());
    state
        .mode
        .store(WorkspaceModeId::Server.as_u8(), Ordering::Release);
    state
}

fn moderation_ctx(
    state: Arc<ServerState>,
    bus_tx: broadcast::Sender<BusEvent>,
) -> ModerationContext {
    ModerationContext {
        state,
        config: Arc::new(AgentHubConfig::default()),
        db: None,
        bus_tx,
        issued_by: "user".to_string(),
        caller_agent_id: None,
    }
}

fn register_mock_agent(
    state: &ServerState,
    tag: &str,
    role: &str,
) -> (Uuid, Arc<agenthub_core::pty::AgentPty>) {
    let id = Uuid::new_v4();
    let (agent, _capture) = mock_agent_with_capture(id, tag, PtyStatus::Idle, true);
    let perms = state
        .permissions_for_role(role)
        .unwrap_or_else(|| default_roles()["Builder"]);
    agent.set_role(role);
    agent.set_permissions(perms);
    state
        .register_agent_state(id, tag.to_string(), "mock".into(), role, 1)
        .expect("register_agent_state");
    state.agents.insert(id, Arc::clone(&agent));
    (id, agent)
}

async fn drain_bus_until<F>(
    bus_rx: &mut broadcast::Receiver<BusEvent>,
    timeout: Duration,
    pred: F,
) -> bool
where
    F: Fn(&BusEvent) -> bool,
{
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(50), bus_rx.recv()).await {
            Ok(Ok(event)) if pred(&event) => return true,
            Ok(Ok(_)) => {}
            Ok(Err(_)) => break,
            Err(_) => {}
        }
    }
    false
}

#[test]
fn builtin_roles_match_blueprint() {
    let roles = default_roles();
    assert_eq!(roles.len(), BUILTIN_ROLES.len());
    for name in BUILTIN_ROLES {
        assert!(roles.contains_key(*name), "missing {name}");
    }
    assert!(roles["Leader"].contains(Permissions::MODIFY_ROLES));
    assert_eq!(roles["Observer"], Permissions::VIEW_CHANNEL);
}

#[tokio::test]
async fn promote_updates_permissions_atomically() {
    let state = server_mode_state();
    let (bus_tx, _) = broadcast::channel(BUS_CAPACITY);
    let (id, agent) = register_mock_agent(&state, "mock-1", "Observer");
    let ctx = moderation_ctx(Arc::clone(&state), bus_tx);

    execute_command(&ctx, "/promote @mock-1 to Leader")
        .await
        .expect("promote");

    let meta = state.agent_states.get(&id).expect("meta");
    assert_eq!(meta.role, "Leader");
    assert!(meta.permissions.contains(Permissions::MODIFY_ROLES));
    assert_eq!(agent.role(), "Leader");
    assert!(agent.permissions().contains(Permissions::MODIFY_ROLES));
}

#[tokio::test]
async fn mute_hides_agent_from_chat() {
    let state = Arc::new(ServerState::new());
    let (bus_tx, mut bus_rx) = broadcast::channel(BUS_CAPACITY);
    let (_id, agent) = register_mock_agent(&state, "mock-1", "Builder");
    let ctx = moderation_ctx(Arc::clone(&state), bus_tx);

    execute_command(&ctx, "/mute @mock-1").await.expect("mute");
    assert!(!agent.visible_in_chat.load(Ordering::Acquire));
    assert!(
        drain_bus_until(&mut bus_rx, Duration::from_secs(1), |e| {
            matches!(e, BusEvent::AgentMuted { .. })
        })
        .await
    );
}

#[tokio::test]
async fn deafen_stops_broadcast_reception() {
    let state = Arc::new(ServerState::new());
    let (bus_tx, mut bus_rx) = broadcast::channel(BUS_CAPACITY);
    let (_id, agent) = register_mock_agent(&state, "mock-1", "Builder");
    let ctx = moderation_ctx(Arc::clone(&state), bus_tx);

    execute_command(&ctx, "/deafen @mock-1")
        .await
        .expect("deafen");
    assert!(!agent.receives_broadcast.load(Ordering::Acquire));
    assert!(
        drain_bus_until(&mut bus_rx, Duration::from_secs(1), |e| {
            matches!(e, BusEvent::AgentDeafened { .. })
        })
        .await
    );
}

#[tokio::test]
async fn timeout_sets_suspended_and_clears_after_duration() {
    let state = Arc::new(ServerState::new());
    let (bus_tx, mut bus_rx) = broadcast::channel(BUS_CAPACITY);
    let (id, agent) = register_mock_agent(&state, "mock-1", "Builder");
    let ctx = moderation_ctx(Arc::clone(&state), bus_tx);

    execute_command(&ctx, "/timeout @mock-1 1s")
        .await
        .expect("timeout");
    assert_eq!(agent.status(), Some(PtyStatus::Suspended));
    assert!(state.agent_states.get(&id).unwrap().is_timed_out());
    assert!(
        drain_bus_until(&mut bus_rx, Duration::from_secs(1), |e| {
            matches!(e, BusEvent::AgentTimedOut { .. })
        })
        .await
    );

    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(agent.status(), Some(PtyStatus::Idle));
    assert!(!state.agent_states.get(&id).unwrap().is_timed_out());
}

#[tokio::test]
async fn kick_removes_agent_from_server_state() {
    let state = Arc::new(ServerState::new());
    let (bus_tx, mut bus_rx) = broadcast::channel(BUS_CAPACITY);
    let (id, _agent) = register_mock_agent(&state, "mock-1", "Builder");
    let ctx = moderation_ctx(Arc::clone(&state), bus_tx);

    execute_command(&ctx, "/kick @mock-1").await.expect("kick");
    assert!(!state.agents.contains_key(&id));
    assert!(!state.agent_states.contains_key(&id));
    assert!(
        drain_bus_until(&mut bus_rx, Duration::from_secs(1), |e| {
            matches!(e, BusEvent::AgentKicked { .. })
        })
        .await
    );
}

#[tokio::test]
async fn demote_reverts_to_observer() {
    let state = server_mode_state();
    let (bus_tx, _) = broadcast::channel(BUS_CAPACITY);
    let (id, agent) = register_mock_agent(&state, "mock-1", "Leader");
    let ctx = moderation_ctx(Arc::clone(&state), bus_tx);

    execute_command(&ctx, "/demote @mock-1")
        .await
        .expect("demote");
    assert_eq!(state.agent_states.get(&id).unwrap().role, "Observer");
    assert_eq!(agent.role(), "Observer");
    assert_eq!(
        state.agent_states.get(&id).unwrap().permissions,
        Permissions::VIEW_CHANNEL
    );
    assert_eq!(agent.permissions(), Permissions::VIEW_CHANNEL);
}

#[tokio::test]
async fn addrole_persists_custom_role_to_config_dir() {
    let config_dir = tempfile::tempdir().expect("tempdir");
    std::env::set_var("AGENTHUB_CONFIG_DIR", config_dir.path());

    let state = Arc::new(ServerState::new());
    state
        .mode
        .store(WorkspaceModeId::Server.as_u8(), Ordering::Release);
    let (bus_tx, _) = broadcast::channel(BUS_CAPACITY);
    let ctx = moderation_ctx(Arc::clone(&state), bus_tx);

    execute_command(
        &ctx,
        "/addrole SecurityAuditor VIEW_CHANNEL SEND_MESSAGES RECEIVE_BROADCAST",
    )
    .await
    .expect("addrole");

    assert!(state.roles.contains_key("SecurityAuditor"));
    let loaded = load_custom_roles(&roles_json_path()).expect("load");
    assert!(loaded.iter().any(|r| r.name == "SecurityAuditor"));

    std::env::remove_var("AGENTHUB_CONFIG_DIR");
}

#[tokio::test]
async fn agent_caller_without_modify_roles_denied_promote() {
    let state = Arc::new(ServerState::new());
    state
        .mode
        .store(WorkspaceModeId::Server.as_u8(), Ordering::Release);
    let (bus_tx, _) = broadcast::channel(BUS_CAPACITY);
    let (mod_id, _) = register_mock_agent(&state, "mod-1", "Moderator");
    let (_target_id, _) = register_mock_agent(&state, "mock-1", "Observer");

    let ctx = ModerationContext {
        state: Arc::clone(&state),
        config: Arc::new(AgentHubConfig::default()),
        db: None,
        bus_tx,
        issued_by: "agent".to_string(),
        caller_agent_id: Some(mod_id),
    };
    execute_command(&ctx, "/promote @mock-1 to Leader")
        .await
        .expect("moderator may promote");

    let (builder_id, _) = register_mock_agent(&state, "builder-1", "Builder");
    let ctx = ModerationContext {
        state: Arc::clone(&state),
        config: Arc::new(AgentHubConfig::default()),
        db: None,
        bus_tx: broadcast::channel(BUS_CAPACITY).0,
        issued_by: "agent".to_string(),
        caller_agent_id: Some(builder_id),
    };
    let err = execute_command(&ctx, "/promote @mock-1 to Leader")
        .await
        .expect_err("builder lacks MODIFY_ROLES");
    assert!(matches!(
        err,
        agenthub_core::error::AgentHubError::PermissionDenied { .. }
    ));
}

#[tokio::test]
async fn resolve_tag_is_case_insensitive() {
    let state = Arc::new(ServerState::new());
    let (bus_tx, _) = broadcast::channel(BUS_CAPACITY);
    register_mock_agent(&state, "Mock-1", "Builder");
    let ctx = moderation_ctx(Arc::clone(&state), bus_tx);

    execute_command(&ctx, "/mute @MOCK-1")
        .await
        .expect("case-insensitive tag");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn grand_induction_ready_before_online() {
    if !pty_tests_enabled() {
        eprintln!("skip: AGENTHUB_SKIP_PTY is set");
        return;
    }
    std::env::set_var("AGENTHUB_INDUCTION_TIMEOUT_MS", "8000");

    tokio::time::timeout(Duration::from_secs(25), async {
        let dir = tempfile::tempdir().expect("tempdir");
        let drivers_dir = dir.path().join("drivers");
        std::fs::create_dir_all(&drivers_dir).expect("mkdir drivers");
        write_mock_driver(&drivers_dir, &mock_cli_executable());

        let config = AgentHubConfig {
            drivers_dir,
            max_agents: 4,
            ..AgentHubConfig::default()
        };
        let state = Arc::new(ServerState::new());
        set_workspace_mode(&state, WorkspaceModeId::GroupChat).expect("mode");
        let (bus_tx, mut bus_rx) = broadcast::channel(BUS_CAPACITY);

        let id = spawn_agent(
            "mock_cli",
            &config,
            Arc::clone(&state),
            bus_tx.clone(),
            None,
            SpawnOptions {
                tag: Some("mock-1".into()),
                role: Some("Builder".into()),
                ..SpawnOptions::default()
            },
        )
        .await
        .expect("spawn");

        let agent = state.agents.get(&id).expect("agent").value().clone();

        assert!(
            drain_bus_until(&mut bus_rx, Duration::from_secs(20), |e| {
                matches!(
                    e,
                    BusEvent::AgentOnline { tag, .. } if tag == "mock-1"
                )
            })
            .await
        );

        let bytes = agent.ring_buffer().peek_all();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            output_contains_ready(&text),
            "mock CLI should emit READY for induction; got: {text}"
        );

        kill_agent(&state, id, &bus_tx, OfflineReason::Kicked).expect("cleanup");
        std::env::remove_var("AGENTHUB_INDUCTION_TIMEOUT_MS");
    })
    .await
    .expect("grand_induction_ready_before_online timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn run_induction_for_test_accepts_ready() {
    let state = ServerState::new();
    let (agent, capture) =
        mock_agent_with_capture(Uuid::new_v4(), "mock-1", PtyStatus::Initializing, true);
    state.agents.insert(agent.id, Arc::clone(&agent));
    state
        .register_agent_state(agent.id, "mock-1".into(), "mock".into(), "Builder", 1)
        .expect("register");

    let agent_c = Arc::clone(&agent);
    let state_c = Arc::new(state);
    let inject = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(80)).await;
        let _ = agent_c.ring_buffer().write(b"READY\n");
    });

    run_induction_for_test(agent, &state_c)
        .await
        .expect("READY induction");
    inject.await.expect("inject");

    let stdin = capture.lock().expect("stdin");
    let text = String::from_utf8_lossy(&stdin);
    assert!(text.contains("AGENTHUB induction"));
    assert!(text.contains("Please acknowledge"));
}

#[test]
fn permissions_for_role_case_insensitive_lookup() {
    let state = ServerState::new();
    let perms = permissions_for_role(&state.roles, "builder").expect("builder");
    assert!(perms.contains(Permissions::WRITE_FILES));
}
