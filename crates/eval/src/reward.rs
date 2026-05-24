//! Phase 6.2: reward functions for verifiable outcomes.
//!
//! Deterministic verifiers that score a task's *result* against checkable
//! signals — a test command passing, a commit landing, a file matching — so
//! RL (Phase 6.6+) has a ground-truth reward, not only the LLM judge (D18).
//!
//! [`score`] combines verifiers into one composite reward; [`score_and_record`]
//! also persists it as a `react.reward` event for the run's `trace_id`, which
//! the Phase 6.1 JSONL export then surfaces on the trajectory's terminal step.

use std::path::PathBuf;

use async_trait::async_trait;
use ravn_persistence::Db;

use crate::Error;

/// Outcome of a verifier (or a composite): a reward in `0.0..=1.0`, whether it
/// fully passed, and a short human-readable explanation.
#[derive(Debug, Clone)]
pub struct RewardOutcome {
    pub reward: f64,
    pub passed: bool,
    pub detail: String,
}

impl RewardOutcome {
    pub fn pass(detail: impl Into<String>) -> Self {
        Self { reward: 1.0, passed: true, detail: detail.into() }
    }
    pub fn fail(detail: impl Into<String>) -> Self {
        Self { reward: 0.0, passed: false, detail: detail.into() }
    }
}

#[async_trait]
pub trait Verifier: Send + Sync {
    fn name(&self) -> &str;
    async fn verify(&self) -> RewardOutcome;
}

/// Reward 1.0 iff a test/build command exits 0 in `dir`.
pub struct TestsPass {
    pub dir: PathBuf,
    pub command: Vec<String>,
}

impl TestsPass {
    pub fn new(dir: impl Into<PathBuf>, command: Vec<String>) -> Self {
        Self { dir: dir.into(), command }
    }
}

#[async_trait]
impl Verifier for TestsPass {
    fn name(&self) -> &str {
        "tests_pass"
    }
    async fn verify(&self) -> RewardOutcome {
        let Some((prog, args)) = self.command.split_first() else {
            return RewardOutcome::fail("empty test command");
        };
        let printable = self.command.join(" ");
        match tokio::process::Command::new(prog)
            .args(args)
            .current_dir(&self.dir)
            .output()
            .await
        {
            Ok(out) if out.status.success() => RewardOutcome::pass(format!("`{printable}` exit 0")),
            Ok(out) => RewardOutcome::fail(format!(
                "`{printable}` exit {}",
                out.status.code().unwrap_or(-1)
            )),
            Err(e) => RewardOutcome::fail(format!("`{printable}` spawn failed: {e}")),
        }
    }
}

/// Reward 1.0 iff `dir`'s `HEAD` differs from `base_rev` — i.e. a commit was
/// made since the task started.
pub struct GitCommitted {
    pub dir: PathBuf,
    pub base_rev: String,
}

impl GitCommitted {
    pub fn new(dir: impl Into<PathBuf>, base_rev: impl Into<String>) -> Self {
        Self { dir: dir.into(), base_rev: base_rev.into() }
    }
}

#[async_trait]
impl Verifier for GitCommitted {
    fn name(&self) -> &str {
        "git_committed"
    }
    async fn verify(&self) -> RewardOutcome {
        match git_head(&self.dir).await {
            Some(head) if head != self.base_rev => {
                RewardOutcome::pass(format!("HEAD {} (was {})", short(&head), short(&self.base_rev)))
            }
            Some(_) => RewardOutcome::fail("no new commit since base"),
            None => RewardOutcome::fail("not a git repo / git unavailable"),
        }
    }
}

async fn git_head(dir: &PathBuf) -> Option<String> {
    let out = tokio::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .await
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

fn short(rev: &str) -> &str {
    &rev[..rev.len().min(8)]
}

/// How a file's content must match.
#[derive(Debug, Clone)]
pub enum Match {
    /// Exact content (trailing whitespace ignored).
    Exact(String),
    /// Content must contain this substring.
    Contains(String),
}

/// Reward 1.0 iff the file at `path` matches `expect`.
pub struct FileMatches {
    pub path: PathBuf,
    pub expect: Match,
}

impl FileMatches {
    pub fn new(path: impl Into<PathBuf>, expect: Match) -> Self {
        Self { path: path.into(), expect }
    }
}

#[async_trait]
impl Verifier for FileMatches {
    fn name(&self) -> &str {
        "file_matches"
    }
    async fn verify(&self) -> RewardOutcome {
        let content = match tokio::fs::read_to_string(&self.path).await {
            Ok(c) => c,
            Err(e) => return RewardOutcome::fail(format!("read {}: {e}", self.path.display())),
        };
        match &self.expect {
            Match::Exact(want) if content.trim_end() == want.trim_end() => {
                RewardOutcome::pass("exact match")
            }
            Match::Exact(_) => RewardOutcome::fail("content != expected"),
            Match::Contains(sub) if content.contains(sub) => RewardOutcome::pass("substring found"),
            Match::Contains(_) => RewardOutcome::fail("substring not found"),
        }
    }
}

/// Combine verifiers: reward is the mean of their rewards; `passed` only if all
/// passed. Empty input fails.
pub async fn score(verifiers: &[Box<dyn Verifier>]) -> RewardOutcome {
    if verifiers.is_empty() {
        return RewardOutcome::fail("no verifiers");
    }
    let mut total = 0.0;
    let mut all_passed = true;
    let mut parts = Vec::with_capacity(verifiers.len());
    for v in verifiers {
        let o = v.verify().await;
        total += o.reward;
        all_passed &= o.passed;
        parts.push(format!("{}={:.2} ({})", v.name(), o.reward, o.detail));
    }
    RewardOutcome {
        reward: total / verifiers.len() as f64,
        passed: all_passed,
        detail: parts.join("; "),
    }
}

/// [`score`] the verifiers and persist the result as a `react.reward` event for
/// `trace_id`, so the trajectory export carries the reward.
pub async fn score_and_record(
    db: &Db,
    trace_id: &str,
    session_id: Option<&str>,
    verifiers: &[Box<dyn Verifier>],
) -> Result<RewardOutcome, Error> {
    let outcome = score(verifiers).await;
    ravn_persistence::trajectory::record_reward(
        db,
        trace_id,
        session_id,
        outcome.reward,
        &outcome.detail,
    )
    .await?;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn tests_pass_reflects_exit_code() {
        let ok = TestsPass::new(".", vec!["true".into()]).verify().await;
        assert!(ok.passed && ok.reward == 1.0);
        let bad = TestsPass::new(".", vec!["false".into()]).verify().await;
        assert!(!bad.passed && bad.reward == 0.0);
    }

    #[tokio::test]
    async fn file_matches_exact_and_contains() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.txt");
        tokio::fs::write(&path, "Example Domain\n").await.unwrap();

        assert!(FileMatches::new(&path, Match::Exact("Example Domain".into()))
            .verify()
            .await
            .passed);
        assert!(FileMatches::new(&path, Match::Contains("Domain".into()))
            .verify()
            .await
            .passed);
        assert!(!FileMatches::new(&path, Match::Contains("nope".into()))
            .verify()
            .await
            .passed);
        assert!(!FileMatches::new(dir.path().join("missing.txt"), Match::Contains("x".into()))
            .verify()
            .await
            .passed);
    }

    #[tokio::test]
    async fn git_committed_detects_new_commit() {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .output()
                .unwrap()
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t.io"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(dir.path().join("a.txt"), "1").unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "base"]);
        let base = String::from_utf8_lossy(&git(&["rev-parse", "HEAD"]).stdout)
            .trim()
            .to_string();

        // No new commit yet → fail.
        assert!(!GitCommitted::new(dir.path(), base.clone()).verify().await.passed);

        // Make a commit → pass.
        std::fs::write(dir.path().join("a.txt"), "2").unwrap();
        git(&["commit", "-aqm", "change"]);
        assert!(GitCommitted::new(dir.path(), base).verify().await.passed);
    }

    #[tokio::test]
    async fn composite_means_rewards_and_ands_passed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.txt");
        tokio::fs::write(&path, "hello").await.unwrap();

        let verifiers: Vec<Box<dyn Verifier>> = vec![
            Box::new(TestsPass::new(".", vec!["true".into()])), // 1.0
            Box::new(FileMatches::new(&path, Match::Contains("nope".into()))), // 0.0
        ];
        let outcome = score(&verifiers).await;
        assert!((outcome.reward - 0.5).abs() < 1e-9);
        assert!(!outcome.passed); // one verifier failed
    }
}
