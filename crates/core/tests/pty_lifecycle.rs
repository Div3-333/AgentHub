// Phase 2 DoD: spawn mock_cli → write stdin → read ring buffer → kill → no zombies.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use agenthub_core::bus::{OfflineReason, BUS_CAPACITY};
use agenthub_core::config::{AgentHubConfig, DriverProfile};
use agenthub_core::pty::manager::spawn_test_pty_agent;
use agenthub_core::pty::{kill_agent, pty_tests_enabled, spawn_agent, PtyStatus, SpawnOptions};
use agenthub_core::server::ServerState;
use tokio::sync::broadcast;

/// `--test` target name when re-invoking this crate's test binary (core vs workspace integration).
fn pty_test_target_name() -> &'static str {
    if let Ok(name) = std::env::var("AGENTHUB_PTY_TEST_TARGET") {
        if !name.is_empty() {
            return Box::leak(name.into_boxed_str());
        }
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem()?.to_str().map(str::to_owned))
        .filter(|stem| stem.contains("integration_pty"))
        .map(|_| "integration_pty_lifecycle")
        .unwrap_or("pty_lifecycle")
}

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

fn drain_ring(agent: &agenthub_core::pty::AgentPty) -> Vec<u8> {
    let rb = agent.ring_buffer();
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = rb.read(&mut buf);
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    out
}

async fn wait_for_substring(
    agent: &agenthub_core::pty::AgentPty,
    needle: &[u8],
    timeout: Duration,
) -> Vec<u8> {
    let deadline = Instant::now() + timeout;
    let mut accumulated = Vec::new();
    while Instant::now() < deadline {
        accumulated.extend(drain_ring(agent));
        if needle.is_empty() || accumulated.windows(needle.len()).any(|w| w == needle) {
            return accumulated;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!(
        "timed out waiting for {:?}; got {:?}",
        String::from_utf8_lossy(needle),
        String::from_utf8_lossy(&accumulated)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pty_lifecycle_spawn_write_read_kill() {
    if !pty_tests_enabled() {
        eprintln!("skip: AGENTHUB_SKIP_PTY is set");
        return;
    }

    tokio::time::timeout(Duration::from_secs(45), async {
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
        let (bus_tx, _bus_rx) = broadcast::channel(BUS_CAPACITY);

        let id = spawn_agent(
            "mock_cli",
            &config,
            Arc::clone(&state),
            bus_tx.clone(),
            None,
            SpawnOptions {
                skip_sanitizer: true,
                skip_induction: true,
                ..SpawnOptions::default()
            },
        )
        .await
        .expect("spawn_agent");

        let agent = state
            .agents
            .get(&id)
            .expect("agent registered")
            .value()
            .clone();

        let _initial = wait_for_substring(&agent, b">", Duration::from_secs(15)).await;

        let line_end = if cfg!(windows) {
            b"hello\r\n".as_slice()
        } else {
            b"hello\n".as_slice()
        };
        agent.write_stdin(line_end).expect("write stdin");

        let output = wait_for_substring(&agent, b"Mock response.", Duration::from_secs(15)).await;
        assert!(
            output
                .windows(b"Mock response.".len())
                .any(|w| w == b"Mock response."),
            "mock CLI response missing from ring buffer"
        );

        let pid = agent.pid;
        kill_agent(&state, id, &bus_tx, OfflineReason::Kicked).expect("kill_agent");
        assert!(!state.agents.contains_key(&id));

        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_no_zombie(pid);
    })
    .await
    .expect("pty_lifecycle_spawn_write_read_kill timed out after 45s");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn agent_pty_drop_reaps_child() {
    if !pty_tests_enabled() {
        eprintln!("skip: AGENTHUB_SKIP_PTY is set");
        return;
    }

    tokio::time::timeout(Duration::from_secs(45), async {
        let dir = tempfile::tempdir().expect("tempdir");
        let drivers_dir = dir.path().join("drivers");
        std::fs::create_dir_all(&drivers_dir).expect("mkdir drivers");

        let (executable, args): (String, Vec<String>) = if cfg!(windows) {
            (
                "cmd".to_string(),
                vec![
                    "/c".to_string(),
                    "ping".to_string(),
                    "127.0.0.1".to_string(),
                    "-n".to_string(),
                    "30".to_string(),
                ],
            )
        } else {
            ("sleep".to_string(), vec!["30".to_string()])
        };
        let profile = DriverProfile {
            name: "slow_echo".to_string(),
            display_name: "Slow".to_string(),
            executable,
            args,
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
        let json = serde_json::to_string_pretty(&profile).expect("json");
        std::fs::write(drivers_dir.join("slow_echo.json"), json).expect("write driver");

        let agent = spawn_test_pty_agent("slow_echo", &drivers_dir, "slow_echo-1")
            .expect("spawn_test_pty_agent");
        let pid = agent.pid;
        assert_ne!(
            agent.status.load(Ordering::Relaxed),
            PtyStatus::Dead.as_u8()
        );
        drop(agent);

        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_no_zombie(pid);
    })
    .await
    .expect("agent_pty_drop_reaps_child timed out after 45s");
}

/// Child entry: spawn mock_cli, register global shutdown, `exit(0)` without dropping state.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn orphan_annihilation_child_process() {
    if std::env::var("AGENTHUB_ORPHAN_CHILD").is_err() {
        return;
    }

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
    let (bus_tx, _bus_rx) = broadcast::channel(BUS_CAPACITY);
    agenthub_core::server::install_global_shutdown(Arc::clone(&state), bus_tx.clone());

    let id = spawn_agent(
        "mock_cli",
        &config,
        Arc::clone(&state),
        bus_tx,
        None,
        SpawnOptions {
            skip_sanitizer: true,
            skip_induction: true,
            ..SpawnOptions::default()
        },
    )
    .await
    .expect("spawn mock_cli");

    let pid = state.agents.get(&id).expect("agent").pid;
    if let Ok(path) = std::env::var("AGENTHUB_ORPHAN_PID_FILE") {
        let _ = std::fs::write(path, pid.to_string());
    }

    // Abrupt exit: atexit handler must kill the mock_cli child.
    std::process::exit(0);
}

#[test]
fn orphan_annihilation_abrupt_parent_exit() {
    if std::env::var("AGENTHUB_ORPHAN_CHILD").is_ok() {
        return;
    }

    let pid_file = std::env::temp_dir().join(format!(
        "agenthub_orphan_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let exe = std::env::current_exe().expect("current exe");
    let mock_cli = mock_cli_executable();
    let mut cmd = std::process::Command::new(&exe);
    cmd.env("AGENTHUB_SKIP_PTY", "1")
        .env("AGENTHUB_ORPHAN_CHILD", "1")
        .env("AGENTHUB_ORPHAN_PID_FILE", &pid_file)
        .env(
            "CARGO_BIN_EXE_mock_cli",
            mock_cli.to_string_lossy().as_ref(),
        );
    if let Ok(target_dir) = std::env::var("CARGO_TARGET_DIR") {
        cmd.env("CARGO_TARGET_DIR", target_dir);
    }
    let status = cmd
        .args([
            "--test",
            pty_test_target_name(),
            "orphan_annihilation_child_process",
            "--exact",
            "--nocapture",
        ])
        .status()
        .expect("spawn orphan child test");

    assert!(
        pid_file.is_file(),
        "orphan child should write pid file before exit"
    );
    let pid: u32 = std::fs::read_to_string(&pid_file)
        .expect("read pid")
        .trim()
        .parse()
        .expect("parse pid");
    let _ = std::fs::remove_file(&pid_file);

    assert_eq!(
        status.code(),
        Some(0),
        "child calls process::exit(0) after registering shutdown hook"
    );
    std::thread::sleep(Duration::from_millis(400));
    assert_no_zombie(pid);
}

fn assert_no_zombie(pid: u32) {
    #[cfg(unix)]
    {
        let mut status: i32 = 0;
        let waited = unsafe { libc::waitpid(pid as i32, &mut status, libc::WNOHANG) };
        assert!(
            waited != 0 || unsafe { libc::kill(pid as i32, 0) } == -1,
            "child pid {pid} still alive or unreaped (zombie)"
        );
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, FALSE};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };

        const STILL_ACTIVE: u32 = 259;

        let deadline = Instant::now() + Duration::from_secs(3);
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
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("child pid {pid} still running after teardown (STILL_ACTIVE)");
    }
    #[cfg(all(not(unix), not(windows)))]
    {
        let _ = pid;
    }
}
