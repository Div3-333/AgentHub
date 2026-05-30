//! TUI color palette (blueprint §15).

use crate::app::AgentStatus;
use ratatui::style::{Color, Modifier, Style};

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub border: Color,
    pub title: Color,
    pub fg: Color,
    pub user_msg: Color,
    pub agent_msg: Color,
    pub system_msg: Color,
    pub input: Color,
    pub cursor: Color,
    pub status_bar: Color,
    pub sidebar_online: Color,
    pub sidebar_muted: Color,
    pub sidebar_deafened: Color,
    pub sidebar_thinking: Color,
    pub sidebar_warning: Color,
    pub sidebar_dead: Color,
    pub search_highlight: Color,
    pub help_overlay_bg: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}

pub fn from_name(name: &str) -> Theme {
    match name {
        "light" => Theme::light(),
        _ => Theme::dark(),
    }
}

impl Theme {
    pub fn dark() -> Self {
        Self {
            border: Color::Rgb(60, 60, 80),
            title: Color::Cyan,
            fg: Color::White,
            user_msg: Color::White,
            agent_msg: Color::Green,
            system_msg: Color::DarkGray,
            input: Color::Yellow,
            cursor: Color::Rgb(255, 255, 128),
            status_bar: Color::DarkGray,
            sidebar_online: Color::Green,
            sidebar_muted: Color::DarkGray,
            sidebar_deafened: Color::Blue,
            sidebar_thinking: Color::Yellow,
            sidebar_warning: Color::Rgb(255, 165, 0),
            sidebar_dead: Color::Red,
            search_highlight: Color::Rgb(255, 215, 0),
            help_overlay_bg: Color::Rgb(20, 20, 30),
        }
    }

    pub fn light() -> Self {
        Self {
            border: Color::Rgb(180, 180, 200),
            title: Color::Blue,
            fg: Color::Black,
            user_msg: Color::Black,
            agent_msg: Color::Rgb(0, 100, 0),
            system_msg: Color::DarkGray,
            input: Color::Rgb(120, 80, 0),
            cursor: Color::Rgb(80, 40, 0),
            status_bar: Color::DarkGray,
            sidebar_online: Color::Green,
            sidebar_muted: Color::Gray,
            sidebar_deafened: Color::Blue,
            sidebar_thinking: Color::Rgb(180, 140, 0),
            sidebar_warning: Color::Rgb(200, 120, 0),
            sidebar_dead: Color::Red,
            search_highlight: Color::Rgb(200, 150, 0),
            help_overlay_bg: Color::Rgb(240, 240, 245),
        }
    }

    pub fn border_style(&self) -> Style {
        Style::default().fg(self.border)
    }

    pub fn title_style(&self) -> Style {
        Style::default().fg(self.title).add_modifier(Modifier::BOLD)
    }

    pub fn footer_style(&self) -> Style {
        Style::default().fg(self.status_bar)
    }

    /// Line-1 presence marker (● online, ○ muted, 💀 dead).
    pub fn presence_style(&self, status: AgentStatus) -> Style {
        Style::default().fg(match status {
            AgentStatus::Dead => self.sidebar_dead,
            AgentStatus::Muted => self.sidebar_muted,
            AgentStatus::Initializing => self.sidebar_thinking,
            _ => self.sidebar_online,
        })
    }

    /// §15.3 status detail colors for sidebar line 2 and DM header.
    pub fn status_detail_style(&self, status: AgentStatus) -> Style {
        Style::default().fg(match status {
            AgentStatus::Initializing => self.sidebar_thinking,
            AgentStatus::Idle => self.sidebar_online,
            AgentStatus::Thinking => self.sidebar_thinking,
            AgentStatus::Muted => self.sidebar_muted,
            AgentStatus::Deafened => self.sidebar_deafened,
            AgentStatus::Suspended => self.sidebar_warning,
            AgentStatus::Dead => self.sidebar_dead,
            AgentStatus::RateLimited => self.sidebar_warning,
        })
    }

    pub fn chat_sender_style(&self, sender: crate::app::ChatSender) -> Style {
        use crate::app::ChatSender;
        match sender {
            ChatSender::User => Style::default().fg(self.user_msg),
            ChatSender::Agent => Style::default().fg(self.agent_msg),
            ChatSender::System => Style::default()
                .fg(self.system_msg)
                .add_modifier(Modifier::ITALIC),
        }
    }
}
