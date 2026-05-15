//! YAML-frontmatter parser for `SKILL.md`.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::Error;

#[derive(Debug, Clone, Deserialize)]
struct Frontmatter {
    name: String,
    description: String,
    #[serde(default)]
    trigger_patterns: Vec<String>,
    #[serde(default)]
    allowed_tools: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub trigger_patterns: Vec<String>,
    pub allowed_tools: Vec<String>,
    pub body: String,
    pub fs_path: PathBuf,
}

/// Parse a single SKILL.md by path.
pub async fn load_skill(path: &Path) -> Result<Skill, Error> {
    let raw = tokio::fs::read_to_string(path).await.map_err(|e| Error::Io(format!(
        "{}: {e}",
        path.display()
    )))?;
    parse_skill_md(&raw, path)
}

fn parse_skill_md(content: &str, fs_path: &Path) -> Result<Skill, Error> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return Err(Error::Parse {
            path: fs_path.display().to_string(),
            reason: "missing `---` frontmatter delimiter at start".into(),
        });
    }
    // Skip the opening `---\n`
    let after_open = &trimmed[3..];
    let after_open = after_open.trim_start_matches(['\r', '\n']);

    // Find the closing `\n---` on its own line.
    let close_marker = "\n---";
    let close_idx = after_open.find(close_marker).ok_or_else(|| Error::Parse {
        path: fs_path.display().to_string(),
        reason: "missing closing `---` for frontmatter".into(),
    })?;
    let frontmatter_str = &after_open[..close_idx];
    let body = after_open[close_idx + close_marker.len()..]
        .trim_start_matches(['\r', '\n'])
        .to_string();

    let fm: Frontmatter = serde_yaml::from_str(frontmatter_str).map_err(|e| Error::Parse {
        path: fs_path.display().to_string(),
        reason: format!("yaml: {e}"),
    })?;

    Ok(Skill {
        name: fm.name,
        description: fm.description,
        trigger_patterns: fm.trigger_patterns,
        allowed_tools: fm.allowed_tools,
        body,
        fs_path: fs_path.to_path_buf(),
    })
}

/// Scan a directory for `<name>/SKILL.md` and parse each one. Subdirs
/// without a SKILL.md are skipped silently; parse errors are logged and
/// the skill is omitted from the result.
pub async fn load_all_from_fs(dir: &Path) -> Result<Vec<Skill>, Error> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::Io(format!("{}: {e}", dir.display()))),
    };
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| Error::Io(format!("read_dir: {e}")))?
    {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let skill_md = path.join("SKILL.md");
        if !skill_md.exists() {
            continue;
        }
        match load_skill(&skill_md).await {
            Ok(s) => out.push(s),
            Err(e) => tracing::warn!(error = %e, "skip malformed skill"),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn parses_full_skill_md() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("SKILL.md");
        let content = r#"---
name: git-workflow
description: Use when working with Git.
trigger_patterns: ["commit", "merge conflict"]
allowed_tools: [shell, file_read]
---
# Git Workflow

## When to use
After every code change.
"#;
        tokio::fs::write(&path, content).await.unwrap();

        let s = load_skill(&path).await.unwrap();
        assert_eq!(s.name, "git-workflow");
        assert_eq!(s.description, "Use when working with Git.");
        assert_eq!(s.trigger_patterns, vec!["commit", "merge conflict"]);
        assert_eq!(s.allowed_tools, vec!["shell", "file_read"]);
        assert!(s.body.starts_with("# Git Workflow"));
    }

    #[tokio::test]
    async fn missing_frontmatter_errors_clearly() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("SKILL.md");
        tokio::fs::write(&path, "# no frontmatter").await.unwrap();
        let err = load_skill(&path).await.unwrap_err();
        match err {
            Error::Parse { reason, .. } => assert!(reason.contains("frontmatter")),
            _ => panic!("expected Parse, got {err:?}"),
        }
    }

    #[tokio::test]
    async fn load_all_scans_subdirectories() {
        let dir = TempDir::new().unwrap();
        for name in ["alpha", "beta"] {
            let sub = dir.path().join(name);
            tokio::fs::create_dir_all(&sub).await.unwrap();
            tokio::fs::write(
                sub.join("SKILL.md"),
                format!(
                    "---\nname: {name}\ndescription: test {name}\n---\nbody {name}"
                ),
            )
            .await
            .unwrap();
        }
        // A subdir without SKILL.md is silently skipped.
        tokio::fs::create_dir_all(dir.path().join("empty"))
            .await
            .unwrap();

        let skills = load_all_from_fs(dir.path()).await.unwrap();
        assert_eq!(skills.len(), 2);
        let mut names: Vec<_> = skills.iter().map(|s| s.name.clone()).collect();
        names.sort();
        assert_eq!(names, vec!["alpha", "beta"]);
    }

    #[tokio::test]
    async fn load_all_missing_dir_returns_empty() {
        let dir = TempDir::new().unwrap();
        let skills = load_all_from_fs(&dir.path().join("absent"))
            .await
            .unwrap();
        assert!(skills.is_empty());
    }
}
