//! Startup splash block — first entry in the scrollback, persistent.
//!
//! Left column: ASCII raven head. Right column: welcome line, version,
//! repo URL, and a quick list of slash-commands. Rendered inline as a
//! single multi-line message so it scrolls with the rest of history.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

const RAVEN: &[&str] = &[
    "                                ",
    "        ___                     ",
    "     ,-\"   `.                  ",
    "    /        \\_,---.           ",
    "   |   _    ,`     )           ",
    "   |  (●)  (  ●    /            ",
    "    \\       )    /             ",
    "     `-.__,-`---'               ",
    "        |_/                     ",
    "         \\\\                    ",
    "          `\\                   ",
    "           `.                   ",
    "                                ",
];

const URL: &str = "https://github.com/mkorbi/ravn";

/// Build the splash content as a vector of ratatui `Line`s. Two columns
/// laid out with explicit padding so it works in any terminal width
/// (left ASCII art is ~32 cols, right column is free-form).
pub fn splash_lines(version: &str) -> Vec<Line<'static>> {
    let cyan = Style::default().fg(Color::Cyan);
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(Color::DarkGray);

    // Right column content as (line, style) pairs.
    let right: Vec<(String, Style)> = vec![
        (format!("ravn v{version}"), bold.fg(Color::Green)),
        ("a personal-assistant AI agent in rust".to_string(), dim),
        (String::new(), Style::default()),
        ("welcome — type a message to chat,".to_string(), Style::default()),
        ("or one of these slash-commands:".to_string(), Style::default()),
        (String::new(), Style::default()),
        ("  /help    list slash-commands".to_string(), cyan),
        ("  /about   reprint this splash".to_string(), cyan),
        ("  /clear   wipe the scrollback".to_string(), cyan),
        ("  /quit    exit ravn".to_string(), cyan),
        (String::new(), Style::default()),
        (URL.to_string(), dim),
    ];

    let raven_lines = RAVEN.len();
    let right_lines = right.len();
    let total = raven_lines.max(right_lines);

    let mut out = Vec::with_capacity(total);
    for i in 0..total {
        let left = RAVEN.get(i).copied().unwrap_or("                                ");
        let (text, style) = right
            .get(i)
            .cloned()
            .unwrap_or_else(|| (String::new(), Style::default()));
        out.push(Line::from(vec![
            Span::styled(left.to_string(), Style::default().fg(Color::DarkGray)),
            Span::styled("   ", Style::default()),
            Span::styled(text, style),
        ]));
    }
    out
}
