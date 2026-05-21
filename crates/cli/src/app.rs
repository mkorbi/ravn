//! Application state + event reducer for the ratatui TUI.

use ravn_core::{LoopEvent, RunSummary};
use ravn_llm::{Message, Role};
use ravn_memory::SemanticMemory;
use ravn_persistence::Db;
use ravn_tools::Permission;
use tokio_util::sync::CancellationToken;

use crate::approver::ApprovalRequest;
use crate::input::InputBuffer;

#[derive(Debug, Clone)]
pub struct DisplayedMessage {
    pub role: DisplayRole,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayRole {
    User,
    Assistant,
    /// Tool invocation header line — rendered dim.
    ToolStart,
    /// Tool result excerpt — rendered dim.
    ToolEnd,
    /// User denied a tool / approval-related notice.
    Notice,
    /// Startup splash (raven + version + URL + slash-commands).
    /// `text` is unused; rendering is delegated to [`crate::splash`].
    Splash,
}

/// One central event type for the cli — bundles loop events from the
/// agent, approval requests from the [`crate::approver::TuiApprover`],
/// and the spawned-task completion signal.
pub enum AppEvent {
    Loop(LoopEvent),
    Approval(ApprovalRequest),
    RunDone { summary: RunSummary },
    RunError { message: String },
}

pub struct App {
    pub db: Db,
    pub session_id: String,
    pub model: String,

    /// Canonical conversation history sent back to the model on the
    /// next user turn. Updated from `RunSummary::history` after every
    /// `Agent::run` finishes.
    pub history: Vec<Message>,
    pub semantic: SemanticMemory,
    pub system_prompt: String,

    /// What gets rendered in the scrollback pane.
    pub messages: Vec<DisplayedMessage>,
    pub input: InputBuffer,
    pub streaming_buffer: String,
    pub streaming_active: bool,
    pub pending_approval: Option<ApprovalRequest>,
    pub last_error: Option<String>,

    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub reasoning_tokens: u64,
    pub cost_usd: f64,

    pub should_quit: bool,
    pub cancel: Option<CancellationToken>,
}

impl App {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db: Db,
        session_id: String,
        model: String,
        system_prompt: String,
        semantic: SemanticMemory,
    ) -> Self {
        Self {
            db,
            session_id,
            model,
            history: Vec::new(),
            semantic,
            system_prompt,
            messages: vec![DisplayedMessage {
                role: DisplayRole::Splash,
                text: String::new(),
            }],
            input: InputBuffer::new(),
            streaming_buffer: String::new(),
            streaming_active: false,
            pending_approval: None,
            last_error: None,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            reasoning_tokens: 0,
            cost_usd: 0.0,
            should_quit: false,
            cancel: None,
        }
    }

    /// Compute Anthropic-style cache hit rate (Phase 1.8).
    /// Returns `None` if no input tokens have been seen yet.
    pub fn cache_hit_rate(&self) -> Option<f64> {
        let denom = self.input_tokens + self.cache_read_tokens + self.cache_creation_tokens;
        if denom == 0 {
            None
        } else {
            Some(self.cache_read_tokens as f64 / denom as f64)
        }
    }

    pub fn push_user_input(&mut self) -> Option<Message> {
        let text = self.input.take();
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return None;
        }
        let owned = trimmed.to_string();
        self.messages.push(DisplayedMessage {
            role: DisplayRole::User,
            text: owned.clone(),
        });
        Some(Message::user(owned))
    }

    pub fn apply(&mut self, event: AppEvent) {
        match event {
            AppEvent::Loop(le) => self.apply_loop(le),
            AppEvent::Approval(req) => {
                self.pending_approval = Some(req);
            }
            AppEvent::RunDone { summary } => {
                if !self.streaming_buffer.is_empty() {
                    self.flush_streaming();
                }
                self.history = summary.history;
                self.streaming_active = false;
                self.cancel = None;
            }
            AppEvent::RunError { message } => {
                self.last_error = Some(message);
                if !self.streaming_buffer.is_empty() {
                    self.flush_streaming();
                }
                self.streaming_active = false;
                self.cancel = None;
            }
        }
    }

    fn apply_loop(&mut self, le: LoopEvent) {
        match le {
            LoopEvent::StepStart { .. } => {}
            LoopEvent::TextDelta(t) => self.streaming_buffer.push_str(&t),
            LoopEvent::ThinkingDelta(_) => {
                // Phase 1 drops thinking deltas from the scrollback.
            }
            LoopEvent::ToolStart {
                name,
                args,
                permission,
                ..
            } => {
                self.flush_streaming();
                let icon = match permission {
                    Permission::Read => "🔎",
                    Permission::Write => "✎",
                    Permission::Exec => "⚙",
                };
                let args_str = serde_json::to_string(&args).unwrap_or_default();
                let args_str = if args_str.len() > 120 {
                    format!("{}…", &args_str[..120])
                } else {
                    args_str
                };
                self.messages.push(DisplayedMessage {
                    role: DisplayRole::ToolStart,
                    text: format!("{icon} {name} {args_str}"),
                });
            }
            LoopEvent::ToolEnd {
                name,
                content,
                is_error,
                ..
            } => {
                let marker = if is_error { "✗" } else { "✓" };
                let preview = content
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .chars()
                    .take(140)
                    .collect::<String>();
                self.messages.push(DisplayedMessage {
                    role: DisplayRole::ToolEnd,
                    text: format!("  {marker} {name}: {preview}"),
                });
            }
            LoopEvent::ToolDenied { name, .. } => {
                self.messages.push(DisplayedMessage {
                    role: DisplayRole::Notice,
                    text: format!("denied: {name}"),
                });
            }
            LoopEvent::Usage(u) => {
                self.input_tokens += u.input_tokens as u64;
                self.output_tokens += u.output_tokens as u64;
                self.cache_read_tokens += u.cache_read_input_tokens as u64;
                self.cache_creation_tokens += u.cache_creation_input_tokens as u64;
                self.reasoning_tokens += u.reasoning_tokens as u64;
                self.cost_usd += ravn_llm::pricing::cost(&self.model, &u);

                // Phase 1.8 / D10: warn once meaningful sample size is
                // reached if Anthropic cache hit-rate is below 60%.
                const CACHE_WARN_THRESHOLD: f64 = 0.60;
                const CACHE_WARN_MIN_SAMPLE: u64 = 5_000;
                let denom =
                    self.input_tokens + self.cache_read_tokens + self.cache_creation_tokens;
                if denom > CACHE_WARN_MIN_SAMPLE {
                    let rate = self.cache_read_tokens as f64 / denom as f64;
                    if rate < CACHE_WARN_THRESHOLD {
                        tracing::warn!(
                            session = %self.session_id,
                            hit_rate = format!("{:.0}%", rate * 100.0),
                            input = self.input_tokens,
                            cache_read = self.cache_read_tokens,
                            cache_creation = self.cache_creation_tokens,
                            "anthropic cache hit-rate below 60% — review system-prompt stability"
                        );
                    }
                }

                // Persist into the session row.
                let db = self.db.clone();
                let session_id = self.session_id.clone();
                let model = self.model.clone();
                let usage = u;
                tokio::spawn(async move {
                    let delta = ravn_persistence::UsageDelta {
                        input_tokens: usage.input_tokens,
                        output_tokens: usage.output_tokens,
                        cache_read_tokens: usage.cache_read_input_tokens,
                        cache_creation_tokens: usage.cache_creation_input_tokens,
                        reasoning_tokens: usage.reasoning_tokens,
                        cost_usd: ravn_llm::pricing::cost(&model, &usage),
                    };
                    if let Err(e) = ravn_persistence::sessions::bump_usage(
                        &db,
                        &session_id,
                        delta,
                    )
                    .await
                    {
                        tracing::warn!(error = %e, "bump_usage");
                    }
                });
            }
            LoopEvent::BudgetExceeded { reason } => {
                self.flush_streaming();
                self.messages.push(DisplayedMessage {
                    role: DisplayRole::Notice,
                    text: format!("budget exceeded: {reason}"),
                });
            }
            LoopEvent::Done => {
                self.flush_streaming();
            }
            LoopEvent::Error(e) => {
                self.last_error = Some(e);
                self.flush_streaming();
            }
        }
    }

    fn flush_streaming(&mut self) {
        if self.streaming_buffer.is_empty() {
            return;
        }
        let text = std::mem::take(&mut self.streaming_buffer);
        self.messages.push(DisplayedMessage {
            role: DisplayRole::Assistant,
            text,
        });
    }
}

impl From<Role> for DisplayRole {
    fn from(r: Role) -> Self {
        match r {
            Role::User => DisplayRole::User,
            Role::Assistant => DisplayRole::Assistant,
            _ => DisplayRole::Notice,
        }
    }
}

