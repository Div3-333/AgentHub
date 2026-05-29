//! End-to-end smoke: config → DB → bus router → mock_cli agent → UserMessage → AgentMessage → clean shutdown.
//! Uses piped stdio when `AGENTHUB_SKIP_PTY=1` (CI-safe).

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use agenthub_core::bus::{spawn_bus_router, BusEvent, MessageTarget, OfflineReason};
use agenthub_core::config::{load_driver_profile_from_dir, AgentHubConfig, DriverProfile};
use agenthub_core::db::{DbClient, NewSession};
use agenthub_core::pipeline::pty_bridge::spawn_agent_message_bridge;
use agenthub_core::pty::{kill_agent, spawn_agent, PtyStatus, SpawnOptions};
use agenthub_core::server::modes::{set_mode, WorkspaceModeId};
use agenthub_core::server::ServerState;
use chrono::Utc;
use tokio::sync::broadcast;
use uuid::Uuid;

include!("../../crates/core/tests/common/mock_cli_exe.rs");

fn write_mock_driver(drivers_dir: &Path, executable: &Path) {
    let profile = DriverProfile {
        name: "mock_cli".to_string(),
        display_name: "Mock CLI".to_string(),
        executable: executable.to_string_lossy().into_owned(),
        args: vec![],
        env: HashMap::from([
            ("NO_COLOR".to_string(), "1".to_string()),
            ("TERM".to_string(), "dumb".to_string()),
            ("MOCK_CLI_LATENCY_MS".to_string(), "10".to_string()),
        ]),
        prompt_regex: "^>\\s*$".to_string(),
        silence_timeout_ms: 300,
        init_sequence: vec![],
        rate_limit_patterns: vec![],
        auto_reply_patterns: HashMap::new(),
        supports_multi_instance: true,
        max_instances: 0,
    };
    let json = serde_json::to_string_pretty(&profile).expect("serialize driver");
    std::fs::write(drivers_dir.join("mock_cli.json"), json).expect("write driver json");
}

fn sqlite_url(path: &Path) -> String {
    let normalized = path.display().to_string().replace('\\', "/");
    if normalized.starts_with('/') || normalized.contains(":/") {
        format!("sqlite:///{normalized}")
    } else {
        format!("sqlite://{normalized}")
    }
}

async fn drain_bus_until<F>(
    bus_rx: &mut broadcast::Receiver<BusEvent>,
    timeout: Duration,
    mut pred: F,
) -> bool
where
    F: FnMut(&BusEvent) -> bool,
{
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(50), bus_rx.recv()).await {
            Ok(Ok(event)) if pred(&event) => return true,
            Ok(Ok(_)) => {}
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(_)) => break,
            Err(_) => {}
        }
    }
    false
}

async fn drain_bus_for(bus_rx: &mut broadcast::Receiver<BusEvent>, duration: Duration) {
    let _ = drain_bus_until(bus_rx, duration, |_| false).await;
}

fn agent_message_body_matches(content: &str) -> bool {
    content.to_ascii_lowercase().contains("mock response")
}

fn assert_process_gone(pid: u32) {
    #[cfg(unix)]
    {
        let mut status: i32 = 0;
        let waited = unsafe { libc::waitpid(pid as i32, &mut status, libc::WNOHANG) };
        assert!(
            waited != 0 || unsafe { libc::kill(pid as i32, 0) } == -1,
            "child pid {pid} still alive or unreaped"
        );
    }
    #[cfg(windows)]
    {
        use std::time::Instant as StdInstant;
        use windows_sys::Win32::Foundation::{CloseHandle, FALSE};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };

        const STILL_ACTIVE: u32 = 259;
        let deadline = StdInstant::now() + Duration::from_secs(3);
        loop {
            let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid) };
            if handle == 0 {
                return;
            }
            let mut code = 0u32;
            let ok = unsafe { GetExitCodeProcess(handle, &mut code) };
            unsafe { CloseHandle(handle) };
            assert_ne!(ok, 0, "GetExitCodeProcess failed for pid {pid}");
            if code != STILL_ACTIVE {
                return;
            }
            if StdInstant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("child pid {pid} still running after kill");
    }
    #[cfg(all(not(unix), not(windows)))]
    {
        let _ = pid;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn smoke_config_db_bus_mock_cli_roundtrip() {
    let prev_skip = std::env::var("AGENTHUB_SKIP_PTY").ok();
    std::env::set_var("AGENTHUB_SKIP_PTY", "1");

    let result = tokio::time::timeout(Duration::from_secs(45), async {
        let config_dir = tempfile::tempdir().expect("tempdir");
        std::env::set_var("AGENTHUB_CONFIG_DIR", config_dir.path());

        let drivers_dir = config_dir.path().join("drivers");
        std::fs::create_dir_all(&drivers_dir).expect("mkdir drivers");
        write_mock_driver(&drivers_dir, &mock_cli_executable());

        let mut config = AgentHubConfig::default();
        config.drivers_dir = drivers_dir;
        config.db_path = config_dir.path().join("agenthub.db");
        config.max_agents = 4;
        config
            .save_to(&config_dir.path().join("config.json"))
            .expect("save config");

        let config = Arc::new(AgentHubConfig::load().expect("load config"));
        let driver =
            load_driver_profile_from_dir(&config.drivers_dir, "mock_cli").expect("mock driver");

        let db = Arc::new(
            DbClient::init_pool(&sqlite_url(&config.db_path))
                .await
                .expect("db pool"),
        );
        db.run_migrations().await.expect("migrations");

        let session_id = Uuid::new_v4();
        db.insert_session(&NewSession {
            id: session_id,
            mode: "group_chat".to_string(),
            cwd: std::env::current_dir()
                .unwrap_or_default()
                .display()
                .to_string(),
        })
        .await
        .expect("insert session");

        let state = Arc::new(ServerState::new());
        set_mode(&state, WorkspaceModeId::GroupChat).expect("workspace mode");

        let channels = spawn_bus_router(Arc::clone(&state), None, session_id);
        let bus_tx = channels.bus_tx.clone();
        let mut bus_rx = bus_tx.subscribe();
        let mut tui_rx = channels.tui_rx;
        tokio::spawn(async move { while tui_rx.recv().await.is_some() {} });

        let agent_id = spawn_agent(
            "mock_cli",
            &config,
            Arc::clone(&state),
            bus_tx.clone(),
            Some(Arc::clone(&db)),
            SpawnOptions {
                tag: Some("mock-1".into()),
                skip_sanitizer: true,
                skip_induction: true,
                ..SpawnOptions::default()
            },
        )
        .await
        .expect("spawn_agent");

        let agent = state
            .agents
            .get(&agent_id)
            .expect("agent registered")
            .clone();
        let pid = agent.pid;
        assert_ne!(pid, 0, "mock agent should have a real child pid");

        let _message_bridge =
            spawn_agent_message_bridge(Arc::clone(&agent), driver, bus_tx.clone());

        tokio::time::sleep(Duration::from_millis(400)).await;
        drain_bus_for(&mut bus_rx, Duration::from_millis(200)).await;
        drop(bus_rx);
        let mut bus_rx = bus_tx.subscribe();

        let _ = bus_tx.send(BusEvent::UserMessage {
            content: "@mock-1 smoke ping".into(),
            timestamp: Utc::now(),
            target: MessageTarget::Broadcast,
        });

        let inject_deadline = Instant::now() + Duration::from_secs(3);
        while agent.status() != Some(PtyStatus::Thinking) && Instant::now() < inject_deadline {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert_eq!(
            agent.status(),
            Some(PtyStatus::Thinking),
            "bus router did not inject UserMessage into mock_cli stdin"
        );

        tokio::time::sleep(Duration::from_millis(500)).await;

        let mut seen_agent_messages: Vec<String> = Vec::new();
        let got_response = drain_bus_until(&mut bus_rx, Duration::from_secs(15), |e| {
            if let BusEvent::AgentMessage {
                id, tag, content, ..
            } = e
            {
                seen_agent_messages.push(format!("{id} @{tag}: {content:?}"));
                return *id == agent_id && agent_message_body_matches(content);
            }
            false
        })
        .await;
        assert!(
            got_response,
            "timed out waiting for AgentMessage from mock_cli; saw: {seen_agent_messages:?}"
        );

        kill_agent(&state, agent_id, &bus_tx, OfflineReason::Kicked).expect("kill_agent");
        assert!(!state.agents.contains_key(&agent_id));

        assert!(
            drain_bus_until(&mut bus_rx, Duration::from_secs(2), |e| {
                matches!(
                    e,
                    BusEvent::AgentOffline { id, reason, .. }
                        if *id == agent_id && *reason == OfflineReason::Kicked
                )
            })
            .await,
            "expected AgentOffline after kill"
        );

        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_process_gone(pid);

        std::env::remove_var("AGENTHUB_CONFIG_DIR");
    })
    .await;

    if let Some(v) = prev_skip {
        std::env::set_var("AGENTHUB_SKIP_PTY", v);
    } else {
        std::env::remove_var("AGENTHUB_SKIP_PTY");
    }

    result.expect("smoke_config_db_bus_mock_cli_roundtrip timed out after 45s");
}
