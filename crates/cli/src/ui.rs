//! ratatui render functions.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};
use ravn_llm::Role;

use crate::app::App;

pub fn render(frame: &mut Frame<'_>, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),     // scrollback
            Constraint::Length(3),  // input
            Constraint::Length(1),  // status
        ])
        .split(frame.area());

    render_scrollback(frame, chunks[0], app);
    render_input(frame, chunks[1], app);
    render_status(frame, chunks[2], app);
}

fn render_scrollback(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let mut lines: Vec<Line> = Vec::new();
    for m in &app.messages {
        lines.push(role_header(m.role));
        for raw in m.text.split('\n') {
            lines.push(Line::from(raw.to_string()));
        }
        lines.push(Line::from(""));
    }
    if app.streaming_active && !app.streaming_buffer.is_empty() {
        lines.push(role_header(Role::Assistant));
        for raw in app.streaming_buffer.split('\n') {
            lines.push(Line::from(raw.to_string()));
        }
        lines.push(Line::from(Span::styled(
            "▌",
            Style::default().fg(Color::DarkGray),
        )));
    }
    if let Some(err) = &app.last_error {
        lines.push(Line::from(Span::styled(
            format!("error: {err}"),
            Style::default().fg(Color::Red),
        )));
    }

    // Auto-scroll: show the last `area.height` lines.
    let visible = area.height.saturating_sub(2) as usize;
    let offset = lines.len().saturating_sub(visible);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" ravn — {} ", app.model));
    let para = Paragraph::new(lines.into_iter().skip(offset).collect::<Vec<_>>())
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(para, area);
}

fn render_input(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let prompt = if app.streaming_active {
        "  (streaming — Esc to cancel) "
    } else {
        "> "
    };
    let mut spans = vec![Span::styled(
        prompt,
        Style::default().fg(Color::DarkGray),
    )];
    spans.push(Span::raw(&app.input));
    spans.push(Span::styled(
        "_",
        Style::default().fg(Color::Yellow).add_modifier(Modifier::SLOW_BLINK),
    ));
    let para =
        Paragraph::new(Line::from(spans)).block(Block::default().borders(Borders::ALL));
    frame.render_widget(para, area);
}

fn render_status(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let status = format!(
        " session {} │ in {} out {} cache_r {} thinking {} │ ${:.4} ",
        short_id(&app.session_id),
        app.input_tokens,
        app.output_tokens,
        app.cache_read_tokens,
        app.reasoning_tokens,
        app.cost_usd
    );
    let para = Paragraph::new(status).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(para, area);
}

fn role_header(role: Role) -> Line<'static> {
    let (label, color) = match role {
        Role::User => ("you", Color::Cyan),
        Role::Assistant => ("ravn", Color::Green),
        Role::System => ("system", Color::DarkGray),
        Role::Tool => ("tool", Color::Magenta),
    };
    Line::from(Span::styled(
        format!("{label}:"),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    ))
}

fn short_id(s: &str) -> &str {
    s.get(..8).unwrap_or(s)
}
