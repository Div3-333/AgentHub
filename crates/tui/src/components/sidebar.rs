//! Agent status sidebar panel (blueprint §15.1, §15.3).

use crate::app::{AgentEntry, AgentStatus, PipelineInfo, WorkspaceMode};
use crate::components::pipeline_viz;
use crate::theme::Theme;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

/// §15.3 status glyphs (also used in DM header).
pub fn status_glyph(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Initializing => "◌",
        AgentStatus::Idle => "●",
        AgentStatus::Thinking => "⏳",
        AgentStatus::Muted => "🔇",
        AgentStatus::Deafened => "🔕",
        AgentStatus::Suspended => "⏸",
        AgentStatus::Dead => "💀",
        AgentStatus::RateLimited => "⚠",
    }
}

/// Line-1 online presence (mockup: ● online, ○ muted).
pub fn presence_glyph(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Dead => "💀",
        AgentStatus::Muted => "○",
        AgentStatus::Initializing => "◌",
        _ => "●",
    }
}

/// Line-2 detail icon (mockup uses ✅ for idle).
pub fn status_detail_icon(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Initializing => "◌",
        AgentStatus::Idle => "✅",
        AgentStatus::Thinking => "⏳",
        AgentStatus::Muted => "🔇",
        AgentStatus::Deafened => "🔕",
        AgentStatus::Suspended => "⏸",
        AgentStatus::Dead => "💀",
        AgentStatus::RateLimited => "⚠",
    }
}

pub fn status_label(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Initializing => "Starting…",
        AgentStatus::Idle => "Idle",
        AgentStatus::Thinking => "Thinking...",
        AgentStatus::Muted => "Muted",
        AgentStatus::Deafened => "Deafened",
        AgentStatus::Suspended => "Timed out",
        AgentStatus::Dead => "Dead",
        AgentStatus::RateLimited => "Rate limited",
    }
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    agents: &[AgentEntry],
    mode: WorkspaceMode,
    snapshot_count: usize,
    pipeline: Option<&PipelineInfo>,
    theme: &Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let meta_h = 5u16.min(area.height);
    let pipe_h = 6u16.min(area.height.saturating_sub(meta_h));
    let agent_h = area.height.saturating_sub(meta_h + pipe_h).max(1);

    let chunks = Layout::vertical([
        Constraint::Length(agent_h),
        Constraint::Length(meta_h),
        Constraint::Length(pipe_h),
    ])
    .split(area);

    let active_count = agents
        .iter()
        .filter(|a| a.status != AgentStatus::Dead)
        .count();

    let agent_lines: Vec<Line> = agents
        .iter()
        .flat_map(|a| {
            let presence = presence_glyph(a.status);
            let detail = status_detail_icon(a.status);
            let label = status_label(a.status);
            [
                Line::from(vec![
                    Span::styled(
                        format!("{presence} {} ", a.tag),
                        theme.presence_style(a.status),
                    ),
                    Span::styled(
                        format!("[{}]", a.role),
                        Style::default().add_modifier(Modifier::DIM),
                    ),
                ]),
                Line::from(Span::styled(
                    format!("   {detail} {label}"),
                    theme.status_detail_style(a.status),
                )),
            ]
        })
        .collect();

    let agents_block = Block::default()
        .title(" AGENT STATUS ")
        .borders(Borders::ALL)
        .border_style(theme.border_style());
    frame.render_widget(Paragraph::new(agent_lines).block(agents_block), chunks[0]);

    let meta = vec![
        Line::from(Span::styled(
            format!("MODE: {}", mode.label()),
            theme.title_style(),
        )),
        Line::from(format!("AGENTS: {active_count}/16")),
        Line::from(format!("SNAPSHOTS: {snapshot_count}")),
    ];
    let meta_block = Block::default().borders(Borders::LEFT | Borders::RIGHT);
    frame.render_widget(Paragraph::new(meta).block(meta_block), chunks[1]);

    let mut pipeline_lines = vec![Line::from(Span::styled("PIPELINE", theme.title_style()))];
    pipeline_lines.extend(pipeline_viz::pipeline_lines(
        pipeline,
        chunks[2].width,
        theme,
    ));
    let pipe_block = Block::default()
        .title(" Pipeline ")
        .borders(Borders::ALL)
        .border_style(theme.border_style());
    frame.render_widget(Paragraph::new(pipeline_lines).block(pipe_block), chunks[2]);
}
