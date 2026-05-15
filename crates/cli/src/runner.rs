//! Bridge between `ravn_core::Agent::run` and the cli's UI channel.
//!
//! Spawns a background tokio task per user turn. The task drives the
//! ReAct loop, forwards every [`LoopEvent`] through [`CliSink`] into
//! the cli's [`AppEvent`] channel, and finally sends `RunDone` /
//! `RunError` so the UI can clear streaming state and absorb the new
//! history.

use std::sync::Arc;

use async_trait::async_trait;
use ravn_core::{Agent, AgentConfig, EventSink, LoopEvent, RunContext};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::app::AppEvent;

pub fn spawn_run(
    agent: Arc<Agent>,
    config: AgentConfig,
    ctx: RunContext,
    tx: mpsc::Sender<AppEvent>,
    cancel: CancellationToken,
) {
    tokio::spawn(async move {
        let sink: Arc<dyn EventSink> = Arc::new(CliSink { tx: tx.clone() });
        match agent.run(&config, ctx, sink, cancel).await {
            Ok(summary) => {
                let _ = tx.send(AppEvent::RunDone { summary }).await;
            }
            Err(e) => {
                let _ = tx
                    .send(AppEvent::RunError {
                        message: e.to_string(),
                    })
                    .await;
            }
        }
    });
}

struct CliSink {
    tx: mpsc::Sender<AppEvent>,
}

#[async_trait]
impl EventSink for CliSink {
    async fn emit(&self, event: LoopEvent) {
        let _ = self.tx.send(AppEvent::Loop(event)).await;
    }
}
