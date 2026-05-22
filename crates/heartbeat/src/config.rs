//! Heartbeat job configuration (Phase 4.10).
//!
//! Parsed from `~/.ravn/heartbeats.toml`. Example:
//!
//! ```toml
//! [[job]]
//! name = "morning-calendar"
//! schedule = "0 0 8 * * *"      # 6-field cron: sec min hour day month day-of-week
//! prompt = "Check my calendar for today and give me a one-line summary."
//! allow_tools = ["datetime"]     # Write/Exec tools auto-approved for this job
//! max_steps = 8                  # optional; defaults to 8
//! budget_cost_usd = 0.10         # optional; defaults to 0.10
//!
//! [[job]]
//! name = "nightly-noop"
//! schedule = "0 0 3 * * *"
//! prompt = "do nothing"
//! enabled = false                # registered jobs default to enabled = true
//! ```

use std::path::Path;

use serde::Deserialize;

use crate::error::Error;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct HeartbeatConfig {
    /// Each `[[job]]` table in the TOML.
    #[serde(default, rename = "job")]
    pub jobs: Vec<JobConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JobConfig {
    pub name: String,
    /// 6-field cron expression (`sec min hour day month day-of-week`).
    pub schedule: String,
    /// The instruction handed to the agent as the (synthetic) user turn.
    pub prompt: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Write/Exec tools auto-approved for this job. Read tools are always
    /// allowed (they never reach the approver).
    #[serde(default)]
    pub allow_tools: Vec<String>,
    #[serde(default)]
    pub max_steps: Option<usize>,
    #[serde(default)]
    pub budget_cost_usd: Option<f64>,
}

fn default_true() -> bool {
    true
}

impl HeartbeatConfig {
    /// Load `heartbeats.toml`. A missing file is a non-error (returns an
    /// empty config); a malformed file is an error.
    pub async fn load(path: &Path) -> Result<Self, Error> {
        let bytes = match tokio::fs::read(path).await {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(Error::Io(e.to_string())),
        };
        let text = std::str::from_utf8(&bytes).map_err(|e| Error::Config(e.to_string()))?;
        toml::from_str(text).map_err(|e| Error::Config(e.to_string()))
    }

    pub fn get(&self, name: &str) -> Option<&JobConfig> {
        self.jobs.iter().find(|j| j.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn missing_file_is_ok() {
        let dir = TempDir::new().unwrap();
        let cfg = HeartbeatConfig::load(&dir.path().join("absent.toml"))
            .await
            .unwrap();
        assert!(cfg.jobs.is_empty());
    }

    #[tokio::test]
    async fn parses_jobs_with_defaults() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("heartbeats.toml");
        tokio::fs::write(
            &path,
            r#"
[[job]]
name = "morning"
schedule = "0 0 8 * * *"
prompt = "what time is it?"
allow_tools = ["datetime"]
max_steps = 5

[[job]]
name = "disabled-one"
schedule = "0 0 9 * * *"
prompt = "noop"
enabled = false
"#,
        )
        .await
        .unwrap();

        let cfg = HeartbeatConfig::load(&path).await.unwrap();
        assert_eq!(cfg.jobs.len(), 2);

        let m = cfg.get("morning").unwrap();
        assert!(m.enabled); // defaulted
        assert_eq!(m.allow_tools, vec!["datetime"]);
        assert_eq!(m.max_steps, Some(5));
        assert_eq!(m.budget_cost_usd, None);

        assert!(!cfg.get("disabled-one").unwrap().enabled);
        assert!(cfg.get("nope").is_none());
    }
}
