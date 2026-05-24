//! Phase 6.4: skill-synthesis verification.
//!
//! Gates the curator's candidates (Phase 6.3): a candidate is promoted into the
//! live skills set only if it does not *regress* the pass-rate on the historical
//! task set — "merge only on a pass-rate improvement" (configurable margin).
//!
//! Pass-rate measurement runs the eval set through the agent, which needs an
//! LLM — so it's injected via [`PassRateMeasurer`] rather than hard-wired here.
//! The decision gate ([`decide`]) and the atomic [`promote`] step are pure and
//! tested directly; the `verify-candidate` bin can also act on rates measured
//! out-of-band (run the eval set with and without the candidate).

use std::path::{Path, PathBuf};

use async_trait::async_trait;

use crate::Error;

/// Negligible-difference epsilon for pass-rate comparison.
const EPS: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Promote,
    Reject,
}

#[derive(Debug, Clone)]
pub struct VerificationReport {
    pub name: String,
    pub baseline: f64,
    pub candidate: f64,
    pub min_improvement: f64,
    pub decision: Decision,
    pub reason: String,
}

/// Promote iff the candidate pass-rate is at least `baseline + min_improvement`.
/// With `min_improvement = 0.0` this means "must not regress"; a positive margin
/// requires a real gain.
pub fn decide(baseline: f64, candidate: f64, min_improvement: f64) -> Decision {
    if candidate + EPS >= baseline + min_improvement {
        Decision::Promote
    } else {
        Decision::Reject
    }
}

fn reason(baseline: f64, candidate: f64, min_improvement: f64, decision: Decision) -> String {
    let delta = candidate - baseline;
    match decision {
        Decision::Promote => format!(
            "promote: pass-rate {baseline:.3} → {candidate:.3} (Δ{delta:+.3} ≥ margin {min_improvement:.3})"
        ),
        Decision::Reject => format!(
            "reject: pass-rate {baseline:.3} → {candidate:.3} (Δ{delta:+.3} < margin {min_improvement:.3})"
        ),
    }
}

/// Atomically move candidate dir `<candidates>/<name>` into `<skills>/<name>`.
/// Refuses to clobber an existing live skill (the rename is atomic, so the live
/// set is never left half-updated). Returns the promoted path.
pub async fn promote(candidate_dir: &Path, skills_dir: &Path) -> Result<PathBuf, Error> {
    if !candidate_dir.is_dir() {
        return Err(Error::Synthesis(format!(
            "candidate dir not found: {}",
            candidate_dir.display()
        )));
    }
    let name = candidate_dir
        .file_name()
        .ok_or_else(|| Error::Synthesis("candidate dir has no name".into()))?;
    let target = skills_dir.join(name);
    if target.exists() {
        return Err(Error::Synthesis(format!(
            "live skill already exists, refusing to clobber: {}",
            target.display()
        )));
    }
    tokio::fs::create_dir_all(skills_dir)
        .await
        .map_err(|e| Error::Io(format!("create {}: {e}", skills_dir.display())))?;
    tokio::fs::rename(candidate_dir, &target)
        .await
        .map_err(|e| Error::Io(format!("promote {} → {}: {e}", candidate_dir.display(), target.display())))?;
    Ok(target)
}

/// Measures pass-rate over the historical task set, optionally with a candidate
/// skill made available to the agent. Implemented against the eval runner (LLM)
/// in production; mocked in tests.
#[async_trait]
pub trait PassRateMeasurer: Send + Sync {
    async fn pass_rate(&self, candidate_skill: Option<&Path>) -> Result<f64, Error>;
}

/// Measure baseline vs. with-candidate pass-rate, decide, and promote on a
/// `Promote` decision. The single source of truth for automated verification.
pub async fn verify_and_promote<M: PassRateMeasurer>(
    measurer: &M,
    name: &str,
    candidates_dir: &Path,
    skills_dir: &Path,
    min_improvement: f64,
) -> Result<VerificationReport, Error> {
    let candidate_dir = candidates_dir.join(name);
    if !candidate_dir.is_dir() {
        return Err(Error::Synthesis(format!(
            "no candidate named {name} in {}",
            candidates_dir.display()
        )));
    }

    let baseline = measurer.pass_rate(None).await?;
    let candidate = measurer.pass_rate(Some(&candidate_dir)).await?;
    let decision = decide(baseline, candidate, min_improvement);

    if decision == Decision::Promote {
        promote(&candidate_dir, skills_dir).await?;
    }

    Ok(VerificationReport {
        name: name.to_string(),
        baseline,
        candidate,
        min_improvement,
        decision,
        reason: reason(baseline, candidate, min_improvement, decision),
    })
}

/// Build a [`VerificationReport`] from pass-rates measured out-of-band, and
/// promote on `Promote`. Used by the `verify-candidate` bin.
pub async fn verify_with_rates(
    name: &str,
    baseline: f64,
    candidate: f64,
    candidates_dir: &Path,
    skills_dir: &Path,
    min_improvement: f64,
) -> Result<VerificationReport, Error> {
    let candidate_dir = candidates_dir.join(name);
    let decision = decide(baseline, candidate, min_improvement);
    if decision == Decision::Promote {
        promote(&candidate_dir, skills_dir).await?;
    }
    Ok(VerificationReport {
        name: name.to_string(),
        baseline,
        candidate,
        min_improvement,
        decision,
        reason: reason(baseline, candidate, min_improvement, decision),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn decide_promotes_only_on_sufficient_gain() {
        assert_eq!(decide(0.6, 0.8, 0.0), Decision::Promote); // improved
        assert_eq!(decide(0.6, 0.6, 0.0), Decision::Promote); // no regression, margin 0
        assert_eq!(decide(0.6, 0.5, 0.0), Decision::Reject); // regressed
        assert_eq!(decide(0.6, 0.62, 0.05), Decision::Reject); // gain below margin
        assert_eq!(decide(0.6, 0.70, 0.05), Decision::Promote); // gain meets margin
    }

    async fn write_candidate(dir: &Path, name: &str) -> PathBuf {
        let cdir = dir.join(name);
        tokio::fs::create_dir_all(&cdir).await.unwrap();
        tokio::fs::write(cdir.join("SKILL.md"), "---\nname: x\ndescription: y\n---\n")
            .await
            .unwrap();
        cdir
    }

    #[tokio::test]
    async fn promote_moves_dir_and_refuses_clobber() {
        let tmp = tempfile::tempdir().unwrap();
        let candidates = tmp.path().join("skill-candidates");
        let skills = tmp.path().join("skills");
        let cdir = write_candidate(&candidates, "auto-a").await;

        let promoted = promote(&cdir, &skills).await.unwrap();
        assert!(promoted.join("SKILL.md").exists());
        assert!(!cdir.exists(), "candidate dir moved, not copied");

        // A second candidate of the same name can't clobber the live one.
        let cdir2 = write_candidate(&candidates, "auto-a").await;
        let err = promote(&cdir2, &skills).await.unwrap_err();
        assert!(matches!(err, Error::Synthesis(_)));
        assert!(cdir2.exists(), "rejected candidate left in place");

        // Missing candidate errors.
        assert!(promote(&candidates.join("nope"), &skills).await.is_err());
    }

    struct MockMeasurer {
        baseline: f64,
        candidate: f64,
        calls: AtomicUsize,
    }

    #[async_trait]
    impl PassRateMeasurer for MockMeasurer {
        async fn pass_rate(&self, candidate_skill: Option<&Path>) -> Result<f64, Error> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(if candidate_skill.is_some() {
                self.candidate
            } else {
                self.baseline
            })
        }
    }

    #[tokio::test]
    async fn verify_promotes_on_gain_and_rejects_on_regression() {
        let tmp = tempfile::tempdir().unwrap();
        let candidates = tmp.path().join("skill-candidates");
        let skills = tmp.path().join("skills");

        // Improvement → promoted (dir moved into live skills).
        write_candidate(&candidates, "auto-good").await;
        let m = MockMeasurer { baseline: 0.6, candidate: 0.8, calls: AtomicUsize::new(0) };
        let rep = verify_and_promote(&m, "auto-good", &candidates, &skills, 0.0)
            .await
            .unwrap();
        assert_eq!(rep.decision, Decision::Promote);
        assert_eq!(m.calls.load(Ordering::Relaxed), 2); // baseline + candidate
        assert!(skills.join("auto-good").join("SKILL.md").exists());
        assert!(!candidates.join("auto-good").exists());

        // Regression → rejected (candidate stays in staging).
        write_candidate(&candidates, "auto-bad").await;
        let m2 = MockMeasurer { baseline: 0.6, candidate: 0.4, calls: AtomicUsize::new(0) };
        let rep2 = verify_and_promote(&m2, "auto-bad", &candidates, &skills, 0.0)
            .await
            .unwrap();
        assert_eq!(rep2.decision, Decision::Reject);
        assert!(!skills.join("auto-bad").exists());
        assert!(candidates.join("auto-bad").exists());
    }
}
