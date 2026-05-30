//! Ratatui front-end for AgentHub.
//!
//! - [`run`] — standalone demo UI (sample agents/chat).
//! - [`run_with_bridge`] — live session: pass [`CoreBridge`] and `tui_rx` from
//!   [`agenthub_core::bus::spawn_bus_router`].

pub mod app;
pub mod components;
pub mod events;
pub mod theme;

pub use app::{CoreBridge, WorkspaceMode};
pub use components::racing;

use agenthub_core::bus::BusEvent;
use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use tokio::sync::{broadcast, mpsc};

use app::App;

/// Standalone preview UI (sample chat/agents). The `agenthub` binary uses [`run_with_bridge`].
pub fn run() -> Result<()> {
    run_terminal(App::new_demo("dark"), None)
}

/// Runs the TUI wired to a live core: processes bus events and sends user input on `bus_tx`.
///
/// Pass `tui_rx` from [`agenthub_core::bus::spawn_bus_router`] (processed events after racing
/// tagging). For tests or minimal wiring without the router task, use
/// [`run_with_bridge_broadcast`] with `bus_tx.subscribe()`.
pub fn run_with_bridge(
    bridge: CoreBridge,
    bus_rx: mpsc::UnboundedReceiver<BusEvent>,
    theme: &str,
    workspace_mode: app::WorkspaceMode,
) -> Result<()> {
    let mut app = App::new_live(theme);
    app.workspace_mode = workspace_mode;
    app.set_core_bridge(bridge);
    run_terminal(app, Some(BusInbox::Mpsc(bus_rx)))
}

/// Same as [`run_with_bridge`] but consumes the broadcast fan-out directly (unprocessed stream).
pub fn run_with_bridge_broadcast(
    bridge: CoreBridge,
    bus_rx: broadcast::Receiver<BusEvent>,
) -> Result<()> {
    let mut app = App::new_live("dark");
    app.set_core_bridge(bridge);
    run_terminal(app, Some(BusInbox::Broadcast(bus_rx)))
}

enum BusInbox {
    Mpsc(mpsc::UnboundedReceiver<BusEvent>),
    Broadcast(broadcast::Receiver<BusEvent>),
}

impl BusInbox {
    fn try_recv_event(&mut self) -> Option<BusEvent> {
        match self {
            Self::Mpsc(rx) => rx.try_recv().ok(),
            Self::Broadcast(rx) => rx.try_recv().ok(),
        }
    }

    fn drain(&mut self, app: &mut App) {
        while let Some(ev) = self.try_recv_event() {
            app.on_bus_event(ev);
        }
    }
}

fn run_terminal(mut app: App, mut bus: Option<BusInbox>) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    if let Ok(size) = terminal.size() {
        app.update_terminal_size(size.width, size.height);
    }
    let result = run_app(&mut terminal, &mut app, &mut bus);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    bus: &mut Option<BusInbox>,
) -> Result<()> {
    loop {
        if let Some(inbox) = bus.as_mut() {
            inbox.drain(app);
        }

        terminal.draw(|f| app.draw(f))?;

        if event::poll(std::time::Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if events::handle_key(app, key) || app.should_quit {
                        return Ok(());
                    }
                }
                Event::Resize(cols, rows) => app.update_terminal_size(cols, rows),
                Event::Mouse(m)
                    if m.kind
                        == MouseEventKind::Down(crossterm::event::MouseButton::Left) =>
                {
                    events::handle_mouse_click(app, m.column, m.row);
                }
                _ => {}
            }
        }
    }
}
