//! Split-pane LLM Racing UI (blueprint Part 11 / §15.1).

use std::time::Instant;

use crate::app::RacingPane;
use crate::theme::Theme;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

/// Minimum column width before collapsing to a single stacked view.
const MIN_COL_WIDTH: u16 = 8;

/// Split the main pane into `count` vertical columns (minimum width each).
pub fn column_rects(area: Rect, count: usize) -> Vec<Rect> {
    let count = count.max(1);
    if area.width < MIN_COL_WIDTH.saturating_mul(count as u16) {
        return vec![area];
    }
    let constraints: Vec<Constraint> = (0..count)
        .map(|_| Constraint::Ratio(1, count as u32))
        .collect();
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area)
        .to_vec()
}

/// Format the header status line for one contestant column.
pub fn format_status(pane: &RacingPane, session_start: Option<Instant>) -> String {
    if pane.done {
        let secs = pane
            .elapsed_secs
            .or_else(|| session_start.map(|s| s.elapsed().as_secs_f64()))
            .unwrap_or(0.0);
        format!("✅ {secs:.1}s")
    } else {
        let secs = session_start
            .map(|s| s.elapsed().as_secs_f64())
            .unwrap_or(0.0);
        format!("⏳ {secs:.1}s")
    }
}

/// Footer hint while racing is active.
#[must_use]
pub fn footer_hint(all_done: bool) -> &'static str {
    if all_done {
        "All done — ←/→ select  Enter confirm  Esc discard"
    } else {
        "←/→ select  Enter confirm  Esc discard"
    }
}

/// Whether every contestant has finished.
pub fn all_done(panes: &[RacingPane]) -> bool {
    !panes.is_empty() && panes.iter().all(|p| p.done)
}

/// Render N side-by-side racing columns with streamed output and selection highlight.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    panes: &[RacingPane],
    selected: usize,
    session_start: Option<Instant>,
    theme: &Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    if panes.is_empty() {
        let block = Block::default()
            .title(" LLM Racing ")
            .borders(Borders::ALL)
            .border_style(theme.border_style());
        frame.render_widget(
            Paragraph::new("No racing session active.").block(block),
            area,
        );
        return;
    }

    let columns = column_rects(area, panes.len());
    let selected = selected.min(panes.len().saturating_sub(1));
    let session_done = all_done(panes);

    if columns.len() == 1 && panes.len() > 1 {
        render_stacked(
            frame,
            columns[0],
            panes,
            selected,
            session_start,
            session_done,
            theme,
        );
        return;
    }

    for (idx, (pane, col_area)) in panes.iter().zip(columns).enumerate() {
        render_column(
            frame,
            col_area,
            pane,
            idx == selected,
            session_start,
            session_done,
            theme,
        );
    }
}

/// Narrow-terminal fallback: stack contestants vertically.
fn render_stacked(
    frame: &mut Frame,
    area: Rect,
    panes: &[RacingPane],
    selected: usize,
    session_start: Option<Instant>,
    session_done: bool,
    theme: &Theme,
) {
    let constraints: Vec<Constraint> = (0..panes.len())
        .map(|_| Constraint::Ratio(1, panes.len().max(1) as u32))
        .collect();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);
    for (idx, (pane, row_area)) in panes.iter().zip(rows.iter().copied()).enumerate() {
        render_column(
            frame,
            row_area,
            pane,
            idx == selected,
            session_start,
            session_done,
            theme,
        );
    }
}

fn column_title(pane: &RacingPane, selected: bool) -> String {
    let marker = if selected { "► " } else { "  " };
    format!("{marker}@{} [{}] ", pane.tag, pane.role)
}

fn render_column(
    frame: &mut Frame,
    area: Rect,
    pane: &RacingPane,
    selected: bool,
    session_start: Option<Instant>,
    session_done: bool,
    theme: &Theme,
) {
    let status = format_status(pane, session_start);
    let title = column_title(pane, selected);

    let border_style = if selected {
        Style::default()
            .fg(theme.title)
            .add_modifier(Modifier::BOLD)
    } else {
        theme.border_style()
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let inner_chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(inner);

    let status_style = if pane.done {
        Style::default().fg(theme.sidebar_online)
    } else if session_done && !pane.done {
        Style::default().fg(theme.sidebar_warning)
    } else {
        Style::default().fg(theme.sidebar_thinking)
    };

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(status, status_style))),
        inner_chunks[0],
    );

    let body = visible_body(pane, inner_chunks[1]);

    let body_style = if selected {
        Style::default().fg(theme.fg).bg(theme.help_overlay_bg)
    } else {
        Style::default().fg(theme.fg)
    };

    let paragraph = Paragraph::new(body)
        .style(body_style)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner_chunks[1]);
}

/// Show the tail of streamed output that fits the column body.
fn visible_body(pane: &RacingPane, area: Rect) -> String {
    if pane.content.is_empty() {
        return "…".to_string();
    }
    let max_lines = area.height.max(1) as usize;
    let lines: Vec<&str> = pane.content.lines().collect();
    if lines.len() <= max_lines {
        return pane.content.clone();
    }
    lines[lines.len() - max_lines..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn sample_pane(tag: &str, done: bool) -> RacingPane {
        RacingPane {
            tag: tag.into(),
            role: "Builder".into(),
            content: "output".into(),
            done,
            elapsed_secs: if done { Some(4.8) } else { None },
        }
    }

    #[test]
    fn format_status_thinking() {
        let pane = sample_pane("gemini", false);
        let start = Instant::now() - Duration::from_secs(2);
        let s = format_status(&pane, Some(start));
        assert!(s.starts_with('⏳'));
        assert!(s.starts_with('⏳'));
    }

    #[test]
    fn format_status_done() {
        let pane = sample_pane("claude", true);
        let s = format_status(&pane, None);
        assert!(s.starts_with('✅'));
        assert!(s.contains("4.8"));
    }

    #[test]
    fn column_rects_single_when_too_narrow() {
        let area = Rect::new(0, 0, 10, 20);
        let rects = column_rects(area, 3);
        assert_eq!(rects.len(), 1);
        assert_eq!(rects[0], area);
    }

    #[test]
    fn all_done_requires_every_contestant() {
        assert!(!all_done(&[]));
        assert!(!all_done(&[sample_pane("a", false)]));
        assert!(all_done(&[sample_pane("a", true), sample_pane("b", true),]));
    }

    #[test]
    fn footer_hint_changes_when_all_done() {
        assert!(footer_hint(false).contains("select"));
        assert!(footer_hint(true).contains("All done"));
    }

    #[test]
    fn column_title_marks_selection() {
        let pane = sample_pane("gemini-1", false);
        assert!(column_title(&pane, true).starts_with('►'));
        assert!(!column_title(&pane, false).starts_with('►'));
    }

    #[test]
    fn visible_body_trims_to_viewport() {
        let mut pane = sample_pane("a", false);
        pane.content = (0..20)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let area = Rect::new(0, 0, 40, 3);
        let body = visible_body(&pane, area);
        assert_eq!(body.lines().count(), 3);
        assert!(body.contains("line 19"));
    }
}
