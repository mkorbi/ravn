//! Ravn CLI — ratatui-TUI Frontend.

mod app;
mod runner;
mod ui;

use std::io::{self, Stdout};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use app::{App, UiUpdate};
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use ravn_llm::anthropic::AnthropicProvider;
use ravn_llm::openai::OpenAiProvider;
use ravn_persistence::{sessions, Db};
use runner::Provider;
use tokio::sync::mpsc;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

const DEFAULT_ANTHROPIC_MODEL: &str = "claude-sonnet-4-6";
const DEFAULT_OPENAI_MODEL: &str = "gpt-4o";
const DEFAULT_SYSTEM_PROMPT: &str = "You are ravn, a concise and helpful assistant.";
const DEFAULT_MAX_TOKENS: u32 = 4096;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let (provider, model) = select_provider()?;
    let db_path = default_db_path()?;
    let db = Db::open(&db_path)
        .await
        .with_context(|| format!("open db at {}", db_path.display()))?;

    let session_id = uuid::Uuid::new_v4().to_string();
    sessions::create(&db, &session_id, "cli", Some(&model)).await?;

    tracing::info!(
        session = %session_id,
        model = %model,
        db = %db_path.display(),
        "ravn session started"
    );

    let (ui_tx, ui_rx) = mpsc::channel::<UiUpdate>(128);
    let app = App::new(
        db.clone(),
        session_id.clone(),
        model,
        DEFAULT_SYSTEM_PROMPT.to_string(),
        ui_tx,
    );

    let mut terminal = init_terminal()?;
    let result = run(&mut terminal, app, Arc::new(provider), ui_rx).await;
    restore_terminal(&mut terminal)?;

    sessions::close(&db, &session_id).await.ok();

    result
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    tracing_subscriber::registry()
        .with(filter)
        .with(
            fmt::layer()
                .with_target(false)
                // Trace output goes to a file instead of stderr to avoid
                // interfering with the TUI alternate-screen buffer.
                .with_writer(|| -> Box<dyn io::Write + Send> {
                    Box::new(
                        std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(default_log_path().unwrap_or_else(|_| PathBuf::from("/tmp/ravn.log")))
                            .unwrap_or_else(|_| {
                                // Last-resort: discard.
                                std::fs::File::create("/dev/null").expect("open /dev/null")
                            }),
                    )
                }),
        )
        .init();
}

fn select_provider() -> anyhow::Result<(Provider, String)> {
    if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        let model = std::env::var("RAVN_MODEL").unwrap_or_else(|_| DEFAULT_ANTHROPIC_MODEL.into());
        let p = AnthropicProvider::from_env()
            .map_err(|e| anyhow::anyhow!("anthropic init: {e}"))?;
        Ok((Provider::Anthropic(p), model))
    } else if std::env::var("OPENAI_API_KEY").is_ok() {
        let model = std::env::var("RAVN_MODEL").unwrap_or_else(|_| DEFAULT_OPENAI_MODEL.into());
        let p = OpenAiProvider::from_env().map_err(|e| anyhow::anyhow!("openai init: {e}"))?;
        Ok((Provider::OpenAi(p), model))
    } else {
        anyhow::bail!("Set ANTHROPIC_API_KEY or OPENAI_API_KEY before running ravn.")
    }
}

fn default_db_path() -> anyhow::Result<PathBuf> {
    let dir = dirs::data_local_dir()
        .or_else(|| std::env::var("HOME").ok().map(PathBuf::from))
        .ok_or_else(|| anyhow::anyhow!("cannot determine data directory"))?;
    let dir = dir.join("ravn");
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    Ok(dir.join("state.db"))
}

fn default_log_path() -> anyhow::Result<PathBuf> {
    let dir = dirs::data_local_dir()
        .or_else(|| std::env::var("HOME").ok().map(PathBuf::from))
        .ok_or_else(|| anyhow::anyhow!("cannot determine data directory"))?;
    let dir = dir.join("ravn");
    std::fs::create_dir_all(&dir).ok();
    Ok(dir.join("ravn.log"))
}

fn init_terminal() -> anyhow::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    Terminal::new(CrosstermBackend::new(stdout)).map_err(Into::into)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> anyhow::Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

async fn run(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    mut app: App,
    provider: Arc<Provider>,
    mut ui_rx: mpsc::Receiver<UiUpdate>,
) -> anyhow::Result<()> {
    let mut term_events = EventStream::new();

    loop {
        terminal.draw(|f| ui::render(f, &app))?;
        if app.should_quit {
            break;
        }

        tokio::select! {
            biased;
            ev = term_events.next() => {
                match ev {
                    Some(Ok(Event::Key(k))) if k.kind == KeyEventKind::Press => {
                        handle_key(&mut app, k, provider.clone());
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        app.last_error = Some(format!("terminal event: {e}"));
                    }
                    None => break,
                }
            }
            chunk = ui_rx.recv() => {
                match chunk {
                    Some(c) => app.apply_chunk(c),
                    None => break,
                }
            }
        }
    }

    Ok(())
}

fn handle_key(app: &mut App, key: KeyEvent, provider: Arc<Provider>) {
    // Quit shortcuts.
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
        if app.streaming_active {
            if let Some(c) = app.cancel.take() {
                c.cancel();
            }
        } else {
            app.should_quit = true;
        }
        return;
    }
    if matches!(key.code, KeyCode::Esc) && app.streaming_active {
        if let Some(c) = app.cancel.take() {
            c.cancel();
        }
        return;
    }

    match key.code {
        KeyCode::Char(ch) if !app.streaming_active => {
            app.input.push(ch);
        }
        KeyCode::Backspace if !app.streaming_active => {
            app.input.pop();
        }
        KeyCode::Enter if !app.streaming_active => {
            if let Some(user_msg) = app.push_user_input() {
                app.last_error = None;
                app.streaming_active = true;
                let args = runner::RunArgs {
                    db: app.db.clone(),
                    session_id: app.session_id.clone(),
                    model: app.model.clone(),
                    system_prompt: app.system_prompt.clone(),
                    history: app.history[..app.history.len().saturating_sub(1)].to_vec(),
                    user_turn: user_msg,
                    max_tokens: DEFAULT_MAX_TOKENS,
                };
                let cancel = runner::spawn_completion(provider, args, app.ui_tx.clone());
                app.cancel = Some(cancel);
            }
        }
        _ => {}
    }
}
