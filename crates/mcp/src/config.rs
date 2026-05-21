//! MCP server configuration (Phase 2.2, D14).
//!
//! Parsed from `~/.ravn/mcp-servers.toml`. Example:
//!
//! ```toml
//! [servers.filesystem]
//! command = "npx"
//! args = ["-y", "@modelcontextprotocol/server-filesystem", "/Users/maxbot/projects"]
//! env = ["PATH", "HOME"]   # passthrough whitelist; if omitted, all env passed
//! permission = "write"      # server-wide default
//!
//! [servers.github]
//! command = "npx"
//! args = ["-y", "@modelcontextprotocol/server-github"]
//! env = ["PATH", "HOME", "GITHUB_PERSONAL_ACCESS_TOKEN"]
//! permission = "read"
//!
//! # Optional per-tool override (rare). Tool names are namespaced as
//! # `<server>__<tool>`.
//! [tools."github__create_issue"]
//! permission = "write"
//! ```

use std::collections::HashMap;
use std::path::Path;

use ravn_tools::Permission;
use serde::Deserialize;

use crate::error::Error;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub servers: HashMap<String, ServerConfig>,
    #[serde(default)]
    pub tools: HashMap<String, ToolOverride>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Whitelist of env-vars to forward to the spawned subprocess. When
    /// omitted, only `PATH` and `HOME` are forwarded (sane minimum).
    #[serde(default)]
    pub env: Option<Vec<String>>,
    #[serde(default = "default_permission", deserialize_with = "deserialize_permission")]
    pub permission: Permission,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolOverride {
    #[serde(deserialize_with = "deserialize_permission")]
    pub permission: Permission,
}

fn default_permission() -> Permission {
    // D14: conservative default — every MCP tool requires approval
    // unless the server explicitly opts into Read.
    Permission::Write
}

fn deserialize_permission<'de, D>(d: D) -> Result<Permission, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(d)?;
    match s.as_str() {
        "read" => Ok(Permission::Read),
        "write" => Ok(Permission::Write),
        "exec" => Ok(Permission::Exec),
        other => Err(serde::de::Error::custom(format!(
            "permission must be read|write|exec, got `{other}`"
        ))),
    }
}

impl Config {
    /// Load `mcp-servers.toml`. A missing file is a non-error (returns
    /// empty config); a malformed file is an error.
    pub async fn load(path: &Path) -> Result<Self, Error> {
        let bytes = match tokio::fs::read(path).await {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(e) => return Err(Error::Io(e.to_string())),
        };
        let text = std::str::from_utf8(&bytes).map_err(|e| Error::Config(e.to_string()))?;
        toml::from_str(text).map_err(|e| Error::Config(e.to_string()))
    }

    /// Resolve the effective permission for `<server>__<tool>`: look at
    /// per-tool override first, fall back to server default.
    pub fn permission_for(&self, server: &str, tool: &str) -> Permission {
        let key = format!("{server}__{tool}");
        if let Some(ov) = self.tools.get(&key) {
            return ov.permission;
        }
        self.servers
            .get(server)
            .map(|s| s.permission)
            .unwrap_or(Permission::Write)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn missing_file_is_ok() {
        let dir = TempDir::new().unwrap();
        let cfg = Config::load(&dir.path().join("absent.toml")).await.unwrap();
        assert!(cfg.servers.is_empty());
        assert!(cfg.tools.is_empty());
    }

    #[tokio::test]
    async fn parses_two_servers_and_one_override() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("mcp.toml");
        tokio::fs::write(
            &path,
            r#"
[servers.filesystem]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
env = ["PATH", "HOME"]
permission = "write"

[servers.github]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
permission = "read"

[tools."github__create_issue"]
permission = "write"
"#,
        )
        .await
        .unwrap();
        let cfg = Config::load(&path).await.unwrap();
        assert_eq!(cfg.servers.len(), 2);
        assert_eq!(
            cfg.servers["filesystem"].args,
            vec!["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
        );
        assert_eq!(cfg.servers["github"].permission, Permission::Read);
        assert_eq!(
            cfg.permission_for("github", "create_issue"),
            Permission::Write
        );
        assert_eq!(
            cfg.permission_for("github", "list_issues"),
            Permission::Read
        );
        assert_eq!(
            cfg.permission_for("unknown_server", "x"),
            Permission::Write
        );
    }
}
