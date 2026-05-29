//! AgentHub binary — production bootstrap (config, DB, bus, TUI).
//!
//! Startup sequence lives in [`agenthub::bootstrap`]:
//! load `~/.agenthub/config.json`, SQLite session, [`ServerState`] + RBAC roles,
//! bus router, then [`agenthub_tui::run_with_bridge`] (no demo data, no auto-spawn).

fn main() {
    if let Err(err) = agenthub::bootstrap::run() {
        eprintln!("agenthub: {err:#}");
        std::process::exit(1);
    }
}
