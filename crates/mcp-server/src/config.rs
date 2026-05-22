//! Which of ravn's tools are exposed to external MCP clients.
//!
//! Parsed from `~/.ravn/mcp-server.toml`. A missing file falls back to a
//! sensible read-only default set. Whatever the config lists, only
//! `Permission::Read` tools that actually exist are ever exposed — Write/Exec
//! are dropped (with a warning), so an external client can never make ravn
//! write files, run shell commands, or mutate memory.

use std::collections::HashSet;
use std::path::Path;

use ravn_tools::{Permission, ToolRegistry};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct ExposeConfig {
    #[serde(default = "default_expose")]
    pub expose: Vec<String>,
}

impl Default for ExposeConfig {
    fn default() -> Self {
        Self {
            expose: default_expose(),
        }
    }
}

fn default_expose() -> Vec<String> {
    ["session_search", "skill_list", "skill_view", "datetime"]
        .into_iter()
        .map(String::from)
        .collect()
}

impl ExposeConfig {
    /// Load `mcp-server.toml`. A missing file yields the default read-only set;
    /// a malformed file is an error.
    pub async fn load(path: &Path) -> anyhow::Result<Self> {
        match tokio::fs::read_to_string(path).await {
            Ok(text) => Ok(toml::from_str(&text)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// Resolve the effective exposure set: configured names ∩ registered tools ∩
    /// `Permission::Read`. Non-Read or unknown names are dropped with a warning,
    /// enforcing the read-only guarantee regardless of what the file lists.
    pub fn resolve_exposed(&self, registry: &ToolRegistry) -> HashSet<String> {
        let mut out = HashSet::new();
        for name in &self.expose {
            match registry.get(name) {
                Some(t) if matches!(t.permission(), Permission::Read) => {
                    out.insert(name.clone());
                }
                Some(t) => tracing::warn!(
                    tool = %name,
                    permission = ?t.permission(),
                    "mcp-server: refusing to expose non-Read tool"
                ),
                None => {
                    tracing::warn!(tool = %name, "mcp-server: configured tool not found; skipping")
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ravn_tools::native;
    use tempfile::TempDir;

    fn registry() -> ToolRegistry {
        let mut r = ToolRegistry::new();
        let dir = TempDir::new().unwrap();
        native::register_defaults(&mut r, dir.path().to_path_buf(), None);
        r
    }

    #[tokio::test]
    async fn missing_file_uses_default_read_set() {
        let dir = TempDir::new().unwrap();
        let cfg = ExposeConfig::load(&dir.path().join("absent.toml"))
            .await
            .unwrap();
        let exposed = cfg.resolve_exposed(&registry());
        assert!(exposed.contains("session_search"));
        assert!(exposed.contains("datetime"));
        assert_eq!(exposed.len(), 4);
    }

    #[tokio::test]
    async fn write_exec_and_unknown_tools_are_filtered_out() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("mcp-server.toml");
        tokio::fs::write(
            &path,
            r#"expose = ["datetime", "shell", "file_write", "world_write", "nope"]"#,
        )
        .await
        .unwrap();
        let cfg = ExposeConfig::load(&path).await.unwrap();
        let exposed = cfg.resolve_exposed(&registry());
        assert!(exposed.contains("datetime"));
        assert!(!exposed.contains("shell")); // Exec
        assert!(!exposed.contains("file_write")); // Write
        assert!(!exposed.contains("world_write")); // Write
        assert!(!exposed.contains("nope")); // unknown
        assert_eq!(exposed.len(), 1);
    }
}
