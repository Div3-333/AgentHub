//! Production stack bootstrap: config, SQLite, server state, bus router, TUI wiring.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use agenthub_core::bus::{spawn_bus_router, BusEvent};
use agenthub_core::config::{AgentHubConfig, WorkspaceMode};
use agenthub_core::context::AstIndexer;
use agenthub_core::db::{DbClient, NewSession};
use agenthub_core::server::moderation::ModerationContext;
use agenthub_core::server::modes::{set_mode, WorkspaceModeId};
use agenthub_core::server::{install_global_shutdown, kill_all_agents, ServerState};
use agenthub_core::Result;
use agenthub_tui::{CoreBridge, WorkspaceMode as TuiWorkspaceMode};
use parking_lot::RwLock;
use tokio::runtime::Runtime;
use tokio::sync::broadcast;
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

struct BootInner {
    config: Arc<AgentHubConfig>,
    state: Arc<ServerState>,
    db: Arc<DbClient>,
    session_id: Uuid,
    cwd: PathBuf,
    bus_tx: broadcast::Sender<BusEvent>,
    tui_rx: tokio::sync::mpsc::UnboundedReceiver<BusEvent>,
}

/// Live session handles torn down on normal exit or panic unwind.
pub struct AgentHubStack {
    pub config: Arc<AgentHubConfig>,
    pub state: Arc<ServerState>,
    pub db: Arc<DbClient>,
    pub session_id: Uuid,
    pub cwd: PathBuf,
    pub bus_tx: broadcast::Sender<BusEvent>,
    pub tui_rx: tokio::sync::mpsc::UnboundedReceiver<BusEvent>,
    runtime: Runtime,
}

impl AgentHubStack {
    /// Load config, init DB, server state, and bus router (no agents spawned).
    pub fn boot() -> Result<Self> {
        let config = Arc::new(AgentHubConfig::load()?);
        init_tracing(&config.log_level);

        let runtime = Runtime::new()
            .map_err(|e| agenthub_core::AgentHubError::Config(format!("tokio runtime: {e}")))?;

        let inner = runtime.block_on(Self::boot_async(config))?;
        Ok(AgentHubStack {
            config: inner.config,
            state: inner.state,
            db: inner.db,
            session_id: inner.session_id,
            cwd: inner.cwd,
            bus_tx: inner.bus_tx,
            tui_rx: inner.tui_rx,
            runtime,
        })
    }

    async fn boot_async(config: Arc<AgentHubConfig>) -> Result<BootInner> {
        let cwd = std::env::current_dir().map_err(agenthub_core::AgentHubError::from)?;

        if let Some(parent) = config.db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let db_url = sqlite_url(&config.db_path);
        let db = Arc::new(DbClient::init_pool(&db_url).await?);
        db.run_migrations().await?;

        if config.pty_debug_log {
            match agenthub_core::pty::rotate_pty_debug(db.as_ref()).await {
                Ok((db_deleted, files_deleted)) => {
                    tracing::info!(
                        db_deleted,
                        files_deleted,
                        "rotated PTY debug log (48h retention)"
                    );
                }
                Err(e) => tracing::warn!(%e, "PTY debug log rotation failed"),
            }
        }

        let session_id = Uuid::new_v4();
        let mode_slug = workspace_mode_slug(config.default_mode.clone());
        db.insert_session(&NewSession {
            id: session_id,
            mode: mode_slug.to_string(),
            cwd: cwd.display().to_string(),
        })
        .await?;

        let state = Arc::new(ServerState::new());
        let mode_id = WorkspaceModeId::from_config(config.default_mode.clone());
        set_mode(&state, mode_id)?;

        // First run: config.json is created by `AgentHubConfig::load()`; we do not auto-spawn
        // any driver PTYs — the user starts agents via `/spawn <driver>` or F5 in the TUI.

        let context_index = Arc::new(RwLock::new(AstIndexer::new(&cwd)));
        {
            let index = Arc::clone(&context_index);
            std::thread::spawn(move || {
                if let Err(e) = index.write().index_all() {
                    tracing::warn!(%e, "initial AST index failed");
                }
            });
        }

        let channels = spawn_bus_router(
            Arc::clone(&state),
            Some(Arc::clone(&db)),
            session_id,
            cwd.clone(),
            Arc::clone(&config),
            context_index,
        );
        install_global_shutdown(Arc::clone(&state), channels.bus_tx.clone());

        Ok(BootInner {
            config,
            state,
            db,
            session_id,
            cwd,
            bus_tx: channels.bus_tx,
            tui_rx: channels.tui_rx,
        })
    }

    pub fn moderation_context(&self) -> ModerationContext {
        ModerationContext {
            state: Arc::clone(&self.state),
            config: Arc::clone(&self.config),
            db: Some(Arc::clone(&self.db)),
            bus_tx: self.bus_tx.clone(),
            session_id: self.session_id,
            cwd: self.cwd.clone(),
            issued_by: "user".to_string(),
            caller_agent_id: None,
        }
    }

    pub fn core_bridge(&self) -> CoreBridge {
        CoreBridge {
            moderation: Arc::new(self.moderation_context()),
            db: Arc::clone(&self.db),
            config: Arc::clone(&self.config),
            cwd: self.cwd.clone(),
            session_id: self.session_id,
            bus_tx: self.bus_tx.clone(),
        }
    }

    pub fn tui_workspace_mode(&self) -> TuiWorkspaceMode {
        tui_mode_from_config(self.config.default_mode.clone())
    }

    /// Run the TUI; always shuts down agents and ends the DB session afterward.
    pub fn run_tui(&mut self) -> anyhow::Result<()> {
        let bridge = self.core_bridge();
        let bus_rx = std::mem::replace(&mut self.tui_rx, tokio::sync::mpsc::unbounded_channel().1);
        let theme = self.config.theme.clone();
        let workspace_mode = self.tui_workspace_mode();

        let tui_result = catch_unwind(AssertUnwindSafe(|| {
            agenthub_tui::run_with_bridge(bridge, bus_rx, &theme, workspace_mode)
        }));

        self.shutdown();

        match tui_result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    /// Kill all agent PTYs, allow teardown to finish, mark session ended.
    pub fn shutdown(&self) {
        kill_all_agents(&self.state, &self.bus_tx);
        wait_agents_drained(&self.state, Duration::from_secs(2));
        if let Err(e) = self.runtime.block_on(self.db.end_session(self.session_id)) {
            tracing::warn!(%e, "failed to end session in database");
        }
    }
}

/// Poll until every PTY entry is removed from [`ServerState::agents`] (or timeout).
fn wait_agents_drained(state: &ServerState, timeout: Duration) {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if state.agents.is_empty() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    if !state.agents.is_empty() {
        tracing::warn!(
            remaining = state.agents.len(),
            "timeout waiting for agent PTY teardown"
        );
    }
}

/// Entry: boot stack, run TUI, guaranteed cleanup.
pub fn run() -> anyhow::Result<()> {
    let mut stack = AgentHubStack::boot().map_err(anyhow::Error::from)?;
    stack.run_tui()
}

fn init_tracing(log_level: &str) {
    use std::fs::OpenOptions;
    use std::sync::Mutex;

    use agenthub_core::config::agenthub_home;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level));
    let log_path = agenthub_home().join("agenthub.log");
    if let Ok(file) = OpenOptions::new().create(true).append(true).open(&log_path) {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(false)
            .with_ansi(false)
            .with_writer(Mutex::new(file))
            .try_init();
        return;
    }
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}

fn sqlite_url(path: &Path) -> String {
    let normalized = path.display().to_string().replace('\\', "/");
    if normalized.starts_with('/') || normalized.contains(":/") {
        format!("sqlite:///{normalized}")
    } else {
        format!("sqlite://{normalized}")
    }
}

fn workspace_mode_slug(mode: WorkspaceMode) -> &'static str {
    match mode {
        WorkspaceMode::DirectMessage => "direct_message",
        WorkspaceMode::GroupChat => "group_chat",
        WorkspaceMode::Server => "server",
    }
}

fn tui_mode_from_config(mode: WorkspaceMode) -> TuiWorkspaceMode {
    match mode {
        WorkspaceMode::DirectMessage => TuiWorkspaceMode::DirectMessage,
        WorkspaceMode::GroupChat => TuiWorkspaceMode::GroupChat,
        WorkspaceMode::Server => TuiWorkspaceMode::Server,
    }
}
