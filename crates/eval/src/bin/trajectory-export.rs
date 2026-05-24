//! `trajectory-export` — dump logged ReAct trajectories as JSONL (Phase 6.1).
//!
//! Reads `react.step` events from the ravn SQLite DB and writes one
//! `{trace_id, step, thought, action, observation, reward?}` record per line —
//! the export path for RL tooling (Phase 6.6+).
//!
//! ```text
//! trajectory-export [--db <path>] [--session <id>] [--trace <id>] [--out <path>]
//! ```
//! Defaults: `--db` is `~/.ravn/state.db` (platform data dir); output to stdout.

use std::path::PathBuf;

use ravn_persistence::trajectory::{export_jsonl, Filter};
use ravn_persistence::Db;
use tokio::io::AsyncWriteExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut filter = Filter::default();
    let mut db_path: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--db" => db_path = args.next().map(PathBuf::from),
            "--session" => filter.session_id = args.next(),
            "--trace" => filter.trace_id = args.next(),
            "--out" => out = args.next().map(PathBuf::from),
            "-h" | "--help" => {
                print_help();
                return Ok(());
            }
            other => anyhow::bail!("unknown argument: {other} (try --help)"),
        }
    }

    let db_path = match db_path {
        Some(p) => p,
        None => default_db_path()?,
    };
    let db = Db::open(&db_path)
        .await
        .map_err(|e| anyhow::anyhow!("open db at {}: {e}", db_path.display()))?;

    let jsonl = export_jsonl(&db, &filter).await?;
    match out {
        Some(path) => {
            tokio::fs::write(&path, jsonl.as_bytes()).await?;
            eprintln!(
                "wrote {} trajectory step(s) to {}",
                jsonl.lines().count(),
                path.display()
            );
        }
        None => tokio::io::stdout().write_all(jsonl.as_bytes()).await?,
    }
    Ok(())
}

fn default_db_path() -> anyhow::Result<PathBuf> {
    let dir = dirs::data_local_dir()
        .or_else(|| std::env::var("HOME").ok().map(PathBuf::from))
        .ok_or_else(|| anyhow::anyhow!("cannot determine data directory; pass --db <path>"))?;
    Ok(dir.join("ravn").join("state.db"))
}

fn print_help() {
    eprintln!(
        "trajectory-export — export ReAct trajectories as JSONL (Phase 6.1)\n\n\
         USAGE:\n  trajectory-export [--db <path>] [--session <id>] [--trace <id>] [--out <path>]\n\n\
         Defaults: --db ~/.ravn/state.db (platform data dir); output to stdout."
    );
}
