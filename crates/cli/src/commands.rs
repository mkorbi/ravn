//! Client-side slash-commands. These never round-trip to the LLM —
//! the cli parses them locally before constructing a `RunContext`.

use crate::app::{App, DisplayRole, DisplayedMessage};

pub enum SlashCommand {
    Help,
    About,
    Clear,
    Quit,
    Unknown(String),
}

impl SlashCommand {
    /// Parse the **trimmed** input as a slash-command. Returns `None`
    /// if the input doesn't start with `/`.
    pub fn parse(input: &str) -> Option<Self> {
        let trimmed = input.trim();
        if !trimmed.starts_with('/') {
            return None;
        }
        // Strip leading '/' and take the first whitespace-separated token.
        let body = &trimmed[1..];
        let cmd = body.split_whitespace().next().unwrap_or("").to_lowercase();
        Some(match cmd.as_str() {
            "help" | "h" | "?" => SlashCommand::Help,
            "about" => SlashCommand::About,
            "clear" | "cls" => SlashCommand::Clear,
            "quit" | "exit" | "q" => SlashCommand::Quit,
            other => SlashCommand::Unknown(other.to_string()),
        })
    }

    /// Apply the command to the app state. Returns immediately —
    /// no async work involved (no DB, no LLM).
    pub fn apply(self, app: &mut App) {
        match self {
            SlashCommand::Help => {
                app.messages.push(DisplayedMessage {
                    role: DisplayRole::Notice,
                    text: HELP_TEXT.to_string(),
                });
            }
            SlashCommand::About => {
                app.messages.push(DisplayedMessage {
                    role: DisplayRole::Splash,
                    text: String::new(),
                });
            }
            SlashCommand::Clear => {
                // Keep the leading Splash so the user still sees the
                // ravn header after clearing.
                app.messages.retain(|m| m.role == DisplayRole::Splash);
                app.last_error = None;
            }
            SlashCommand::Quit => {
                app.should_quit = true;
            }
            SlashCommand::Unknown(name) => {
                app.messages.push(DisplayedMessage {
                    role: DisplayRole::Notice,
                    text: format!("unknown command: /{name}  (try /help)"),
                });
            }
        }
    }
}

const HELP_TEXT: &str = "available slash-commands:\n  \
/help            list slash-commands\n  \
/about           reprint the startup splash\n  \
/clear           wipe the scrollback (session keeps running)\n  \
/quit, /exit     close ravn\n\n\
non-slash input is sent to the model.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_known_commands() {
        assert!(matches!(SlashCommand::parse("/help"), Some(SlashCommand::Help)));
        assert!(matches!(SlashCommand::parse("/h"), Some(SlashCommand::Help)));
        assert!(matches!(SlashCommand::parse("/?"), Some(SlashCommand::Help)));
        assert!(matches!(SlashCommand::parse("/about"), Some(SlashCommand::About)));
        assert!(matches!(SlashCommand::parse("/clear"), Some(SlashCommand::Clear)));
        assert!(matches!(SlashCommand::parse("/CLS"), Some(SlashCommand::Clear)));
        assert!(matches!(SlashCommand::parse("/quit"), Some(SlashCommand::Quit)));
        assert!(matches!(SlashCommand::parse("/exit"), Some(SlashCommand::Quit)));
        assert!(matches!(SlashCommand::parse("/q"), Some(SlashCommand::Quit)));
    }

    #[test]
    fn parse_unknown_command() {
        match SlashCommand::parse("/foo") {
            Some(SlashCommand::Unknown(name)) => assert_eq!(name, "foo"),
            other => panic!("expected Unknown(foo), got {:?}", other.map(|_| ())),
        }
    }

    #[test]
    fn non_slash_input_returns_none() {
        assert!(SlashCommand::parse("hello").is_none());
        assert!(SlashCommand::parse("").is_none());
        assert!(SlashCommand::parse("   ").is_none());
    }

    #[test]
    fn whitespace_around_slash_is_tolerated() {
        assert!(matches!(SlashCommand::parse("  /help  "), Some(SlashCommand::Help)));
    }
}
