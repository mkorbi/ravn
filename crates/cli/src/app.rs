//! Application state + event handling for the ratatui TUI.

use std::sync::Arc;

use ravn_llm::{Message, Role, Usage};
use ravn_persistence::{sessions, Db, UsageDelta};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// A single message as it should appear in the scrollback pane.
#[derive(Debug, Clone)]
pub struct DisplayedMessage {
    pub role: Role,
    pub text: String,
}

/// Messages sent from the LLM-call task back to the UI.
#[derive(Debug)]
pub enum UiUpdate {
    /// Delta of visible assistant text.
    TextDelta(String),
    /// Extended Thinking delta. We drop it from the visible scrollback but
    /// log it via tracing for inspection in `ravn.log`.
    ThinkingDelta(#[allow(dead_code)] String),
    /// Final usage report from the provider.
    Usage(Usage),
    /// The stream ended successfully.
    Done,
    /// The stream failed.
    Error(String),
}

pub struct App {
    pub db: Db,
    pub session_id: String,
    pub model: String,
    /// Visible scrollback (oldest first).
    pub messages: Vec<DisplayedMessage>,
    /// Full conversation history sent back to the model.
    pub history: Vec<Message>,
    pub system_prompt: String,
    /// Current user input buffer.
    pub input: String,
    /// Partial assistant response currently streaming in.
    pub streaming_buffer: String,
    /// True while a completion request is in-flight.
    pub streaming_active: bool,
    pub last_error: Option<String>,
    pub cost_usd: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub reasoning_tokens: u64,
    pub should_quit: bool,
    /// Cancellation handle for the currently-running stream task.
    pub cancel: Option<CancellationToken>,
    /// Sender that the LLM task uses to push updates back to the UI. Cloned
    /// per request.
    pub ui_tx: Arc<mpsc::Sender<UiUpdate>>,
}

impl App {
    pub fn new(
        db: Db,
        session_id: String,
        model: String,
        system_prompt: String,
        ui_tx: mpsc::Sender<UiUpdate>,
    ) -> Self {
        Self {
            db,
            session_id,
            model,
            messages: Vec::new(),
            history: Vec::new(),
            system_prompt,
            input: String::new(),
            streaming_buffer: String::new(),
            streaming_active: false,
            last_error: None,
            cost_usd: 0.0,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            reasoning_tokens: 0,
            should_quit: false,
            cancel: None,
            ui_tx: Arc::new(ui_tx),
        }
    }

    pub fn push_user_input(&mut self) -> Option<Message> {
        let text = std::mem::take(&mut self.input);
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return None;
        }
        let owned = trimmed.to_string();
        self.messages.push(DisplayedMessage {
            role: Role::User,
            text: owned.clone(),
        });
        let msg = Message::user(owned);
        self.history.push(msg.clone());
        Some(msg)
    }

    pub fn apply_chunk(&mut self, chunk: UiUpdate) {
        match chunk {
            UiUpdate::TextDelta(t) => {
                self.streaming_buffer.push_str(&t);
            }
            UiUpdate::ThinkingDelta(_) => {
                // Phase 0: hide thinking deltas from scrollback.
            }
            UiUpdate::Usage(u) => {
                self.input_tokens += u.input_tokens as u64;
                self.output_tokens += u.output_tokens as u64;
                self.cache_read_tokens += u.cache_read_input_tokens as u64;
                self.reasoning_tokens += u.reasoning_tokens as u64;
                let delta_cost = ravn_llm::pricing::cost(&self.model, &u);
                self.cost_usd += delta_cost;

                // Persist incremental usage into the session row.
                let db = self.db.clone();
                let session_id = self.session_id.clone();
                let delta = UsageDelta {
                    input_tokens: u.input_tokens,
                    output_tokens: u.output_tokens,
                    cache_read_tokens: u.cache_read_input_tokens,
                    cache_creation_tokens: u.cache_creation_input_tokens,
                    reasoning_tokens: u.reasoning_tokens,
                    cost_usd: delta_cost,
                };
                tokio::spawn(async move {
                    if let Err(e) = sessions::bump_usage(&db, &session_id, delta).await {
                        tracing::warn!(error = %e, "failed to bump session usage");
                    }
                });
            }
            UiUpdate::Done => {
                let final_text = std::mem::take(&mut self.streaming_buffer);
                if !final_text.is_empty() {
                    self.messages.push(DisplayedMessage {
                        role: Role::Assistant,
                        text: final_text.clone(),
                    });
                    self.history.push(Message::assistant(final_text));
                }
                self.streaming_active = false;
                self.cancel = None;
            }
            UiUpdate::Error(e) => {
                self.last_error = Some(e);
                self.streaming_active = false;
                self.cancel = None;
            }
        }
    }
}
