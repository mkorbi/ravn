//! Wires the agent loop, the LLM judge, and the report writer.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use ravn_core::{Agent, AgentConfig, Budget, NullSink, RunContext};
use ravn_llm::{LlmProvider, Message};
use ravn_memory::SemanticMemory;
use ravn_persistence::Db;
use ravn_tools::ToolRegistry;
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::judge::{Judge, Judgement};
use crate::task::{EvalTask, ToolSet};
use crate::Error;

#[derive(Debug, Clone, Serialize)]
pub struct TaskOutcome {
    pub id: String,
    pub title: String,
    pub tags: Vec<String>,
    pub final_text: String,
    pub steps: usize,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub duration_ms: u128,
    pub agent_error: Option<String>,
    pub judgement: Option<Judgement>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvalReport {
    pub model: String,
    pub generated_at: String,
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub errored: usize,
    pub mean_score: f64,
    pub mean_cost_usd: f64,
    pub mean_steps: f64,
    pub total_cost_usd: f64,
    pub outcomes: Vec<TaskOutcome>,
}

pub struct EvalRunner {
    pub provider: Arc<dyn LlmProvider>,
    pub judge: Judge,
    pub model: String,
    pub data_dir: PathBuf,
    pub native_tools_dir: PathBuf,
}

impl EvalRunner {
    pub async fn run(self, tasks: Vec<EvalTask>) -> Result<EvalReport, Error> {
        let mut outcomes = Vec::with_capacity(tasks.len());
        for task in tasks {
            tracing::info!(id = %task.id, title = %task.title, "running eval task");
            let outcome = self.run_one(&task).await;
            outcomes.push(outcome);
        }
        Ok(aggregate(self.model.clone(), outcomes))
    }

    async fn run_one(&self, task: &EvalTask) -> TaskOutcome {
        let start = Instant::now();
        // Fresh DB per task so embeddings/sessions don't bleed across.
        let db_path = self.data_dir.join(format!("eval-{}.db", task.id));
        let _ = std::fs::remove_file(&db_path);
        let db = match Db::open(&db_path).await {
            Ok(d) => d,
            Err(e) => {
                return TaskOutcome::errored(task, format!("db open: {e}"), start);
            }
        };
        if let Err(e) = ravn_persistence::sessions::create(
            &db,
            &format!("eval-{}", task.id),
            "eval",
            Some(&self.model),
        )
        .await
        {
            return TaskOutcome::errored(task, format!("session create: {e}"), start);
        }

        let registry = self.build_registry(task);
        let agent = Agent::new(
            self.provider.clone(),
            Arc::new(registry),
            Arc::new(ravn_tools::AllowAll),
            db.clone(),
        );

        let budget = Budget {
            max_steps: task.max_steps.unwrap_or(20),
            max_cost_usd: task.max_cost_usd.unwrap_or(0.50),
            ..Budget::default()
        };
        let config = AgentConfig {
            model: self.model.clone(),
            budget,
            ..AgentConfig::new(self.model.clone())
        };
        let ctx = RunContext {
            session_id: format!("eval-{}", task.id),
            trace_id: format!("eval-trace-{}", task.id),
            semantic: SemanticMemory::default(),
            history: Vec::new(),
            user_turn: Message::user(task.input.clone()),
        };

        let cancel = CancellationToken::new();
        let result = agent
            .run(&config, ctx, Arc::new(NullSink), cancel)
            .await;
        let elapsed = start.elapsed();

        let summary = match result {
            Ok(s) => s,
            Err(e) => {
                return TaskOutcome::errored(task, format!("agent: {e}"), start);
            }
        };

        let judgement = match self
            .judge
            .grade(&task.input, &task.rubric, &summary.final_text)
            .await
        {
            Ok(j) => Some(j),
            Err(e) => {
                tracing::warn!(error = %e, id = %task.id, "judge failed");
                None
            }
        };

        TaskOutcome {
            id: task.id.clone(),
            title: task.title.clone(),
            tags: task.tags.clone(),
            final_text: summary.final_text,
            steps: summary.steps,
            input_tokens: summary.usage.input_tokens,
            output_tokens: summary.usage.output_tokens,
            cost_usd: summary.usage.cost_usd,
            duration_ms: elapsed.as_millis(),
            agent_error: None,
            judgement,
        }
    }

    fn build_registry(&self, task: &EvalTask) -> ToolRegistry {
        let mut reg = ToolRegistry::new();
        match &task.tools {
            ToolSet::None => reg,
            ToolSet::AllNative => {
                ravn_tools::native::register_defaults(
                    &mut reg,
                    self.native_tools_dir.clone(),
                    None,
                );
                reg
            }
            ToolSet::Subset { names } => {
                let mut full = ToolRegistry::new();
                ravn_tools::native::register_defaults(
                    &mut full,
                    self.native_tools_dir.clone(),
                    None,
                );
                for name in names {
                    if let Some(t) = full.get(name) {
                        reg.register_arc(t);
                    } else {
                        tracing::warn!(
                            id = %task.id,
                            missing = %name,
                            "eval task references unknown tool"
                        );
                    }
                }
                reg
            }
        }
    }
}

impl TaskOutcome {
    fn errored(task: &EvalTask, message: String, start: Instant) -> Self {
        Self {
            id: task.id.clone(),
            title: task.title.clone(),
            tags: task.tags.clone(),
            final_text: String::new(),
            steps: 0,
            input_tokens: 0,
            output_tokens: 0,
            cost_usd: 0.0,
            duration_ms: start.elapsed().as_millis(),
            agent_error: Some(message),
            judgement: None,
        }
    }
}

fn aggregate(model: String, outcomes: Vec<TaskOutcome>) -> EvalReport {
    let total = outcomes.len();
    let mut passed = 0;
    let mut failed = 0;
    let mut errored = 0;
    let mut score_sum = 0.0;
    let mut score_count = 0;
    let mut cost_sum = 0.0;
    let mut steps_sum = 0.0;
    for o in &outcomes {
        if o.agent_error.is_some() {
            errored += 1;
            continue;
        }
        match &o.judgement {
            Some(j) if j.pass => {
                passed += 1;
                score_sum += j.score;
                score_count += 1;
            }
            Some(j) => {
                failed += 1;
                score_sum += j.score;
                score_count += 1;
            }
            None => {
                errored += 1;
            }
        }
        cost_sum += o.cost_usd;
        steps_sum += o.steps as f64;
    }
    let scored = score_count.max(1) as f64;
    let runs = (total - errored).max(1) as f64;
    EvalReport {
        model,
        generated_at: chrono::Utc::now().to_rfc3339(),
        total,
        passed,
        failed,
        errored,
        mean_score: score_sum / scored,
        mean_cost_usd: cost_sum / runs,
        mean_steps: steps_sum / runs,
        total_cost_usd: cost_sum,
        outcomes,
    }
}
