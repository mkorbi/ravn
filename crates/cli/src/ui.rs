//! ratatui render functions.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};
use ravn_tools::Permission;

use crate::app::{App, DisplayRole};

pub fn render(frame: &mut Frame<'_>, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(frame.area());

    render_scrollback(frame, chunks[0], app);
    render_input(frame, chunks[1], app);
    render_status(frame, chunks[2], app);

    if app.pending_approval.is_some() {
        render_approval_modal(frame, app);
    }
}

fn render_scrollback(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let mut lines: Vec<Line> = Vec::new();
    for m in &app.messages {
        match m.role {
            DisplayRole::Splash => {
                lines.extend(crate::splash::splash_lines(env!("CARGO_PKG_VERSION")));
            }
            DisplayRole::User => {
                lines.push(Line::from(Span::styled(
                    "you:",
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                )));
                for raw in m.text.split('\n') {
                    lines.push(Line::from(raw.to_string()));
                }
            }
            DisplayRole::Assistant => {
                lines.push(Line::from(Span::styled(
                    "ravn:",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )));
                for raw in m.text.split('\n') {
                    lines.push(Line::from(raw.to_string()));
                }
            }
            DisplayRole::ToolStart | DisplayRole::ToolEnd => {
                lines.push(Line::from(Span::styled(
                    m.text.clone(),
                    Style::default().fg(Color::DarkGray),
                )));
            }
            DisplayRole::Notice => {
                lines.push(Line::from(Span::styled(
                    m.text.clone(),
                    Style::default().fg(Color::Yellow),
                )));
            }
        }
        lines.push(Line::from(""));
    }
    if app.streaming_active && !app.streaming_buffer.is_empty() {
        lines.push(Line::from(Span::styled(
            "ravn:",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )));
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
    let prompt = if app.pending_approval.is_some() {
        "  (approval needed) "
    } else if app.recording {
        "  🎙 recording — /voice to stop "
    } else if app.streaming_active {
        "  (streaming — Esc to cancel) "
    } else {
        "> "
    };

    let cursor_byte = app.input.cursor.min(app.input.text.len());
    let (before, rest) = app.input.text.split_at(cursor_byte);
    let (cursor_ch, after) = match rest.chars().next() {
        Some(ch) => {
            let len = ch.len_utf8();
            (ch.to_string(), &rest[len..])
        }
        None => (" ".to_string(), rest),
    };

    let cursor_style = Style::default()
        .bg(Color::Yellow)
        .fg(Color::Black)
        .add_modifier(Modifier::BOLD);

    let spans = vec![
        Span::styled(prompt, Style::default().fg(Color::DarkGray)),
        Span::raw(before.to_string()),
        Span::styled(cursor_ch, cursor_style),
        Span::raw(after.to_string()),
    ];

    let para =
        Paragraph::new(Line::from(spans)).block(Block::default().borders(Borders::ALL));
    frame.render_widget(para, area);
}

fn render_status(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let hit = match app.cache_hit_rate() {
        Some(r) => format!("{:>3.0}%", r * 100.0),
        None => "  --".into(),
    };
    let rec = if app.recording { "🎙 REC │ " } else { "" };
    let status = format!(
        " {rec}session {} │ in {} out {} cache_r {} hit {} │ ${:.4} ",
        short_id(&app.session_id),
        app.input_tokens,
        app.output_tokens,
        app.cache_read_tokens,
        hit,
        app.cost_usd
    );
    let para = Paragraph::new(status).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(para, area);
}

fn render_approval_modal(frame: &mut Frame<'_>, app: &App) {
    let req = app.pending_approval.as_ref().unwrap();
    let perm_label = match req.permission {
        Permission::Read => "READ",
        Permission::Write => "WRITE",
        Permission::Exec => "EXEC",
    };
    let perm_color = match req.permission {
        Permission::Read => Color::Cyan,
        Permission::Write => Color::Yellow,
        Permission::Exec => Color::Red,
    };
    let args_pretty = serde_json::to_string_pretty(&req.args).unwrap_or_default();

    let lines: Vec<Line> = vec![
        Line::from(Span::styled(
            format!("Tool call requested: {}", req.tool),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!("Permission: {}", perm_label),
            Style::default().fg(perm_color),
        )),
        Line::from(""),
        Line::from(Span::raw("Args:")),
    ]
    .into_iter()
    .chain(args_pretty.lines().map(|l| Line::from(format!("  {l}"))))
    .chain([
        Line::from(""),
        Line::from(Span::styled(
            "[y] allow once   [n] deny   [a] allow this tool always   [Esc] cancel run",
            Style::default().fg(Color::Cyan),
        )),
    ])
    .collect();

    let body_lines = lines.len() as u16;
    let width = 80.min(frame.area().width.saturating_sub(4));
    let height = (body_lines + 2).min(frame.area().height.saturating_sub(2));
    let area = center_rect(width, height, frame.area());

    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" approval ")
        .border_style(Style::default().fg(perm_color));
    let para = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
    frame.render_widget(para, area);
}

fn center_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width,
        height,
    }
}

fn short_id(s: &str) -> &str {
    s.get(..8).unwrap_or(s)
}
