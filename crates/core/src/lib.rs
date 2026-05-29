pub mod error;

#[cfg(feature = "full")]
pub mod pipeline;

#[cfg(any(
    feature = "full",
    feature = "vfs-tests",
    feature = "db-tests",
    feature = "bus-tests"
))]
pub mod bus;
#[cfg(any(
    feature = "full",
    feature = "vfs-tests",
    feature = "db-tests",
    feature = "config-tests",
    feature = "bus-tests"
))]
pub mod config;
#[cfg(any(
    feature = "full",
    feature = "vfs-tests",
    feature = "db-tests",
    feature = "bus-tests"
))]
pub mod db;
#[cfg(any(feature = "full", feature = "vfs-tests", feature = "bus-tests"))]
pub mod vfs;

#[cfg(any(feature = "full", feature = "bus-tests"))]
pub mod pty;
#[cfg(feature = "full")]
pub mod sanitizer;
#[cfg(any(feature = "full", feature = "server-tests", feature = "bus-tests"))]
pub mod server;

pub mod context;

pub use error::{AgentHubError, GraphError, Result};

#[cfg(any(
    feature = "full",
    feature = "vfs-tests",
    feature = "db-tests",
    feature = "config-tests"
))]
pub use config::AgentHubConfig;

#[cfg(any(feature = "full", feature = "config-tests"))]
pub use config::{DriverProfile, WorkspaceMode};

#[cfg(test)]
mod tests {
    use super::AgentHubError;

    #[test]
    fn agenthub_error_displays() {
        let err = AgentHubError::Pty("session lost".into());
        let msg = err.to_string();
        assert!(msg.contains("PTY"));
        assert!(msg.contains("session lost"));
    }
}
