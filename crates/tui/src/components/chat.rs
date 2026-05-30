//! Chat history pane (blueprint §15.1).

use crate::app::ChatLine;
use crate::theme::Theme;
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

/// Render scrollable chat history with optional search highlighting.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    lines: &[ChatLine],
    scroll: usize,
    search_query: Option<&str>,
    theme: &Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let inner_width = area.width.saturating_sub(2) as usize;
    let visible_height = area.height.saturating_sub(2) as usize;
    let query = search_query.filter(|q| !q.is_empty());

    let rendered: Vec<Line<'static>> = lines
        .iter()
        .flat_map(|line| format_chat_line(line, inner_width, query, theme))
        .collect();

    let total = rendered.len();
    let max_scroll = total.saturating_sub(visible_height.max(1));
    let scroll = scroll.min(max_scroll);

    let visible: Vec<Line<'static>> = rendered
        .into_iter()
        .skip(scroll)
        .take(visible_height.max(1))
        .collect();

    let title = if query.is_some() {
        " CHAT HISTORY (search) "
    } else {
        " CHAT HISTORY "
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(theme.border_style());

    let paragraph = Paragraph::new(visible)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

/// Wrapped display rows for all chat lines (must match [`render`] scroll math).
#[must_use]
pub fn total_rendered_rows(lines: &[ChatLine], inner_width: usize) -> usize {
    if inner_width == 0 {
        return lines.len();
    }
    lines
        .iter()
        .map(|line| wrapped_row_count(line, inner_width))
        .sum()
}

fn wrapped_row_count(line: &ChatLine, inner_width: usize) -> usize {
    let prefix_len = format!("[{}] ", line.time_label).len();
    let body_width = inner_width.saturating_sub(prefix_len);
    let rows = wrap_plain_text(&line.text, body_width, Style::default()).len();
    rows.max(1)
}

fn format_chat_line(
    line: &ChatLine,
    width: usize,
    query: Option<&str>,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let base_style = theme.chat_sender_style(line.sender);
    let prefix = format!("[{}] ", line.time_label);
    let prefix_style = Style::default().fg(theme.border);
    let prefix_len = prefix.len();

    let body_spans = if let Some(q) = query {
        highlight_substring(&line.text, q, base_style, theme.search_highlight)
    } else {
        vec![Span::styled(line.text.clone(), base_style)]
    };

    let flat: String = body_spans.iter().map(|s| s.content.as_ref()).collect();

    let mut out = Vec::new();
    let mut first = true;
    for wrapped in wrap_plain_text(&flat, width.saturating_sub(prefix_len), base_style) {
        if first {
            let mut spans = vec![Span::styled(prefix.clone(), prefix_style)];
            spans.extend(wrapped);
            out.push(Line::from(spans));
            first = false;
        } else {
            let mut spans = vec![Span::raw(" ".repeat(prefix_len))];
            spans.extend(wrapped);
            out.push(Line::from(spans));
        }
    }
    if out.is_empty() {
        out.push(Line::from(vec![
            Span::styled(prefix, prefix_style),
            Span::styled(String::new(), base_style),
        ]));
    }
    out
}

fn wrap_plain_text(text: &str, max_width: usize, style: Style) -> Vec<Vec<Span<'static>>> {
    if max_width == 0 {
        return vec![vec![Span::styled(text.to_string(), style)]];
    }
    text.split('\n')
        .flat_map(|paragraph| {
            if paragraph.is_empty() {
                return vec![vec![Span::raw(String::new())]];
            }
            let words: Vec<&str> = paragraph.split_whitespace().collect();
            if words.is_empty() {
                return vec![vec![Span::raw(String::new())]];
            }
            let mut lines = Vec::new();
            let mut current = String::new();
            for word in words {
                let extra = if current.is_empty() { 0 } else { 1 };
                if current.len() + extra + word.len() > max_width && !current.is_empty() {
                    lines.push(vec![Span::styled(current.clone(), style)]);
                    current.clear();
                }
                if !current.is_empty() {
                    current.push(' ');
                }
                current.push_str(word);
            }
            if !current.is_empty() {
                lines.push(vec![Span::styled(current, style)]);
            }
            lines
        })
        .collect()
}

fn highlight_substring(
    text: &str,
    query: &str,
    base: Style,
    highlight: ratatui::style::Color,
) -> Vec<Span<'static>> {
    let q_lower = query.to_lowercase();
    let text_lower = text.to_lowercase();
    let mut spans = Vec::new();
    let mut start = 0usize;
    while let Some(rel) = text_lower[start..].find(&q_lower) {
        let mid = start + rel;
        let end = mid + query.len();
        if mid > start {
            spans.push(Span::styled(text[start..mid].to_string(), base));
        }
        spans.push(Span::styled(
            text[mid..end.min(text.len())].to_string(),
            base.add_modifier(Modifier::REVERSED).fg(highlight),
        ));
        start = end;
        if start >= text.len() {
            break;
        }
    }
    if start < text.len() {
        spans.push(Span::styled(text[start..].to_string(), base));
    }
    if spans.is_empty() {
        spans.push(Span::styled(text.to_string(), base));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{ChatLine, ChatSender};

    #[test]
    fn total_rendered_rows_counts_wrapped_lines() {
        let lines = vec![ChatLine {
            time_label: "12:00:00".into(),
            text: "word ".repeat(80),
            sender: ChatSender::System,
        }];
        let one = total_rendered_rows(&lines, 40);
        assert!(one > 1, "long line should wrap to multiple rows");
    }
}
