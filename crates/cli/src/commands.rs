//! Client-side slash-commands. These never round-trip to the LLM —
//! the cli parses them locally before constructing a `RunContext`.

use crate::app::{App, AppEvent, DisplayRole, DisplayedMessage};

pub enum SlashCommand {
    Help,
    About,
    Clear,
    Quit,
    Heartbeat(HeartbeatAction),
    Voice,
    Unknown(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum HeartbeatAction {
    /// List configured jobs.
    List,
    /// Fire a job immediately by name (bypasses cron).
    Run(String),
    /// Re-read `heartbeats.toml`.
    Reload,
    /// Malformed invocation; carries a usage hint.
    Usage(String),
}

fn notice(text: impl Into<String>) -> DisplayedMessage {
    DisplayedMessage {
        role: DisplayRole::Notice,
        text: text.into(),
    }
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
            "heartbeat" | "hb" => {
                // Tokens after the command word: <subcommand> [args…].
                let mut rest = body.split_whitespace();
                let _ = rest.next(); // consume the command word itself
                let action = match rest.next().unwrap_or("list").to_lowercase().as_str() {
                    "list" | "ls" => HeartbeatAction::List,
                    "run" | "fire" => match rest.next() {
                        Some(name) => HeartbeatAction::Run(name.to_string()),
                        None => HeartbeatAction::Usage("usage: /heartbeat run <name>".into()),
                    },
                    "reload" => HeartbeatAction::Reload,
                    other => HeartbeatAction::Usage(format!(
                        "unknown /heartbeat subcommand `{other}` (try: list | run <name> | reload)"
                    )),
                };
                SlashCommand::Heartbeat(action)
            }
            "voice" | "v" => SlashCommand::Voice,
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
            SlashCommand::Heartbeat(action) => {
                if let HeartbeatAction::Usage(msg) = action {
                    app.messages.push(notice(msg));
                    return;
                }
                let Some(sched) = app.hb.clone() else {
                    app.messages
                        .push(notice("heartbeat scheduler is not running"));
                    return;
                };
                // Scheduler calls are async; run them off the UI loop and
                // post the result back as a notice.
                let tx = app.events.clone();
                tokio::spawn(async move {
                    let text = match action {
                        HeartbeatAction::List => {
                            let jobs = sched.list().await;
                            if jobs.is_empty() {
                                "no heartbeat jobs configured (~/.ravn/heartbeats.toml)".to_string()
                            } else {
                                let mut s = String::from("heartbeat jobs:");
                                for (name, schedule, enabled) in jobs {
                                    let tag = if enabled { "" } else { "  (disabled)" };
                                    s.push_str(&format!("\n  • {name}  [{schedule}]{tag}"));
                                }
                                s
                            }
                        }
                        HeartbeatAction::Run(name) => {
                            if sched.run_now(&name).await {
                                format!("heartbeat '{name}' triggered")
                            } else {
                                format!("no such heartbeat job: '{name}'")
                            }
                        }
                        HeartbeatAction::Reload => match sched.reload_from_disk().await {
                            Ok(n) => format!("heartbeats reloaded: {n} enabled job(s)"),
                            Err(e) => format!("heartbeat reload failed: {e}"),
                        },
                        HeartbeatAction::Usage(_) => unreachable!("handled above"),
                    };
                    let _ = tx.send(AppEvent::Notice { text }).await;
                });
            }
            SlashCommand::Voice => {
                let Some(voice) = app.voice.clone() else {
                    app.messages.push(notice("voice input unavailable"));
                    return;
                };
                if app.recording {
                    app.recording = false;
                    match voice.recorder.stop() {
                        Some(rec) => {
                            app.messages.push(notice(
                                "transcribing… (first run downloads the model, ~142 MB)",
                            ));
                            let tx = app.events.clone();
                            tokio::spawn(async move {
                                let mono = ravn_voice::resample::to_whisper_mono_16k(
                                    &rec.samples,
                                    rec.sample_rate,
                                    rec.channels,
                                );
                                let event = match voice.transcriber.transcribe(mono).await {
                                    Ok(text) => AppEvent::Transcript { text },
                                    Err(e) => AppEvent::Notice {
                                        text: format!("voice: transcription failed: {e}"),
                                    },
                                };
                                let _ = tx.send(event).await;
                            });
                        }
                        None => app
                            .messages
                            .push(notice("voice: no audio captured (is a mic available?)")),
                    }
                } else {
                    match voice.recorder.start() {
                        Ok(()) => {
                            app.recording = true;
                            app.messages
                                .push(notice("🎙 recording… type /voice again to stop"));
                        }
                        Err(e) => app
                            .messages
                            .push(notice(format!("voice: could not start mic: {e}"))),
                    }
                }
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
/help                  list slash-commands\n  \
/about                 reprint the startup splash\n  \
/clear                 wipe the scrollback (session keeps running)\n  \
/heartbeat list        list scheduled heartbeat jobs\n  \
/heartbeat run <name>  fire a heartbeat job now\n  \
/heartbeat reload      re-read ~/.ravn/heartbeats.toml\n  \
/voice, /v             toggle mic recording → transcript into the input\n  \
/quit, /exit           close ravn\n\n\
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
    fn parse_heartbeat_subcommands() {
        assert!(matches!(
            SlashCommand::parse("/heartbeat"),
            Some(SlashCommand::Heartbeat(HeartbeatAction::List))
        ));
        assert!(matches!(
            SlashCommand::parse("/hb list"),
            Some(SlashCommand::Heartbeat(HeartbeatAction::List))
        ));
        match SlashCommand::parse("/hb run morning") {
            Some(SlashCommand::Heartbeat(HeartbeatAction::Run(n))) => assert_eq!(n, "morning"),
            _ => panic!("expected Run(morning)"),
        }
        assert!(matches!(
            SlashCommand::parse("/hb reload"),
            Some(SlashCommand::Heartbeat(HeartbeatAction::Reload))
        ));
        assert!(matches!(
            SlashCommand::parse("/hb run"),
            Some(SlashCommand::Heartbeat(HeartbeatAction::Usage(_)))
        ));
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
