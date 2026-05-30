//! Root application state and layout (Part 15.1).

use std::path::PathBuf;
use std::sync::Arc;

use agenthub_core::bus::{
    is_racing_input, normalize_racing_tag, BusEvent, MessageTarget, WorkspaceModeRepr,
};
use agenthub_core::config::AgentHubConfig;
use agenthub_core::db::DbClient;
use agenthub_core::server::moderation::ModerationContext;
use chrono::{DateTime, Utc};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::{
    components::{chat, input, pipeline_viz, racing, sidebar},
    theme::{self, Theme},
};

fn racing_tags_match(left: &str, right: &str) -> bool {
    normalize_racing_tag(left) == normalize_racing_tag(right)
}

/// Optional hook to the running AgentHub core (moderation + VFS slash commands).
#[derive(Clone)]
pub struct CoreBridge {
    pub moderation: Arc<ModerationContext>,
    pub db: Arc<DbClient>,
    pub config: Arc<AgentHubConfig>,
    pub cwd: PathBuf,
    pub session_id: Uuid,
    pub bus_tx: broadcast::Sender<BusEvent>,
}

pub const FOOTER_HINT: &str =
    "F1:Help  F4:Scroll  F7:Copy  Wheel:Scroll  Ctrl+/:Search  F5:Spawn  Esc:Cancel";

/// F1 overlay and `/help` share [`crate::events::TUI_HELP`].
pub use crate::events::TUI_HELP as HELP_TEXT;

/// Workspace mode (mirrors `agenthub_core::WorkspaceMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceMode {
    DirectMessage,
    GroupChat,
    Server,
}

impl WorkspaceMode {
    pub fn from_repr(mode: WorkspaceModeRepr) -> Self {
        match mode {
            WorkspaceModeRepr::Dm => Self::DirectMessage,
            WorkspaceModeRepr::GroupChat => Self::GroupChat,
            WorkspaceModeRepr::Server => Self::Server,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::DirectMessage => "DM",
            Self::GroupChat => "GroupChat",
            Self::Server => "Server",
        }
    }

    pub fn cycle(self) -> Self {
        match self {
            Self::DirectMessage => Self::GroupChat,
            Self::GroupChat => Self::Server,
            Self::Server => Self::DirectMessage,
        }
    }

    /// Argument for `/mode` (blueprint §8.3).
    pub fn mode_slug(self) -> &'static str {
        match self {
            Self::DirectMessage => "dm",
            Self::GroupChat => "groupchat",
            Self::Server => "server",
        }
    }
}

/// Agent status indicators (blueprint §15.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStatus {
    Initializing,
    Idle,
    Thinking,
    Muted,
    Deafened,
    Suspended,
    Dead,
    RateLimited,
}

#[derive(Debug, Clone)]
pub struct AgentEntry {
    pub id: Option<Uuid>,
    pub tag: String,
    pub role: String,
    pub status: AgentStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatSender {
    User,
    Agent,
    System,
}

#[derive(Debug, Clone)]
pub struct ChatLine {
    pub time_label: String,
    pub text: String,
    pub sender: ChatSender,
}

/// Pipeline execution state driven by bus events (see `pipeline_viz`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineRunStatus {
    Running,
    Complete,
    Failed,
}

#[derive(Debug, Clone)]
pub struct PipelineInfo {
    /// Short label per stage (`@tag` or first token of a unix command).
    pub stage_labels: Vec<String>,
    /// 0-based index of the stage currently executing.
    pub active_index: usize,
    pub progress: u8,
    pub output_preview: String,
    pub status: PipelineRunStatus,
}

#[derive(Debug, Clone)]
pub struct RacingPane {
    pub tag: String,
    pub role: String,
    pub content: String,
    pub done: bool,
    pub elapsed_secs: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    #[default]
    Input,
    Chat,
}

/// Pane hit-tested for mouse focus (Part 15.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneTarget {
    Chat,
    Input,
}

/// High-level UI state machine (Part 15.1–15.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppState {
    Normal,
    Search,
    Help,
    QuitConfirm,
    RevertConfirm,
    Racing,
    SpawnDialog,
    AgentList,
    SavePath,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    None,
    Help,
    QuitConfirm,
    RevertConfirm,
    Racing,
    SpawnDialog,
    AgentList,
    SavePath,
}

/// Two-step revert confirmation (overwrite, then optional delete-new-files).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevertDialogStep {
    ConfirmRevert,
    ConfirmDeleteNewFiles,
}

#[derive(Debug, Clone)]
pub struct RevertDialogState {
    pub preview: agenthub_core::vfs::RevertPreview,
    pub step: RevertDialogStep,
}

/// Which pane receives arrow-key style navigation.
pub struct App {
    pub theme: Theme,
    pub workspace_mode: WorkspaceMode,
    pub chat_lines: Vec<ChatLine>,
    pub chat_scroll: usize,
    pub input_buffer: String,
    pub input_cursor: usize,
    pub input_history: Vec<String>,
    pub input_history_idx: Option<usize>,
    pub agents: Vec<AgentEntry>,
    pub snapshot_count: usize,
    pub pipeline: Option<PipelineInfo>,
    pub racing_panes: Vec<RacingPane>,
    pub racing_selected: usize,
    pub racing_started_at: Option<std::time::Instant>,
    pub race_session_id: Option<Uuid>,
    pub focus: Focus,
    pub overlay: Overlay,
    pub search_mode: bool,
    pub search_query: String,
    pub save_path_buffer: String,
    pub spawn_buffer: String,
    pub agent_list_selected: usize,
    pub spar_active: bool,
    pub should_quit: bool,
    pub status_message: String,
    pub revert_dialog: Option<RevertDialogState>,
    pub term_cols: u16,
    pub term_rows: u16,
    /// When set, `/commands` route to [`agenthub_core::server::moderation`].
    pub core: Option<CoreBridge>,
}

impl App {
    #[must_use]
    pub fn state(&self) -> AppState {
        if self.overlay == Overlay::Racing {
            return AppState::Racing;
        }
        if self.search_mode {
            return AppState::Search;
        }
        match self.overlay {
            Overlay::Help => AppState::Help,
            Overlay::QuitConfirm => AppState::QuitConfirm,
            Overlay::RevertConfirm => AppState::RevertConfirm,
            Overlay::SpawnDialog => AppState::SpawnDialog,
            Overlay::AgentList => AppState::AgentList,
            Overlay::SavePath => AppState::SavePath,
            Overlay::None | Overlay::Racing => AppState::Normal,
        }
    }

    /// Standalone demo UI with sample chat, agents, and pipeline preview.
    pub fn new_demo(theme_name: &str) -> Self {
        Self::new_inner(theme_name, true)
    }

    /// Empty shell for live core wiring ([`crate::run_with_bridge`]).
    pub fn new_live(theme_name: &str) -> Self {
        Self::new_inner(theme_name, false)
    }

    /// Defaults to demo data; use [`Self::new_live`] when attaching a core bridge.
    pub fn new(theme_name: &str) -> Self {
        Self::new_demo(theme_name)
    }

    fn new_inner(theme_name: &str, demo: bool) -> Self {
        let (chat_lines, agents, snapshot_count, pipeline) = if demo {
            (
                sample_chat(),
                sample_agents(),
                2,
                Some(pipeline_viz::demo_pipeline_info()),
            )
        } else {
            let version = env!("CARGO_PKG_VERSION");
            let welcome = format!(
                "AgentHub v{version} ready.\n\
                 Quickstart: /spawn gemini | claude | codex | aider   F1: help   F3: snapshot   Ctrl+Z: undo\n\
                 F7: copy session to clipboard (+ ~/.agenthub/chat_export.txt)   F4 / wheel: scroll chat\n\
                 Set \"spawn_debug\": true in ~/.agenthub/config.json for live PTY I/O in chat.\n\
                 Modes: F2 cycles DM → GroupChat → Server (/channel in Server mode)."
            );
            let welcome = vec![ChatLine {
                time_label: time_label_now(),
                text: welcome,
                sender: ChatSender::System,
            }];
            (welcome, Vec::<AgentEntry>::new(), 0, None)
        };

        Self {
            theme: theme::from_name(theme_name),
            workspace_mode: WorkspaceMode::GroupChat,
            chat_lines,
            chat_scroll: 0,
            input_buffer: String::new(),
            input_cursor: 0,
            input_history: Vec::new(),
            input_history_idx: None,
            agents,
            snapshot_count,
            pipeline,
            racing_panes: Vec::new(),
            racing_selected: 0,
            racing_started_at: None,
            race_session_id: None,
            focus: Focus::Input,
            overlay: Overlay::None,
            search_mode: false,
            search_query: String::new(),
            save_path_buffer: String::from("chat_export.txt"),
            spawn_buffer: String::new(),
            agent_list_selected: 0,
            spar_active: false,
            should_quit: false,
            status_message: String::new(),
            revert_dialog: None,
            term_cols: 120,
            term_rows: 40,
            core: None,
        }
    }

    /// Attach the live core so slash commands hit moderation / VFS handlers.
    pub fn set_core_bridge(&mut self, bridge: CoreBridge) {
        self.core = Some(bridge);
    }

    pub fn update_terminal_size(&mut self, cols: u16, rows: u16) {
        self.term_cols = cols.max(1);
        self.term_rows = rows.max(1);
    }

    fn layout_areas(&self) -> LayoutAreas {
        let area = Rect::new(0, 0, self.term_cols, self.term_rows);
        compute_layout(area, self.workspace_mode)
    }

    pub fn chat_viewport_rows(&self) -> usize {
        let areas = self.layout_areas();
        self.chat_visible_rows(areas.chat.height)
    }

    /// Inner width of the chat pane (matches [`components::chat::render`]).
    pub fn chat_inner_width(&self) -> usize {
        let areas = self.layout_areas();
        areas.chat.width.saturating_sub(2) as usize
    }

    fn chat_rendered_row_count(&self) -> usize {
        crate::components::chat::total_rendered_rows(&self.chat_lines, self.chat_inner_width())
    }

    /// Cursor index into the string shown in the input/search widget.
    pub fn input_display_cursor(&self) -> usize {
        if self.search_mode {
            "search: ".len() + self.search_query.len()
        } else {
            2 + self.input_cursor
        }
    }

    /// Map terminal cell to chat or input pane (mouse focus).
    pub fn hit_test(&self, col: u16, row: u16) -> Option<PaneTarget> {
        use ratatui::layout::Position;

        let pos = Position::new(col, row);
        let areas = self.layout_areas();
        if areas.input.contains(pos) {
            return Some(PaneTarget::Input);
        }
        if areas.chat.contains(pos) {
            return Some(PaneTarget::Chat);
        }
        None
    }

    pub fn agent_tags(&self) -> Vec<String> {
        self.agents
            .iter()
            .filter(|a| a.status != AgentStatus::Dead)
            .map(|a| a.tag.clone())
            .collect()
    }

    pub fn chat_visible_rows(&self, height: u16) -> usize {
        height.saturating_sub(2).max(1) as usize
    }

    pub fn max_chat_scroll(&self, visible_rows: usize) -> usize {
        self.chat_rendered_row_count()
            .saturating_sub(visible_rows.max(1))
    }

    pub fn scroll_chat_down(&mut self, amount: usize, visible_rows: usize) {
        let max = self.max_chat_scroll(visible_rows);
        self.chat_scroll = self.chat_scroll.saturating_add(amount).min(max);
    }

    pub fn scroll_chat_up(&mut self, amount: usize) {
        self.chat_scroll = self.chat_scroll.saturating_sub(amount);
    }

    pub fn scroll_chat_to_bottom(&mut self, visible_rows: usize) {
        self.chat_scroll = self.max_chat_scroll(visible_rows);
    }

    pub fn push_chat(&mut self, sender: ChatSender, text: impl Into<String>) {
        self.chat_lines.push(ChatLine {
            time_label: time_label_now(),
            text: text.into(),
            sender,
        });
        self.chat_scroll = usize::MAX;
    }

    /// Apply a bus event from the core message router (blueprint §7.2).
    pub fn on_bus_event(&mut self, event: BusEvent) {
        match event {
            BusEvent::UserMessage {
                content, timestamp, ..
            } => self.push_bus_chat(ChatSender::User, "User", content, timestamp),
            BusEvent::AgentMessage {
                tag,
                content,
                timestamp,
                race_session_id: None,
                ..
            } => self.push_bus_chat(ChatSender::Agent, &tag, content, timestamp),
            BusEvent::AgentMessage {
                race_session_id: Some(_),
                ..
            } => {}
            BusEvent::SystemMessage { content, timestamp } => {
                self.push_bus_chat(ChatSender::System, "system", content, timestamp)
            }
            BusEvent::AgentOnline { id, tag, role } => {
                if let Some(a) = self
                    .agents
                    .iter_mut()
                    .find(|a| a.id == Some(id) || a.tag == tag)
                {
                    a.id = Some(id);
                    a.role = role;
                    a.status = AgentStatus::Idle;
                } else {
                    self.agents.push(AgentEntry {
                        id: Some(id),
                        tag: tag.clone(),
                        role,
                        status: AgentStatus::Idle,
                    });
                }
                self.push_chat(ChatSender::System, format!("{tag} is online"));
            }
            BusEvent::AgentSpawnStarted {
                id,
                tag,
                driver,
                role,
                command_line,
            } => {
                if let Some(a) = self.agents.iter_mut().find(|a| a.tag == tag) {
                    a.id = Some(id);
                    a.role = role;
                    a.status = AgentStatus::Initializing;
                } else {
                    self.agents.push(AgentEntry {
                        id: Some(id),
                        tag: tag.clone(),
                        role,
                        status: AgentStatus::Initializing,
                    });
                }
                self.push_chat(
                    ChatSender::System,
                    format!("[Spawn] @{tag} ({driver}): `{command_line}`"),
                );
            }
            BusEvent::SpawnTrace { tag, message, .. } => {
                self.push_chat(ChatSender::System, format!("[spawn @{tag}] {message}"));
            }
            BusEvent::PtyIoTrace {
                tag,
                direction,
                preview,
                ..
            } => {
                self.push_chat(
                    ChatSender::System,
                    format!("[pty @{tag} {direction}] {preview}"),
                );
            }
            BusEvent::AgentOffline { tag, .. } => {
                if let Some(a) = self.agents.iter_mut().find(|a| a.tag == tag) {
                    a.status = AgentStatus::Dead;
                }
                self.push_chat(ChatSender::System, format!("{tag} went offline"));
            }
            BusEvent::AgentStatusChanged { id, new, .. } => {
                if let Some(a) = self.agents.iter_mut().find(|a| a.id == Some(id)) {
                    a.status = decode_agent_status(new);
                }
            }
            BusEvent::SubagentDetected {
                child_id,
                child_tag,
                ..
            } => {
                self.agents.push(AgentEntry {
                    id: Some(child_id),
                    tag: child_tag.clone(),
                    role: "Subagent".into(),
                    status: AgentStatus::Idle,
                });
                self.push_chat(
                    ChatSender::System,
                    format!("Subagent {child_tag} registered"),
                );
            }
            BusEvent::AgentMuted { id, .. } => {
                if let Some(a) = self.agents.iter_mut().find(|a| a.id == Some(id)) {
                    a.status = AgentStatus::Muted;
                }
            }
            BusEvent::AgentUnmuted { id, .. } => {
                if let Some(a) = self.agents.iter_mut().find(|a| a.id == Some(id)) {
                    a.status = AgentStatus::Idle;
                }
            }
            BusEvent::AgentDeafened { id, .. } => {
                if let Some(a) = self.agents.iter_mut().find(|a| a.id == Some(id)) {
                    a.status = AgentStatus::Deafened;
                }
            }
            BusEvent::AgentUndeafened { id, .. } => {
                if let Some(a) = self.agents.iter_mut().find(|a| a.id == Some(id)) {
                    if a.status == AgentStatus::Deafened {
                        a.status = AgentStatus::Idle;
                    }
                }
            }
            BusEvent::AgentTimedOut { id, .. } => {
                if let Some(a) = self.agents.iter_mut().find(|a| a.id == Some(id)) {
                    a.status = AgentStatus::Suspended;
                }
            }
            BusEvent::AgentKicked { id, .. } | BusEvent::AgentBanned { id, .. } => {
                if let Some(a) = self.agents.iter_mut().find(|a| a.id == Some(id)) {
                    a.status = AgentStatus::Dead;
                }
            }
            BusEvent::RoleAssigned { agent_id, role, .. } => {
                if let Some(a) = self.agents.iter_mut().find(|a| a.id == Some(agent_id)) {
                    a.role = role;
                }
            }
            BusEvent::RacingStarted {
                tags,
                prompt,
                session_id,
                ..
            } => {
                self.race_session_id = Some(session_id);
                let mention = tags
                    .iter()
                    .map(|t| format!("@{t}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                self.activate_racing(&format!("{mention} {prompt}"));
            }
            BusEvent::RacingOutput { tag, chunk, .. } => {
                self.append_racing_output(&tag, &chunk);
            }
            BusEvent::RacingAgentComplete {
                tag, elapsed_ms, ..
            } => {
                if let Some(pane) = self
                    .racing_panes
                    .iter_mut()
                    .find(|p| racing_tags_match(&p.tag, &tag))
                {
                    pane.done = true;
                    pane.elapsed_secs = Some(elapsed_ms as f64 / 1000.0);
                }
            }
            BusEvent::RacingComplete { .. } => {
                self.status_message = "All done — ←/→ select  Enter confirm  Esc discard".into();
            }
            BusEvent::RacingCancelled { .. } => {
                self.dismiss_racing_ui();
            }
            BusEvent::RevertInitiated { .. } => {
                self.status_message = "Reverting to snapshot…".into();
            }
            BusEvent::RevertComplete { .. } => {
                self.status_message = "Revert complete.".into();
            }
            BusEvent::ModeChanged { new, .. } => {
                self.workspace_mode = WorkspaceMode::from_repr(new);
            }
            BusEvent::SnapshotCreated { .. } => {
                self.snapshot_count = self.snapshot_count.saturating_add(1);
            }
            BusEvent::PipelineStarted { definition, .. } => {
                self.pipeline = Some(pipeline_viz::pipeline_from_started(&definition));
            }
            BusEvent::PipelineStageComplete {
                stage,
                output_preview,
                ..
            } => {
                if let Some(p) = &mut self.pipeline {
                    pipeline_viz::on_stage_complete(p, stage, &output_preview);
                }
            }
            BusEvent::PipelineComplete { .. } => {
                if let Some(p) = &mut self.pipeline {
                    pipeline_viz::on_pipeline_complete(p);
                }
            }
            BusEvent::PipelineFailed { stage, error, .. } => {
                if let Some(p) = &mut self.pipeline {
                    pipeline_viz::on_pipeline_failed(p, stage);
                }
                self.push_chat(ChatSender::System, format!("Pipeline failed: {error}"));
            }
            BusEvent::RateLimitDetected { tag, .. } => {
                if let Some(a) = self.agents.iter_mut().find(|a| a.tag == tag) {
                    a.status = AgentStatus::RateLimited;
                }
            }
        }
    }

    fn push_bus_chat(
        &mut self,
        sender: ChatSender,
        name: &str,
        content: String,
        timestamp: DateTime<Utc>,
    ) {
        let text = match sender {
            ChatSender::User => format!("{name}: {content}"),
            ChatSender::Agent => format!("@{name}: {content}"),
            ChatSender::System => content,
        };
        self.chat_lines.push(ChatLine {
            time_label: timestamp.format("%H:%M:%S").to_string(),
            text,
            sender,
        });
        self.chat_scroll = usize::MAX;
    }

    pub fn on_submit(&mut self, text: String) {
        if text.is_empty() {
            return;
        }
        self.input_history.push(text.clone());
        self.input_history_idx = None;

        if text.starts_with('/') {
            crate::events::route_slash_command(self, &text);
            return;
        }

        if let Some(bridge) = self.core.clone() {
            let _ = bridge.bus_tx.send(BusEvent::UserMessage {
                content: text,
                timestamp: Utc::now(),
                target: MessageTarget::Broadcast,
            });
            return;
        }

        self.push_chat(ChatSender::User, text.clone());
        if is_racing_input(&text) {
            self.activate_racing(&text);
        }
    }

    /// Show a system line and optional footer status from command output.
    pub fn apply_command_result(&mut self, message: String) {
        self.status_message = message.clone();
        self.push_chat(ChatSender::System, message);
    }

    pub fn activate_racing(&mut self, prompt: &str) {
        let tags: Vec<String> = prompt
            .split_whitespace()
            .filter_map(|w| {
                w.strip_prefix('@').map(|t| {
                    t.trim_end_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
                })
            })
            .filter(|t| !t.is_empty())
            .map(|t| t.to_string())
            .collect();

        self.racing_started_at = Some(std::time::Instant::now());
        if tags.len() < 2 {
            self.status_message = "Racing requires at least two @tags before the prompt.".into();
            return;
        }

        self.racing_panes = tags
            .iter()
            .map(|tag| {
                let role = self
                    .agents
                    .iter()
                    .find(|a| racing_tags_match(&a.tag, tag))
                    .map(|a| a.role.clone())
                    .unwrap_or_else(|| "Agent".into());
                RacingPane {
                    tag: tag.clone(),
                    role,
                    content: String::new(),
                    done: false,
                    elapsed_secs: None,
                }
            })
            .collect();
        self.racing_selected = 0;
        self.overlay = Overlay::Racing;
        self.focus = Focus::Chat;
        self.status_message = "←/→ select  Enter confirm  Esc discard".into();
    }

    pub fn racing_select_prev(&mut self) {
        if self.racing_panes.is_empty() {
            return;
        }
        if self.racing_selected == 0 {
            self.racing_selected = self.racing_panes.len() - 1;
        } else {
            self.racing_selected -= 1;
        }
    }

    pub fn racing_select_next(&mut self) {
        if self.racing_panes.is_empty() {
            return;
        }
        self.racing_selected = (self.racing_selected + 1) % self.racing_panes.len();
    }

    pub fn append_racing_output(&mut self, tag: &str, chunk: &str) {
        if let Some(pane) = self
            .racing_panes
            .iter_mut()
            .find(|p| racing_tags_match(&p.tag, tag))
        {
            if chunk.len() >= pane.content.len() && chunk.starts_with(&pane.content) {
                pane.content.push_str(&chunk[pane.content.len()..]);
            } else if pane.content.is_empty() || chunk.len() > pane.content.len() {
                pane.content = chunk.to_string();
            } else {
                pane.content.push_str(chunk);
            }
        }
    }

    pub fn mark_racing_done(&mut self, tag: &str) {
        let elapsed = self
            .racing_started_at
            .map(|s| s.elapsed().as_secs_f64())
            .unwrap_or(0.0);
        if let Some(pane) = self
            .racing_panes
            .iter_mut()
            .find(|p| racing_tags_match(&p.tag, tag))
        {
            pane.done = true;
            pane.elapsed_secs = Some(elapsed);
        }
        if racing::all_done(&self.racing_panes) {
            self.status_message = "All done — ←/→ select  Enter confirm  Esc discard".into();
        }
    }

    pub fn confirm_racing_winner(&mut self) {
        if self.racing_panes.is_empty() {
            return;
        }
        let idx = self.racing_selected.min(self.racing_panes.len() - 1);
        let winner = self.racing_panes[idx].clone();
        self.push_chat(
            ChatSender::Agent,
            format!("@{}: {}", winner.tag, winner.content),
        );
        let archives: Vec<String> = self
            .racing_panes
            .iter()
            .enumerate()
            .filter(|(i, pane)| *i != idx && !pane.content.is_empty())
            .map(|(_, pane)| {
                format!(
                    "Racing archive @{} ({:.1}s, {} chars)",
                    pane.tag,
                    pane.elapsed_secs.unwrap_or(0.0),
                    pane.content.len()
                )
            })
            .collect();
        for line in archives {
            self.push_chat(ChatSender::System, line);
        }
        self.dismiss_racing_ui();
        self.status_message = format!("Selected @{} as winner", winner.tag);
    }

    pub fn cancel_racing(&mut self) {
        if let (Some(bridge), Some(rid)) = (&self.core, self.race_session_id) {
            let _ = bridge.bus_tx.send(BusEvent::RacingCancelled {
                session_id: rid,
                timestamp: Utc::now(),
            });
        }
        self.dismiss_racing_ui();
    }

    pub fn dismiss_racing_ui(&mut self) {
        self.racing_panes.clear();
        self.racing_started_at = None;
        self.race_session_id = None;
        self.overlay = Overlay::None;
        self.focus = Focus::Input;
        self.status_message = "Racing discarded".into();
    }

    pub fn cancel_overlay(&mut self) {
        if self.overlay == Overlay::Racing {
            self.cancel_racing();
            return;
        }
        self.overlay = Overlay::None;
        self.revert_dialog = None;
        self.search_mode = false;
        self.search_query.clear();
        self.spawn_buffer.clear();
        self.agent_list_selected = 0;
        self.save_path_buffer = "chat_export.txt".into();
        if self.spar_active {
            self.spar_active = false;
            self.status_message = "Spar session cancelled".into();
        }
        if self.pipeline.take().is_some() {
            self.status_message = "Pipeline view cleared (Esc)".into();
        }
    }

    pub fn agent_list_entries(&self) -> Vec<&AgentEntry> {
        self.agents
            .iter()
            .filter(|a| a.status != AgentStatus::Dead)
            .collect()
    }

    pub fn agent_list_body(&self) -> String {
        let entries = self.agent_list_entries();
        if entries.is_empty() {
            return "No active agents.\nEsc to close.".into();
        }
        let mut lines = vec![
            "j/k or ↑/↓ select   m mute   K kick   p promote   d demote   Esc close".into(),
            String::new(),
        ];
        for (idx, agent) in entries.iter().enumerate() {
            let marker = if idx == self.agent_list_selected {
                "►"
            } else {
                " "
            };
            lines.push(format!(
                "{marker} {} [{}] {} {}",
                agent.tag,
                agent.role,
                sidebar::status_glyph(agent.status),
                sidebar::status_label(agent.status)
            ));
        }
        lines.join("\n")
    }

    pub fn save_chat_to_path(&self, path: &str) -> std::io::Result<usize> {
        let body = self.session_plain_text();
        std::fs::write(path, &body)?;
        Ok(body.len())
    }

    /// Plain-text dump of agents + chat + status (for copy/paste and exports).
    #[must_use]
    pub fn session_plain_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "AgentHub v{} — {} mode\n",
            env!("CARGO_PKG_VERSION"),
            self.workspace_mode.label()
        ));
        out.push_str(&format!("Agents online: {}\n", self.agents.len()));
        out.push_str("\n--- AGENTS ---\n");
        if self.agents.is_empty() {
            out.push_str("(none)\n");
        } else {
            for agent in &self.agents {
                out.push_str(&format!(
                    "@{} [{}] {} {}\n",
                    agent.tag,
                    agent.role,
                    sidebar::status_glyph(agent.status),
                    sidebar::status_label(agent.status)
                ));
            }
        }
        if !self.status_message.is_empty() {
            out.push_str(&format!("\n--- STATUS ---\n{}\n", self.status_message));
        }
        out.push_str("\n--- CHAT ---\n");
        for line in &self.chat_lines {
            let prefix = match line.sender {
                ChatSender::User => "User",
                ChatSender::Agent => "Agent",
                ChatSender::System => "System",
            };
            out.push_str(&format!(
                "[{}] {}: {}\n",
                line.time_label, prefix, line.text
            ));
        }
        out
    }

    /// Copy full session text to the OS clipboard; also writes `~/.agenthub/chat_export.txt`.
    pub fn copy_session_to_clipboard(&self) -> Result<usize, String> {
        let text = self.session_plain_text();
        crate::clipboard::copy_to_clipboard(&text)?;
        let path = agenthub_core::config::agenthub_home().join("chat_export.txt");
        std::fs::write(&path, &text)
            .map_err(|e| format!("wrote clipboard but failed to save {}: {e}", path.display()))?;
        Ok(text.len())
    }

    pub fn draw(&mut self, f: &mut Frame) {
        let size = f.size();
        if self.overlay == Overlay::Racing {
            self.draw_racing(f, size);
            return;
        }

        let areas = compute_layout(size, self.workspace_mode);
        match self.workspace_mode {
            WorkspaceMode::DirectMessage => self.draw_dm(f, &areas),
            _ => self.draw_group(f, &areas),
        }

        match self.overlay {
            Overlay::Help => render_help_overlay(f, size, self.theme),
            Overlay::QuitConfirm => {}
            Overlay::RevertConfirm => render_dialog(
                f,
                size,
                self.theme,
                " Revert Workspace ",
                &self.status_message,
            ),
            Overlay::SpawnDialog => render_dialog(
                f,
                size,
                self.theme,
                " Spawn Agent (F5) ",
                &format!(
                    "Driver/tag: {}\nEnter to confirm, Esc to cancel",
                    self.spawn_buffer
                ),
            ),
            Overlay::AgentList => render_dialog(
                f,
                size,
                self.theme,
                " Agents (F6) ",
                &self.agent_list_body(),
            ),
            Overlay::SavePath => render_dialog(
                f,
                size,
                self.theme,
                " Save Chat (Ctrl+S) ",
                &format!(
                    "Path: {}\nEnter to save, Esc to cancel",
                    self.save_path_buffer
                ),
            ),
            Overlay::None | Overlay::Racing => {}
        }
    }

    fn draw_group(&self, f: &mut Frame, areas: &LayoutAreas) {
        chat::render(
            f,
            areas.chat,
            &self.chat_lines,
            self.chat_scroll,
            if self.search_mode {
                Some(self.search_query.as_str())
            } else {
                None
            },
            &self.theme,
        );
        sidebar::render(
            f,
            areas.sidebar,
            &self.agents,
            self.workspace_mode,
            self.snapshot_count,
            self.pipeline.as_ref(),
            &self.theme,
        );
        let input_text = if self.search_mode {
            format!("search: {}", self.search_query)
        } else {
            format!("> {}", self.input_buffer)
        };
        input::render(
            f,
            areas.input,
            &input_text,
            self.input_display_cursor(),
            self.search_mode,
            self.focus == Focus::Chat,
            &self.theme,
        );
        let footer = if self.overlay == Overlay::QuitConfirm {
            "Ctrl+Q again to quit (agents killed), Esc to cancel"
        } else if self.overlay == Overlay::RevertConfirm {
            "Y: confirm   N / Esc: cancel"
        } else if !self.status_message.is_empty() {
            self.status_message.as_str()
        } else {
            FOOTER_HINT
        };
        render_footer(f, areas.footer, self.theme, footer);
    }

    fn draw_dm(&self, f: &mut Frame, areas: &LayoutAreas) {
        let agent = self.agents.first();
        let header = if let Some(a) = agent {
            format!(
                "DM: {} [{}] {} {}",
                a.tag,
                a.role,
                sidebar::status_glyph(a.status),
                sidebar::status_label(a.status)
            )
        } else {
            "DM: (no agent)".into()
        };
        let header_block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.border_style())
            .title(" Direct Message ");
        f.render_widget(
            Paragraph::new(header).block(header_block),
            areas.dm_header.unwrap_or_default(),
        );
        chat::render(
            f,
            areas.chat,
            &self.chat_lines,
            self.chat_scroll,
            if self.search_mode {
                Some(self.search_query.as_str())
            } else {
                None
            },
            &self.theme,
        );
        let input_text = if self.search_mode {
            format!("search: {}", self.search_query)
        } else {
            format!("> {}", self.input_buffer)
        };
        input::render(
            f,
            areas.input,
            &input_text,
            self.input_display_cursor(),
            self.search_mode,
            self.focus == Focus::Chat,
            &self.theme,
        );
        let footer = if self.overlay == Overlay::QuitConfirm {
            "Ctrl+Q again to quit, Esc to cancel"
        } else if self.overlay == Overlay::RevertConfirm {
            "Y: confirm   N / Esc: cancel"
        } else {
            FOOTER_HINT
        };
        render_footer(f, areas.footer, self.theme, footer);
    }

    fn draw_racing(&self, f: &mut Frame, area: Rect) {
        let footer_h = 1u16.min(area.height);
        let main_h = area.height.saturating_sub(footer_h);
        let chunks = Layout::vertical([Constraint::Length(main_h), Constraint::Length(footer_h)])
            .split(area);
        racing::render(
            f,
            chunks[0],
            &self.racing_panes,
            self.racing_selected,
            self.racing_started_at,
            &self.theme,
        );
        render_footer(
            f,
            chunks[1],
            self.theme,
            racing::footer_hint(racing::all_done(&self.racing_panes)),
        );
    }
}

struct LayoutAreas {
    chat: Rect,
    sidebar: Rect,
    input: Rect,
    footer: Rect,
    dm_header: Option<Rect>,
}

/// Resize-safe layout for terminals from 80×24 through 220×50.
fn compute_layout(area: Rect, mode: WorkspaceMode) -> LayoutAreas {
    let w = area.width.max(1);
    let h = area.height.max(1);

    let footer_h = if h >= 4 { 1 } else { 0 };
    let input_h = if h >= 6 {
        3.min(h.saturating_sub(footer_h + 1))
    } else {
        1.min(h)
    };
    let body_h = h.saturating_sub(input_h + footer_h).max(1);

    let root = Layout::vertical([
        Constraint::Length(body_h),
        Constraint::Length(input_h),
        Constraint::Length(footer_h),
    ])
    .split(area);

    if mode == WorkspaceMode::DirectMessage {
        let dm_header_h = if body_h >= 4 { 2.min(body_h - 1) } else { 0 };
        let chat_h = body_h.saturating_sub(dm_header_h).max(1);
        let dm_chunks =
            Layout::vertical([Constraint::Length(dm_header_h), Constraint::Length(chat_h)])
                .split(root[0]);
        return LayoutAreas {
            chat: dm_chunks[1],
            sidebar: Rect::default(),
            input: root[1],
            footer: root[2],
            dm_header: if dm_header_h > 0 {
                Some(dm_chunks[0])
            } else {
                None
            },
        };
    }

    let sidebar_w = if w >= 100 {
        28.min(w / 3).max(18)
    } else if w >= 80 {
        20.min(w.saturating_sub(40)).max(12)
    } else {
        0
    };

    if sidebar_w == 0 || w < 80 {
        return LayoutAreas {
            chat: root[0],
            sidebar: Rect::default(),
            input: root[1],
            footer: root[2],
            dm_header: None,
        };
    }

    let chat_w = w.saturating_sub(sidebar_w).max(1);
    let horiz = Layout::horizontal([Constraint::Length(chat_w), Constraint::Length(sidebar_w)])
        .split(root[0]);

    LayoutAreas {
        chat: horiz[0],
        sidebar: horiz[1],
        input: root[1],
        footer: root[2],
        dm_header: None,
    }
}

fn render_footer(f: &mut Frame, area: Rect, theme: Theme, hint: &str) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let paragraph = Paragraph::new(Line::from(Span::styled(hint, theme.footer_style())));
    f.render_widget(paragraph, area);
}

fn render_help_overlay(f: &mut Frame, area: Rect, theme: Theme) {
    let overlay_area = centered_rect(70, 80, area);
    if overlay_area.width == 0 || overlay_area.height == 0 {
        return;
    }
    f.render_widget(Clear, overlay_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border_style())
        .title(Span::styled(" HELP (F1 to close) ", theme.title_style()));
    let paragraph = Paragraph::new(HELP_TEXT)
        .block(block)
        .wrap(Wrap { trim: true })
        .style(Style::default().fg(theme.fg).bg(theme.help_overlay_bg));
    f.render_widget(paragraph, overlay_area);
}

fn render_dialog(f: &mut Frame, area: Rect, theme: Theme, title: &str, body: &str) {
    let overlay_area = centered_rect(60, 40, area);
    if overlay_area.width == 0 || overlay_area.height == 0 {
        return;
    }
    f.render_widget(Clear, overlay_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border_style())
        .title(Span::styled(title, theme.title_style()));
    f.render_widget(
        Paragraph::new(body).block(block).wrap(Wrap { trim: true }),
        overlay_area,
    );
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let px = percent_x.min(100);
    let py = percent_y.min(100);
    let vertical = Layout::vertical([
        Constraint::Percentage((100 - py) / 2),
        Constraint::Percentage(py),
        Constraint::Percentage((100 - py) / 2),
    ])
    .split(area);

    Layout::horizontal([
        Constraint::Percentage((100 - px) / 2),
        Constraint::Percentage(px),
        Constraint::Percentage((100 - px) / 2),
    ])
    .split(vertical[1])[1]
}

fn decode_agent_status(code: u8) -> AgentStatus {
    match code {
        0 => AgentStatus::Initializing,
        1 => AgentStatus::Idle,
        2 => AgentStatus::Thinking,
        3 => AgentStatus::Muted,
        4 => AgentStatus::Deafened,
        5 => AgentStatus::Suspended,
        6 => AgentStatus::Dead,
        7 => AgentStatus::RateLimited,
        _ => AgentStatus::Thinking,
    }
}

fn time_label_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() % 86_400)
        .unwrap_or(0);
    format!(
        "{:02}:{:02}:{:02}",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

fn sample_chat() -> Vec<ChatLine> {
    vec![
        ChatLine {
            time_label: "12:04:01".into(),
            text: "User: @gemini write an auth module".into(),
            sender: ChatSender::User,
        },
        ChatLine {
            time_label: "12:04:03".into(),
            text: "@gemini-1: Here is the auth module...".into(),
            sender: ChatSender::Agent,
        },
        ChatLine {
            time_label: "12:04:45".into(),
            text: "@claude-1: I notice the authenticate function lacks...".into(),
            sender: ChatSender::Agent,
        },
    ]
}

fn sample_agents() -> Vec<AgentEntry> {
    vec![
        AgentEntry {
            id: None,
            tag: "gemini-1".into(),
            role: "Builder".into(),
            status: AgentStatus::Thinking,
        },
        AgentEntry {
            id: None,
            tag: "claude-1".into(),
            role: "Reviewer".into(),
            status: AgentStatus::Idle,
        },
        AgentEntry {
            id: None,
            tag: "aider-1".into(),
            role: "Builder".into(),
            status: AgentStatus::Muted,
        },
        AgentEntry {
            id: None,
            tag: "codex-1".into(),
            role: "Builder".into(),
            status: AgentStatus::Deafened,
        },
        AgentEntry {
            id: None,
            tag: "mock-1".into(),
            role: "Subagent".into(),
            status: AgentStatus::Suspended,
        },
        AgentEntry {
            id: None,
            tag: "old-1".into(),
            role: "Builder".into(),
            status: AgentStatus::Dead,
        },
        AgentEntry {
            id: None,
            tag: "api-1".into(),
            role: "Builder".into(),
            status: AgentStatus::RateLimited,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn draw_all_sizes_80x24_to_220x50_no_panic() {
        let sizes: &[(u16, u16)] = &[
            (80, 24),
            (80, 50),
            (100, 30),
            (120, 40),
            (160, 35),
            (220, 24),
            (220, 50),
        ];
        for &(w, h) in sizes {
            let backend = TestBackend::new(w, h);
            let mut terminal = Terminal::new(backend).expect("terminal");
            let mut app = App::new("dark");
            for mode in [
                WorkspaceMode::GroupChat,
                WorkspaceMode::Server,
                WorkspaceMode::DirectMessage,
            ] {
                app.workspace_mode = mode;
                app.overlay = Overlay::None;
                terminal
                    .draw(|f| app.draw(f))
                    .expect("draw group/server/dm");
            }
            app.overlay = Overlay::Help;
            terminal.draw(|f| app.draw(f)).expect("draw help");
            app.overlay = Overlay::Racing;
            app.activate_racing("@gemini-1 @claude-1 test");
            terminal.draw(|f| app.draw(f)).expect("draw racing");
            app.overlay = Overlay::AgentList;
            terminal.draw(|f| app.draw(f)).expect("draw agent list");
        }
    }

    #[test]
    fn save_chat_writes_file() {
        let dir = std::env::temp_dir().join(format!("agenthub_tui_test_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("export.txt");
        let app = App::new("dark");
        let bytes = app
            .save_chat_to_path(path.to_str().expect("utf8 path"))
            .expect("write");
        assert!(bytes > 0);
        let read = std::fs::read_to_string(&path).expect("read");
        assert!(read.contains("12:04:01"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn racing_stream_and_done_updates_panes() {
        let mut app = App::new("dark");
        app.activate_racing("@gemini-1 @claude-1 race");
        app.append_racing_output("gemini-1", "fn foo() {}");
        app.mark_racing_done("gemini-1");
        assert!(app.racing_panes[0].done);
        assert!(app.racing_panes[0].content.contains("foo"));
    }

    #[test]
    fn sidebar_status_glyphs_match_blueprint() {
        assert_eq!(sidebar::status_glyph(AgentStatus::Idle), "●");
        assert_eq!(sidebar::status_glyph(AgentStatus::Thinking), "⏳");
        assert_eq!(sidebar::status_glyph(AgentStatus::Muted), "🔇");
        assert_eq!(sidebar::status_glyph(AgentStatus::Deafened), "🔕");
        assert_eq!(sidebar::status_glyph(AgentStatus::Suspended), "⏸");
        assert_eq!(sidebar::status_glyph(AgentStatus::Dead), "💀");
        assert_eq!(sidebar::status_glyph(AgentStatus::RateLimited), "⚠");
    }
}
