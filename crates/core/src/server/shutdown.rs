//! Global shutdown: SIGINT/SIGTERM (Unix) and console ctrl events (Windows).
//!
//! Ensures every [`crate::pty::AgentPty`] is torn down on signal or abrupt `exit()`.

use std::sync::{Arc, OnceLock};

use tokio::sync::broadcast;
use uuid::Uuid;

use crate::bus::{BusEvent, OfflineReason};
use crate::pty::kill_agent;
use crate::server::ServerState;

struct ShutdownState {
    state: Arc<ServerState>,
    bus_tx: broadcast::Sender<BusEvent>,
}

static SHUTDOWN: OnceLock<Arc<ShutdownState>> = OnceLock::new();

/// Terminate every registered agent and emit offline bus events.
pub fn kill_all_agents(state: &ServerState, bus_tx: &broadcast::Sender<BusEvent>) {
    let ids: Vec<Uuid> = state.agents.iter().map(|entry| *entry.key()).collect();
    for id in ids {
        if let Err(err) = kill_agent(state, id, bus_tx, OfflineReason::Kicked) {
            tracing::debug!(%err, %id, "agent already removed during shutdown");
        }
    }
}

fn run_shutdown() {
    if let Some(handle) = SHUTDOWN.get() {
        kill_all_agents(&handle.state, &handle.bus_tx);
    }
}

#[cfg(unix)]
extern "C" fn atexit_shutdown() {
    run_shutdown();
}

#[cfg(windows)]
fn install_windows_console_handler() {
    use std::sync::Once;
    use windows_sys::Win32::Foundation::{BOOL, FALSE, TRUE};
    use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;

    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        unsafe extern "system" fn handler(ctrl_type: u32) -> BOOL {
            const CTRL_C_EVENT: u32 = 0;
            const CTRL_BREAK_EVENT: u32 = 1;
            const CTRL_CLOSE_EVENT: u32 = 2;
            const CTRL_SHUTDOWN_EVENT: u32 = 6;

            match ctrl_type {
                CTRL_C_EVENT | CTRL_BREAK_EVENT | CTRL_CLOSE_EVENT | CTRL_SHUTDOWN_EVENT => {
                    run_shutdown();
                    TRUE
                }
                _ => FALSE,
            }
        }
        let _ = SetConsoleCtrlHandler(Some(handler), TRUE);
    });
}

/// Register handlers so agent children are killed on Ctrl+C, SIGTERM, or `exit()`.
pub fn install_global_shutdown(state: Arc<ServerState>, bus_tx: broadcast::Sender<BusEvent>) {
    let _ = SHUTDOWN.set(Arc::new(ShutdownState { state, bus_tx }));

    let _ = ctrlc::set_handler(|| {
        run_shutdown();
        std::process::exit(0);
    });

    #[cfg(windows)]
    install_windows_console_handler();

    #[cfg(unix)]
    unsafe {
        libc::atexit(atexit_shutdown);
    }
}
