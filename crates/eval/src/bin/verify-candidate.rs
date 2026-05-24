//! `verify-candidate` — gate a curator candidate on pass-rate (Phase 6.4).
//!
//! Promotes `~/.ravn/skill-candidates/<name>` into the live skills set only if
//! the candidate doesn't regress the historical pass-rate. Pass-rates are
//! measured out-of-band — run the eval set without, then with, the candidate
//! synced — and supplied here:
//!
//! ```text
//! verify-candidate --name <name> --baseline <rate> --candidate <rate> \
//!                  [--min-improvement R] [--candidates-dir D] [--skills-dir D]
//! ```
//! Defaults: dirs under `~/.ravn`; `--min-improvement 0.0` (must not regress).

use std::path::PathBuf;

use ravn_eval::synthesis::{verify_with_rates, Decision};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut name: Option<String> = None;
    let mut baseline: Option<f64> = None;
    let mut candidate: Option<f64> = None;
    let mut min_improvement = 0.0_f64;
    let mut candidates_dir: Option<PathBuf> = None;
    let mut skills_dir: Option<PathBuf> = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--name" => name = args.next(),
            "--baseline" => baseline = Some(parse_next(&mut args, "--baseline")?),
            "--candidate" => candidate = Some(parse_next(&mut args, "--candidate")?),
            "--min-improvement" => min_improvement = parse_next(&mut args, "--min-improvement")?,
            "--candidates-dir" => candidates_dir = args.next().map(PathBuf::from),
            "--skills-dir" => skills_dir = args.next().map(PathBuf::from),
            "-h" | "--help" => {
                print_help();
                return Ok(());
            }
            other => anyhow::bail!("unknown argument: {other} (try --help)"),
        }
    }

    let name = name.ok_or_else(|| anyhow::anyhow!("--name is required"))?;
    let baseline = baseline.ok_or_else(|| anyhow::anyhow!("--baseline is required"))?;
    let candidate = candidate.ok_or_else(|| anyhow::anyhow!("--candidate is required"))?;
    let candidates_dir = match candidates_dir {
        Some(p) => p,
        None => data_dir()?.join("skill-candidates"),
    };
    let skills_dir = match skills_dir {
        Some(p) => p,
        None => data_dir()?.join("skills"),
    };

    let report =
        verify_with_rates(&name, baseline, candidate, &candidates_dir, &skills_dir, min_improvement)
            .await?;

    println!("{}", report.reason);
    match report.decision {
        Decision::Promote => println!(
            "promoted `{name}` → {}",
            skills_dir.join(&name).display()
        ),
        Decision::Reject => println!("left `{name}` in {}", candidates_dir.join(&name).display()),
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
    let raw = args.next().ok_or_else(|| anyhow::anyhow!("{flag} expects a value"))?;
    raw.parse()
        .map_err(|e| anyhow::anyhow!("{flag}: invalid value `{raw}`: {e}"))
}

fn data_dir() -> anyhow::Result<PathBuf> {
    let dir = dirs::data_local_dir()
        .or_else(|| std::env::var("HOME").ok().map(PathBuf::from))
        .ok_or_else(|| anyhow::anyhow!("cannot determine data directory; pass --candidates-dir / --skills-dir"))?;
    Ok(dir.join("ravn"))
}

fn print_help() {
    eprintln!(
        "verify-candidate — promote a curator candidate on pass-rate (Phase 6.4)\n\n\
         USAGE:\n  verify-candidate --name <name> --baseline <rate> --candidate <rate> \\\n\
         \x20                  [--min-improvement R] [--candidates-dir D] [--skills-dir D]\n\n\
         Measure <rate>s by running the eval set without, then with, the candidate\n\
         skill synced. Promotes only if candidate >= baseline + min-improvement.\n\
         Defaults: dirs under ~/.ravn; --min-improvement 0.0 (must not regress)."
    );
}
