//! `curator` — mine logged trajectories into SKILL.md candidates (Phase 6.3).
//!
//! A nightly-style job (wire it into the heartbeat scheduler, Phase 4.10): it
//! reads `react.step` trajectories, finds recurring tool sequences, and writes
//! each as a candidate `SKILL.md` under a staging directory. Candidates are
//! proposals only — verify (6.4) before promoting into the live skills set.
//!
//! ```text
//! curator [--db <path>] [--out-dir <dir>] [--min-support N] [--min-reward R]
//!         [--min-len N] [--max-len N] [--max-candidates N] [--dry-run]
//! ```
//! Defaults: `--db` `~/.ravn/state.db`; `--out-dir` `~/.ravn/skill-candidates`
//! (kept out of `~/.ravn/skills` so candidates aren't auto-loaded).

use std::path::PathBuf;

use ravn_eval::curator::{mine, render_skill_md, CuratorConfig};
use ravn_persistence::trajectory::{self, Filter};
use ravn_persistence::Db;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut cfg = CuratorConfig::default();
    let mut db_path: Option<PathBuf> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut dry_run = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--db" => db_path = args.next().map(PathBuf::from),
            "--out-dir" => out_dir = args.next().map(PathBuf::from),
            "--min-support" => cfg.min_support = parse_next(&mut args, "--min-support")?,
            "--min-reward" => cfg.min_reward = parse_next(&mut args, "--min-reward")?,
            "--min-len" => cfg.min_len = parse_next(&mut args, "--min-len")?,
            "--max-len" => cfg.max_len = parse_next(&mut args, "--max-len")?,
            "--max-candidates" => cfg.max_candidates = parse_next(&mut args, "--max-candidates")?,
            "--dry-run" => dry_run = true,
            "-h" | "--help" => {
                print_help();
                return Ok(());
            }
            other => anyhow::bail!("unknown argument: {other} (try --help)"),
        }
    }

    let db_path = match db_path {
        Some(p) => p,
        None => default_data_dir()?.join("state.db"),
    };
    let out_dir = match out_dir {
        Some(p) => p,
        None => default_data_dir()?.join("skill-candidates"),
    };

    let db = Db::open(&db_path)
        .await
        .map_err(|e| anyhow::anyhow!("open db at {}: {e}", db_path.display()))?;
    let steps = trajectory::load(&db, &Filter::default()).await?;
    let candidates = mine(&steps, &cfg);

    if candidates.is_empty() {
        eprintln!(
            "no skill candidates (min_support={}, min_reward={}); {} trajectory step(s) scanned",
            cfg.min_support,
            cfg.min_reward,
            steps.len()
        );
        return Ok(());
    }

    for c in &candidates {
        if dry_run {
            println!("{}  (support {})  {}", c.name, c.support, c.sequence.join(" → "));
            continue;
        }
        let dir = out_dir.join(&c.name);
        tokio::fs::create_dir_all(&dir).await?;
        tokio::fs::write(dir.join("SKILL.md"), render_skill_md(c)).await?;
    }

    if dry_run {
        eprintln!("{} candidate(s) (dry run — nothing written)", candidates.len());
    } else {
        eprintln!(
            "wrote {} candidate(s) to {}",
            candidates.len(),
            out_dir.display()
        );
    }
    Ok(())
}

fn parse_next<T: std::str::FromStr>(
    args: &mut impl Iterator<Item = String>,
    flag: &str,
) -> anyhow::Result<T>
where
    T::Err: std::fmt::Display,
{
    let raw = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("{flag} expects a value"))?;
    raw.parse()
        .map_err(|e| anyhow::anyhow!("{flag}: invalid value `{raw}`: {e}"))
}

fn default_data_dir() -> anyhow::Result<PathBuf> {
    let dir = dirs::data_local_dir()
        .or_else(|| std::env::var("HOME").ok().map(PathBuf::from))
        .ok_or_else(|| anyhow::anyhow!("cannot determine data directory; pass --db / --out-dir"))?;
    Ok(dir.join("ravn"))
}

fn print_help() {
    eprintln!(
        "curator — mine trajectories into SKILL.md candidates (Phase 6.3)\n\n\
         USAGE:\n  curator [--db <path>] [--out-dir <dir>] [--min-support N] [--min-reward R]\n\
         \x20         [--min-len N] [--max-len N] [--max-candidates N] [--dry-run]\n\n\
         Defaults: --db ~/.ravn/state.db, --out-dir ~/.ravn/skill-candidates,\n\
         --min-support 2, --min-reward 0.0, --min-len 2, --max-len 4, --max-candidates 20.\n\
         Raise --min-reward (e.g. 1.0) to mine only verified-successful runs."
    );
}
