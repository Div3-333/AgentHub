//! Smoke test: production bootstrap loads config, DB, and bus without hanging.

use std::sync::mpsc;
use std::time::Duration;

use agenthub::AgentHubStack;
use tempfile::TempDir;

#[test]
fn app_bootstrap_smoke_config_db_bus_without_hanging() {
    let config_dir = TempDir::new().expect("tempdir");
    std::env::set_var("AGENTHUB_CONFIG_DIR", config_dir.path());

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(AgentHubStack::boot());
    });

    let stack = rx
        .recv_timeout(Duration::from_secs(15))
        .expect("boot must finish within 15s")
        .expect("boot must succeed");

    assert!(stack.config.max_agents > 0);
    assert!(
        stack.config.db_path.exists(),
        "bootstrap must create db path"
    );
    assert_eq!(
        stack.state.agents.len(),
        0,
        "bootstrap must not auto-spawn agents"
    );
    assert!(
        stack.bus_tx.receiver_count() >= 1,
        "bus router must register at least one subscriber"
    );

    stack.shutdown();
    std::env::remove_var("AGENTHUB_CONFIG_DIR");
}
