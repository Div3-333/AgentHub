//! Stream sanitizer: virtual terminal grid + turn-detection heuristics (blueprint §6).

pub mod heuristic;
pub mod parser;

use std::sync::Arc;

use tokio::sync::broadcast;

use crate::bus::BusEvent;
use crate::config::DriverProfile;
use crate::pty::manager::AgentPty;
use crate::server::ServerState;

pub use heuristic::{sanitizer_task, AutoReplies, TurnDetector, CONFIRMATION_MS, SILENCE_POLL_MS};
pub use parser::{last_non_empty_line, VirtualGrid, GRID_COLS, GRID_ROWS};

/// Spawns [`sanitizer_task`] alongside [`crate::pty::io::pty_reader_task`] at agent spawn.
pub fn spawn_sanitizer_task(
    agent: Arc<AgentPty>,
    driver: DriverProfile,
    state: Arc<ServerState>,
    bus_tx: broadcast::Sender<BusEvent>,
) {
    tokio::spawn(sanitizer_task(agent, driver, state, bus_tx));
}
