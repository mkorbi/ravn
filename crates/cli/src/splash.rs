//! Startup splash block — first entry in the scrollback, persistent.
//!
//! Left column: ASCII raven (credit: "SSt" signature preserved in the
//! art). Right column: welcome line, version, slash-commands, repo URL.
//! Rendered inline as a single multi-line message so it scrolls with
//! the rest of history.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

const RAVEN: &[&str] = &[
    "                                                  ,::::.._",
    "                                               ,':::::::::.",
    "                                           _,-'`:::,::(o)::`-,.._",
    "                                        _.', ', `:::::::::;'-..__`.",
    "                                   _.-'' ' ,' ,' ,\\:::,'::-`'''",
    "                               _.-'' , ' , ,'  ' ,' `:::/",
    "                         _..-'' , ' , ' ,' , ,' ',' '/::",
    "                 _...:::'`-..'_, ' , ,'  , ' ,'' , ,'::|",
    "              _`.:::::,':::::,'::`-:..'_',_'_,'..-'::,'|",
    "      _..-:::'::,':::::::,':::,':,'::,':::,'::::::,':::;",
    "        `':,'::::::,:,':::::::::::::::::':::,'::_:::,'/",
    "        __..:'::,':::::::--''' `-:,':,':::'::-' ,':::/",
    "   _.::::::,:::.-''-`-`..'_,'. ,',  , ' , ,'  ', `','",
    " ,::SSt:''''`                 \\:. . ,' '  ,',' '_,'",
    "                               ``::._,'_'_,',.-'",
    "                                   \\\\ \\\\",
    "                                    \\\\_\\\\",
    "                                     \\\\`-`.-'_",
    "                                  .`-.\\\\__`. ``",
    "                                     ``-.-._",
    "                                         `",
];

const URL: &str = "https://github.com/mkorbi/ravn";
const GAP: &str = "   ";

/// Build the splash content as a vector of ratatui `Line`s.
///
/// Two columns: ASCII art on the left, padded to a uniform width;
/// welcome + slash-commands + URL on the right. The raven is taller
/// than the right column, so trailing art rows render alone.
pub fn splash_lines(version: &str) -> Vec<Line<'static>> {
    let cyan = Style::default().fg(Color::Cyan);
    let bold_green = Style::default().fg(Color::Green).add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(Color::DarkGray);
    let plain = Style::default();

    let right: Vec<(String, Style)> = vec![
        (format!("ravn v{version}"), bold_green),
        ("a personal-assistant AI agent in rust".into(), dim),
        (String::new(), plain),
        ("welcome — type a message to chat,".into(), plain),
        ("or one of these slash-commands:".into(), plain),
        (String::new(), plain),
        ("  /help    list slash-commands".into(), cyan),
        ("  /about   reprint this splash".into(), cyan),
        ("  /clear   wipe the scrollback".into(), cyan),
        ("  /quit    exit ravn".into(), cyan),
        (String::new(), plain),
        (URL.into(), dim),
    ];

    let raven_width = RAVEN
        .iter()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0);

    let total_rows = RAVEN.len().max(right.len());
    let mut out = Vec::with_capacity(total_rows);

    for i in 0..total_rows {
        let raven_line = RAVEN.get(i).copied().unwrap_or("");
        let pad_count = raven_width.saturating_sub(raven_line.chars().count());
        let padded_left: String = raven_line
            .chars()
            .chain(std::iter::repeat_n(' ', pad_count))
            .collect();

        let mut spans = vec![Span::styled(padded_left, dim), Span::raw(GAP)];
        if let Some((text, style)) = right.get(i) {
            spans.push(Span::styled(text.clone(), *style));
        }
        out.push(Line::from(spans));
    }
    out
}
