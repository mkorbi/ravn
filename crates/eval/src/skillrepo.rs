//! Phase 6.5: the live skills directory as a git repo, with atomic rollback.
//!
//! Each promotion (Phase 6.4) becomes one commit, so a regression is undone by
//! resetting to the commit's parent — `git reset --hard` + `clean` is atomic
//! (the working tree ends up exactly at the target revision, never partway).
//! Promotions are therefore reversible and auditable.

use std::path::{Path, PathBuf};

use tokio::process::Command;

use crate::Error;

pub struct SkillRepo {
    dir: PathBuf,
}

/// The result of [`promote_committed`]: the new revision and the `parent` it
/// replaced. `repo.rollback_to(parent)` undoes the promotion.
#[derive(Debug, Clone)]
pub struct PromotionCommit {
    pub rev: String,
    pub parent: Option<String>,
}

impl SkillRepo {
    pub fn open(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    async fn git(&self, args: &[&str]) -> Result<String, Error> {
        let out = Command::new("git")
            .arg("-C")
            .arg(&self.dir)
            .args(args)
            .output()
            .await
            .map_err(|e| Error::Io(format!("git {}: {e}", args.join(" "))))?;
        if !out.status.success() {
            return Err(Error::Synthesis(format!(
                "git {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    fn is_repo(&self) -> bool {
        self.dir.join(".git").exists()
    }

    /// Ensure the dir is a git repo with at least one commit (a base state), so
    /// later promotions have a parent to roll back to. Sets a local identity so
    /// commits work without global git config.
    pub async fn ensure_init(&self) -> Result<(), Error> {
        tokio::fs::create_dir_all(&self.dir)
            .await
            .map_err(|e| Error::Io(format!("create {}: {e}", self.dir.display())))?;
        if !self.is_repo() {
            self.git(&["init", "-q"]).await?;
            self.git(&["config", "user.email", "curator@ravn.local"]).await?;
            self.git(&["config", "user.name", "ravn curator"]).await?;
            self.git(&["config", "commit.gpgsign", "false"]).await?;
        }
        if self.head().await?.is_none() {
            self.git(&["add", "-A"]).await?;
            self.git(&["commit", "-q", "--allow-empty", "-m", "init skills repo"])
                .await?;
        }
        Ok(())
    }

    /// Current HEAD revision, or `None` if there are no commits yet.
    pub async fn head(&self) -> Result<Option<String>, Error> {
        if !self.is_repo() {
            return Ok(None);
        }
        let out = Command::new("git")
            .arg("-C")
            .arg(&self.dir)
            .args(["rev-parse", "HEAD"])
            .output()
            .await
            .map_err(|e| Error::Io(format!("git rev-parse: {e}")))?;
        Ok(out
            .status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
            .filter(|s| !s.is_empty()))
    }

    /// Stage everything and commit; returns the resulting HEAD. A no-op commit
    /// (nothing changed) just returns the current HEAD.
    pub async fn commit_all(&self, message: &str) -> Result<String, Error> {
        self.git(&["add", "-A"]).await?;
        if !self.git(&["status", "--porcelain"]).await?.is_empty() {
            self.git(&["commit", "-q", "-m", message]).await?;
        }
        self.head()
            .await?
            .ok_or_else(|| Error::Synthesis("no HEAD after commit".into()))
    }

    /// Atomically reset the working tree to `rev` and remove untracked files —
    /// undoing any promotions made after `rev`.
    pub async fn rollback_to(&self, rev: &str) -> Result<(), Error> {
        self.git(&["reset", "--hard", rev]).await?;
        self.git(&["clean", "-fdq"]).await?;
        Ok(())
    }
}

/// Promote a candidate into `repo` and record it as one commit. Roll the
/// promotion back later with `repo.rollback_to(commit.parent)`.
pub async fn promote_committed(
    repo: &SkillRepo,
    candidate_dir: &Path,
    message: &str,
) -> Result<PromotionCommit, Error> {
    repo.ensure_init().await?;
    let parent = repo.head().await?;
    crate::synthesis::promote(candidate_dir, repo.dir()).await?;
    let rev = repo.commit_all(message).await?;
    Ok(PromotionCommit { rev, parent })
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn candidate(dir: &Path, name: &str) -> PathBuf {
        let c = dir.join(name);
        tokio::fs::create_dir_all(&c).await.unwrap();
        tokio::fs::write(c.join("SKILL.md"), format!("---\nname: {name}\ndescription: t\n---\n"))
            .await
            .unwrap();
        c
    }

    #[tokio::test]
    async fn promote_commits_and_rolls_back_atomically() {
        let tmp = tempfile::tempdir().unwrap();
        let skills = tmp.path().join("skills");
        let candidates = tmp.path().join("skill-candidates");
        let cdir = candidate(&candidates, "auto-a").await;

        let repo = SkillRepo::open(&skills);
        let commit = promote_committed(&repo, &cdir, "promote auto-a").await.unwrap();

        // Promoted, committed, candidate consumed.
        assert!(skills.join("auto-a").join("SKILL.md").exists());
        assert!(!cdir.exists());
        assert!(commit.parent.is_some(), "base commit exists to roll back to");
        assert_eq!(repo.head().await.unwrap().as_deref(), Some(commit.rev.as_str()));

        // Rollback to the parent removes the promoted skill and restores HEAD.
        repo.rollback_to(commit.parent.as_deref().unwrap()).await.unwrap();
        assert!(!skills.join("auto-a").exists(), "promotion undone");
        assert_eq!(repo.head().await.unwrap(), commit.parent);
    }

    #[tokio::test]
    async fn second_promotion_is_a_separate_revertible_commit() {
        let tmp = tempfile::tempdir().unwrap();
        let skills = tmp.path().join("skills");
        let candidates = tmp.path().join("skill-candidates");
        let repo = SkillRepo::open(&skills);

        let a = candidate(&candidates, "auto-a").await;
        promote_committed(&repo, &a, "promote auto-a").await.unwrap();
        let b = candidate(&candidates, "auto-b").await;
        let c2 = promote_committed(&repo, &b, "promote auto-b").await.unwrap();

        // Rolling back only the second promotion keeps the first.
        repo.rollback_to(c2.parent.as_deref().unwrap()).await.unwrap();
        assert!(skills.join("auto-a").exists());
        assert!(!skills.join("auto-b").exists());
    }
}
