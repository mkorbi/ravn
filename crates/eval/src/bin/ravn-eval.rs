//! `ravn-eval` — run the eval-set and print a JSON report.
//!
//! Usage:
//! ```bash
//! export ANTHROPIC_API_KEY=sk-ant-...
//! cargo run --release -p ravn-eval -- [--tasks <DIR>] [--out <FILE>]
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use ravn_eval::{
    judge::Judge,
    runner::EvalRunner,
    task::{default_tasks_dir, EvalTask},
};
use ravn_llm::{anthropic::AnthropicProvider, LlmProvider};
use tracing_subscriber::EnvFilter;

const DEFAULT_MODEL: &str = "claude-sonnet-4-6";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let args = parse_args()?;
    let tasks = EvalTask::load_all(&args.tasks_dir)
        .await
        .with_context(|| format!("load tasks from {}", args.tasks_dir.display()))?;
    if tasks.is_empty() {
        anyhow::bail!("no tasks found in {}", args.tasks_dir.display());
    }
    tracing::info!(
        count = tasks.len(),
        dir = %args.tasks_dir.display(),
        "loaded eval tasks"
    );

    let provider: Arc<dyn LlmProvider> = Arc::new(
        AnthropicProvider::from_env()
            .map_err(|e| anyhow::anyhow!("anthropic init: {e}"))?,
    );
    let judge = Judge::new(provider.clone());

    let data_dir = std::env::temp_dir().join("ravn-eval-data");
    let _ = std::fs::create_dir_all(&data_dir);
    let native_tools_dir = data_dir.join("memory");
    let _ = std::fs::create_dir_all(&native_tools_dir);

    let runner = EvalRunner {
        provider,
        judge,
        model: args.model,
        data_dir,
        native_tools_dir,
    };
    let report = runner.run(tasks).await?;

    let json = serde_json::to_string_pretty(&report)?;
    if let Some(out) = &args.out_path {
        tokio::fs::write(out, &json)
            .await
            .with_context(|| format!("write report to {}", out.display()))?;
        tracing::info!(out = %out.display(), "report written");
    }
    println!("{json}");
    print_summary(&report);
    Ok(())
}

struct Args {
    tasks_dir: PathBuf,
    out_path: Option<PathBuf>,
    model: String,
}

fn parse_args() -> anyhow::Result<Args> {
    let mut tasks_dir = default_tasks_dir();
    let mut out_path = None;
    let mut model = DEFAULT_MODEL.to_string();

    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--tasks" => {
                tasks_dir = iter
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--tasks needs a path"))?
                    .into();
            }
            "--out" => {
                out_path = Some(
                    iter.next()
                        .ok_or_else(|| anyhow::anyhow!("--out needs a path"))?
                        .into(),
                );
            }
            "--model" => {
                model = iter
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--model needs a value"))?;
            }
            "-h" | "--help" => {
                println!(
                    "ravn-eval [--tasks <DIR>] [--out <FILE>] [--model <NAME>]\n\
                     \n\
                     Runs the hand-crafted eval-set in <DIR> (default: crates/eval/tasks/),\n\
                     grades each task with Sonnet-as-Judge, and emits a JSON report on stdout."
                );
                std::process::exit(0);
            }
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }
    Ok(Args {
        tasks_dir,
        out_path,
        model,
    })
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,ravn=info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

fn print_summary(report: &ravn_eval::EvalReport) {
    eprintln!();
    eprintln!("eval summary  model={}", report.model);
    eprintln!(
        "  passed:  {}/{}    failed: {}    errored: {}",
        report.passed, report.total, report.failed, report.errored
    );
    eprintln!(
        "  mean score: {:.2}    mean cost: ${:.4}    mean steps: {:.1}",
        report.mean_score, report.mean_cost_usd, report.mean_steps
    );
    eprintln!("  total cost: ${:.4}", report.total_cost_usd);
}
