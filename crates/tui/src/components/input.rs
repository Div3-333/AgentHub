//! Input box (blueprint §15.1).

use crate::theme::Theme;
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

const INPUT_HINT: &str = "/command or @agent message or pipeline syntax";

pub fn render(
    frame: &mut Frame,
    area: Rect,
    display: &str,
    cursor: usize,
    search_mode: bool,
    theme: &Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let title = if search_mode { " SEARCH " } else { " INPUT " };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(theme.border_style());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let mut lines = Vec::new();
    if !search_mode && inner.height >= 2 {
        lines.push(Line::from(Span::styled(
            INPUT_HINT,
            Style::default()
                .fg(theme.border)
                .add_modifier(Modifier::DIM),
        )));
    }

    let safe_cursor = cursor.min(display.len());
    let before = &display[..safe_cursor];
    let at_cursor = display.get(safe_cursor..=safe_cursor).unwrap_or(" ");
    let after = &display[safe_cursor.saturating_add(at_cursor.len())..];

    lines.push(Line::from(vec![
        Span::styled(before.to_string(), Style::default().fg(theme.input)),
        Span::styled(
            at_cursor.to_string(),
            Style::default()
                .fg(theme.cursor)
                .add_modifier(Modifier::UNDERLINED | Modifier::BOLD),
        ),
        Span::styled(after.to_string(), Style::default().fg(theme.input)),
    ]));

    frame.render_widget(Paragraph::new(lines), inner);
}
