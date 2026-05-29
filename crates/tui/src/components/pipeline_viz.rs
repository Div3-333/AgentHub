//! Live pipeline flow visualizer (blueprint §15.1 sidebar).

use crate::app::{PipelineInfo, PipelineRunStatus};
use crate::theme::Theme;
use agenthub_core::pipeline::{parse, PipelineStage};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

/// Progress bar string: `████░░` style.
pub fn progress_bar(percent: u8, width: usize) -> String {
    let width = width.max(1);
    let filled = ((percent as usize).saturating_mul(width) / 100).min(width);
    let empty = width.saturating_sub(filled);
    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
}

/// 1-based stage number shown in the progress header.
pub fn display_stage(p: &PipelineInfo) -> u32 {
    let total = p.stage_labels.len().max(1);
    match p.status {
        PipelineRunStatus::Complete => total as u32,
        PipelineRunStatus::Failed | PipelineRunStatus::Running => {
            (p.active_index + 1).min(total) as u32
        }
    }
}

pub fn total_stages(p: &PipelineInfo) -> u32 {
    p.stage_labels.len().max(1) as u32
}

/// Build pipeline state when `BusEvent::PipelineStarted` fires.
pub fn pipeline_from_started(definition: &str) -> PipelineInfo {
    let stage_labels = labels_from_definition(definition);
    PipelineInfo {
        stage_labels,
        active_index: 0,
        progress: 0,
        output_preview: String::new(),
        status: PipelineRunStatus::Running,
    }
}

/// Demo sidebar state matching blueprint §15.1 mockup.
pub fn demo_pipeline_info() -> PipelineInfo {
    PipelineInfo {
        stage_labels: vec!["@gemini-1".into(), "cargo".into(), "@claude-1".into()],
        active_index: 1,
        progress: 67,
        output_preview: String::new(),
        status: PipelineRunStatus::Running,
    }
}

/// Apply `BusEvent::PipelineStageComplete`.
pub fn on_stage_complete(p: &mut PipelineInfo, completed_index: usize, preview: &str) {
    let total = p.stage_labels.len().max(1);
    p.output_preview = preview.to_string();
    p.active_index = (completed_index + 1).min(total.saturating_sub(1));
    let done = completed_index + 1;
    p.progress = (((done * 100) / total).min(100)) as u8;
    p.status = PipelineRunStatus::Running;
}

/// Apply `BusEvent::PipelineComplete`.
pub fn on_pipeline_complete(p: &mut PipelineInfo) {
    let total = p.stage_labels.len().max(1);
    p.active_index = total.saturating_sub(1);
    p.progress = 100;
    p.status = PipelineRunStatus::Complete;
}

/// Apply `BusEvent::PipelineFailed`.
pub fn on_pipeline_failed(p: &mut PipelineInfo, failed_stage: usize) {
    let total = p.stage_labels.len().max(1);
    p.active_index = failed_stage.min(total.saturating_sub(1));
    p.status = PipelineRunStatus::Failed;
}

fn labels_from_definition(definition: &str) -> Vec<String> {
    match parse(definition) {
        Ok(stages) => stages.iter().map(stage_label).collect(),
        Err(_) => vec![truncate_label(definition, 28)],
    }
}

fn stage_label(stage: &PipelineStage) -> String {
    match stage {
        PipelineStage::Agent(agent) => match &agent.tag {
            Some(tag) => format!("@{tag}"),
            None => {
                let prompt = agent.prompt.trim();
                if prompt.is_empty() {
                    "broadcast".into()
                } else {
                    truncate_label(prompt, 16)
                }
            }
        },
        PipelineStage::Unix(unix) => {
            let cmd = unix.command.trim();
            cmd.split_whitespace()
                .next()
                .map(|w| truncate_label(w, 16))
                .unwrap_or_else(|| truncate_label(cmd, 16))
        }
    }
}

fn truncate_label(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let end = s
            .char_indices()
            .nth(max.saturating_sub(1))
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(s.len());
        format!("{}…", &s[..end])
    }
}

/// Header line: `Stage 2/3 ████░░ 67%` (or done/failed suffix).
pub fn stage_progress_line(p: &PipelineInfo, bar_width: usize) -> Line<'static> {
    let cur = display_stage(p);
    let total = total_stages(p);
    let suffix = match p.status {
        PipelineRunStatus::Complete => " ✓",
        PipelineRunStatus::Failed => " ✗",
        PipelineRunStatus::Running => "",
    };
    Line::from(format!(
        "Stage {cur}/{total} {} {}{}",
        progress_bar(p.progress, bar_width),
        p.progress,
        suffix
    ))
}

/// Flow graph lines with the active stage highlighted (blueprint §15.1).
pub fn flow_lines(p: &PipelineInfo, width: u16, theme: &Theme) -> Vec<Line<'static>> {
    if p.stage_labels.is_empty() {
        return vec![Line::from("(empty pipeline)")];
    }

    let max_w = usize::from(width.saturating_sub(2)).max(8);
    let mut lines = Vec::new();
    let mut row: Vec<Span<'static>> = Vec::new();
    let mut row_len = 0usize;

    for (i, label) in p.stage_labels.iter().enumerate() {
        let sep = if i == 0 { "" } else { " → " };
        let piece_len = sep.len() + label.len();
        if row_len > 0 && row_len + piece_len > max_w {
            lines.push(Line::from(row));
            row = Vec::new();
            row.push(Span::raw(" → "));
            row_len = 3;
        } else if i > 0 {
            row.push(Span::raw(" → "));
            row_len += 3;
        }

        let style = stage_style(p, i, theme);
        row.push(Span::styled(label.clone(), style));
        row_len += label.len();
    }

    if !row.is_empty() {
        lines.push(Line::from(row));
    }

    lines
}

fn stage_style(p: &PipelineInfo, index: usize, theme: &Theme) -> Style {
    match p.status {
        PipelineRunStatus::Complete => Style::default()
            .fg(theme.sidebar_online)
            .add_modifier(Modifier::DIM),
        PipelineRunStatus::Failed if index == p.active_index => Style::default()
            .fg(theme.sidebar_dead)
            .add_modifier(Modifier::BOLD),
        PipelineRunStatus::Failed => Style::default().add_modifier(Modifier::DIM),
        PipelineRunStatus::Running if index == p.active_index => {
            theme.title_style().add_modifier(Modifier::BOLD)
        }
        PipelineRunStatus::Running if index < p.active_index => Style::default()
            .fg(theme.sidebar_online)
            .add_modifier(Modifier::DIM),
        PipelineRunStatus::Running => Style::default().add_modifier(Modifier::DIM),
    }
}

/// Lines for embedding in the sidebar pipeline section.
pub fn pipeline_lines(
    pipeline: Option<&PipelineInfo>,
    width: u16,
    theme: &Theme,
) -> Vec<Line<'static>> {
    match pipeline {
        Some(p) => {
            let bar_w = usize::from(width.saturating_sub(14)).clamp(6, 12);
            let mut lines = vec![stage_progress_line(p, bar_w)];
            lines.extend(flow_lines(p, width, theme));
            if !p.output_preview.is_empty() {
                lines.push(Line::from(Span::styled(
                    truncate_label(&p.output_preview, usize::from(width).max(12)),
                    Style::default().add_modifier(Modifier::DIM | Modifier::ITALIC),
                )));
            }
            lines
        }
        None => vec![Line::from(Span::styled(
            "(idle)",
            Style::default().add_modifier(Modifier::DIM),
        ))],
    }
}

/// Compact pipeline block for sidebar footer.
pub fn render_compact(
    frame: &mut Frame,
    area: Rect,
    pipeline: Option<&PipelineInfo>,
    theme: &Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let lines = pipeline_lines(pipeline, area.width, theme);
    let block = Block::default()
        .title(" Pipeline ")
        .borders(Borders::ALL)
        .border_style(theme.border_style());
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::PipelineRunStatus;
    use crate::theme::Theme;

    #[test]
    fn progress_bar_67_percent() {
        let bar = progress_bar(67, 10);
        assert_eq!(bar.chars().count(), 10);
        assert!(bar.contains('█'));
        assert!(bar.contains('░'));
    }

    #[test]
    fn stage_label_agent_and_unix() {
        use agenthub_core::pipeline::{AgentStage, UnixStage};
        assert_eq!(
            stage_label(&PipelineStage::Agent(AgentStage {
                tag: Some("gemini-1".into()),
                prompt: "write".into(),
            })),
            "@gemini-1"
        );
        assert_eq!(
            stage_label(&PipelineStage::Unix(UnixStage {
                command: "cargo build".into(),
            })),
            "cargo"
        );
    }

    #[test]
    fn pipeline_from_started_parses_stages() {
        let p = pipeline_from_started("@gemini-1 hello | > cargo build | @claude-1 review");
        assert_eq!(p.stage_labels.len(), 3);
        assert_eq!(p.stage_labels[0], "@gemini-1");
        assert_eq!(p.stage_labels[1], "cargo");
        assert_eq!(p.stage_labels[2], "@claude-1");
        assert_eq!(p.active_index, 0);
        assert_eq!(p.progress, 0);
    }

    #[test]
    fn on_stage_complete_updates_progress_and_active() {
        let mut p = pipeline_from_started("@a x | > echo y | @b z");
        on_stage_complete(&mut p, 0, "out0");
        assert_eq!(p.active_index, 1);
        assert_eq!(p.progress, 33);
        assert_eq!(p.output_preview, "out0");
        on_stage_complete(&mut p, 1, "out1");
        assert_eq!(p.active_index, 2);
        assert_eq!(p.progress, 66);
    }

    #[test]
    fn on_pipeline_complete_marks_done() {
        let mut p = pipeline_from_started("@a x | @b y");
        on_pipeline_complete(&mut p);
        assert_eq!(p.status, PipelineRunStatus::Complete);
        assert_eq!(p.progress, 100);
    }

    #[test]
    fn pipeline_lines_show_stage_and_flow() {
        let info = demo_pipeline_info();
        let theme = Theme::dark();
        let lines = pipeline_lines(Some(&info), 40, &theme);
        assert!(lines.len() >= 2);
        assert!(lines[0].spans[0].content.contains("Stage 2/3"));
        assert!(lines[0].spans[0].content.contains("67"));
        let flow_text: String = lines[1..]
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(flow_text.contains("@gemini-1"));
        assert!(flow_text.contains("cargo"));
    }

    #[test]
    fn flow_lines_highlight_active_stage() {
        let info = demo_pipeline_info();
        let theme = Theme::dark();
        let lines = flow_lines(&info, 40, &theme);
        let active = lines
            .iter()
            .flat_map(|l| &l.spans)
            .find(|s| s.content == "cargo")
            .expect("cargo label");
        assert!(active.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn display_stage_matches_blueprint_demo() {
        let info = demo_pipeline_info();
        assert_eq!(display_stage(&info), 2);
        assert_eq!(total_stages(&info), 3);
    }
}
