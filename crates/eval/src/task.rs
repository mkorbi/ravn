//! Eval-task file format.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Error;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EvalTask {
    /// Stable id for cross-run comparison. Filename stem if not set.
    #[serde(default)]
    pub id: String,
    /// Short human-readable title.
    pub title: String,
    /// User input the agent receives as its `user_turn`.
    pub input: String,
    /// Rubric the judge uses to grade `final_text`. Free-text, but
    /// favor concrete checks: "mentions X", "produces a file at Y",
    /// "no shell command run", etc.
    pub rubric: String,
    /// Which tools the agent should have access to. Default = all
    /// native tools.
    #[serde(default)]
    pub tools: ToolSet,
    /// Hard caps on the agent run.
    #[serde(default)]
    pub max_steps: Option<usize>,
    #[serde(default)]
    pub max_cost_usd: Option<f64>,
    /// Topic / tag for report grouping.
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ToolSet {
    /// All native tools registered by `ravn_tools::native::register_defaults`.
    #[default]
    AllNative,
    /// No tools (pure-text Q&A only).
    None,
    /// Explicit allowlist of tool names.
    Subset { names: Vec<String> },
}

impl EvalTask {
    pub async fn load(path: &Path) -> Result<Self, Error> {
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
        let text = std::str::from_utf8(&bytes).map_err(|e| Error::Parse {
            path: path.display().to_string(),
            reason: format!("utf-8: {e}"),
        })?;
        let mut task: EvalTask = toml::from_str(text).map_err(|e| Error::Parse {
            path: path.display().to_string(),
            reason: e.to_string(),
        })?;
        if task.id.is_empty() {
            task.id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();
        }
        Ok(task)
    }

    pub async fn load_all(dir: &Path) -> Result<Vec<Self>, Error> {
        if !dir.exists() {
            return Err(Error::Io(format!(
                "task dir not found: {}",
                dir.display()
            )));
        }
        let mut entries = tokio::fs::read_dir(dir)
            .await
            .map_err(|e| Error::Io(format!("{}: {e}", dir.display())))?;
        let mut tasks = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| Error::Io(format!("read_dir: {e}")))?
        {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("toml") {
                continue;
            }
            let task = Self::load(&path).await?;
            tasks.push(task);
        }
        tasks.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(tasks)
    }
}

/// Resolves the default task-dir relative to this crate. Used by the
/// `ravn-eval` binary when no `--tasks` flag is given.
pub fn default_tasks_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tasks")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn loads_minimal_toml() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("t1.toml");
        tokio::fs::write(
            &path,
            r#"
title = "Trivia"
input = "What year is it?"
rubric = "mentions a four-digit year"
"#,
        )
        .await
        .unwrap();
        let task = EvalTask::load(&path).await.unwrap();
        assert_eq!(task.id, "t1");
        assert_eq!(task.title, "Trivia");
        assert!(matches!(task.tools, ToolSet::AllNative));
    }

    #[tokio::test]
    async fn load_all_skips_non_toml() {
        let dir = TempDir::new().unwrap();
        tokio::fs::write(
            dir.path().join("ok.toml"),
            r#"title="a"
input="b"
rubric="c""#,
        )
        .await
        .unwrap();
        tokio::fs::write(dir.path().join("README.md"), "ignore me")
            .await
            .unwrap();
        let tasks = EvalTask::load_all(dir.path()).await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, "ok");
    }
}
