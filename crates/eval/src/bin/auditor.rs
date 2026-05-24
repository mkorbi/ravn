//! `auditor` — constitutional self-audit of recent sessions (Phase 6.9).
//!
//! A nightly-style job (wire into the heartbeat scheduler, Phase 4.10): reviews
//! the last N sessions against `~/.ravn/constitution.md` with an LLM auditor and
//! appends concrete findings to `memory.md`.
//!
//! ```text
//! auditor [--db <path>] [--constitution <path>] [--memory-dir <dir>]
//!         [--limit N] [--dry-run]
//! ```
//! Needs ANTHROPIC_API_KEY or OPENAI_API_KEY. Defaults: paths under ~/.ravn,
//! --limit 100.

use std::path::PathBuf;
use std::sync::Arc;

use ravn_eval::audit::{load_transcripts, write_findings, Auditor, Constitution};
use ravn_llm::anthropic::AnthropicProvider;
use ravn_llm::openai::OpenAiProvider;
use ravn_llm::LlmProvider;
use ravn_persistence::Db;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut db_path: Option<PathBuf> = None;
    let mut constitution_path: Option<PathBuf> = None;
    let mut memory_dir: Option<PathBuf> = None;
    let mut limit: i64 = 100;
    let mut dry_run = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--db" => db_path = args.next().map(PathBuf::from),
            "--constitution" => constitution_path = args.next().map(PathBuf::from),
            "--memory-dir" => memory_dir = args.next().map(PathBuf::from),
            "--limit" => {
                limit = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--limit expects a value"))?
                    .parse()?
            }
            "--dry-run" => dry_run = true,
            "-h" | "--help" => {
                print_help();
                return Ok(());
            }
            other => anyhow::bail!("unknown argument: {other} (try --help)"),
        }
    }

    let data = data_dir()?;
    let db_path = db_path.unwrap_or_else(|| data.join("state.db"));
    let constitution_path = constitution_path.unwrap_or_else(|| data.join("constitution.md"));
    let memory_dir = memory_dir.unwrap_or(data);

    let constitution = Constitution::load(&constitution_path).await.map_err(|e| {
        anyhow::anyhow!(
            "{e}\nWrite your rules to {} first (privacy, tone, safety, …).",
            constitution_path.display()
        )
    })?;

    let db = Db::open(&db_path)
        .await
        .map_err(|e| anyhow::anyhow!("open db at {}: {e}", db_path.display()))?;
    let transcripts = load_transcripts(&db, limit).await?;
    if transcripts.is_empty() {
        eprintln!("no sessions to audit");
        return Ok(());
    }

    let auditor = Auditor::new(select_provider()?);
    let findings = auditor.audit(&constitution, &transcripts).await?;

    eprintln!(
        "audited {} session(s) → {} finding(s)",
        transcripts.len(),
        findings.len()
    );
    for f in &findings {
        println!("[{}] {} — {} (session {})", f.severity, f.principle, f.finding, f.session_id);
    }

    if dry_run {
        eprintln!("dry run — nothing written to memory");
    } else if !findings.is_empty() {
        write_findings(&memory_dir, &findings).await?;
        eprintln!("appended findings to {}", memory_dir.join("memory.md").display());
    }
    Ok(())
}

fn select_provider() -> anyhow::Result<Arc<dyn LlmProvider>> {
    if std::env::var("ANTHROPIC_API_KEY").is_ok() {
        Ok(Arc::new(
            AnthropicProvider::from_env().map_err(|e| anyhow::anyhow!("anthropic init: {e}"))?,
        ))
    } else if std::env::var("OPENAI_API_KEY").is_ok() {
        Ok(Arc::new(
            OpenAiProvider::from_env().map_err(|e| anyhow::anyhow!("openai init: {e}"))?,
        ))
    } else {
        anyhow::bail!("set ANTHROPIC_API_KEY or OPENAI_API_KEY")
    }
}

fn data_dir() -> anyhow::Result<PathBuf> {
    let dir = dirs::data_local_dir()
        .or_else(|| std::env::var("HOME").ok().map(PathBuf::from))
        .ok_or_else(|| anyhow::anyhow!("cannot determine data directory; pass --db / --memory-dir"))?;
    Ok(dir.join("ravn"))
}

fn print_help() {
    eprintln!(
        "auditor — constitutional self-audit of recent sessions (Phase 6.9)\n\n\
         USAGE:\n  auditor [--db <path>] [--constitution <path>] [--memory-dir <dir>] \\\n\
         \x20       [--limit N] [--dry-run]\n\n\
         Reviews the last N sessions against constitution.md and appends findings\n\
         to memory.md. Needs ANTHROPIC_API_KEY or OPENAI_API_KEY. --limit default 100."
    );
}
