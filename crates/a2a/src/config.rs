//! A2A configuration (`~/.ravn/a2a.toml`).
//!
//! Controls the server (bind address, advertised card metadata, how much an
//! incoming task is allowed to do), optional OAuth/JWT auth, and the known
//! peer agents ravn can call (the client side).

use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct A2aConfig {
    #[serde(default)]
    pub server: ServerConfig,
    /// Absent ⇒ auth disabled (dev only).
    #[serde(default)]
    pub auth: Option<AuthConfig>,
    #[serde(default, rename = "peer")]
    pub peers: Vec<PeerConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ServerConfig {
    /// Socket address to listen on.
    #[serde(default = "default_bind")]
    pub bind: String,
    /// URL advertised in the Agent Card `url` field (what clients POST to).
    #[serde(default = "default_public_url")]
    pub public_url: String,
    #[serde(default = "default_name")]
    pub name: String,
    #[serde(default = "default_description")]
    pub description: String,
    /// Write/Exec tools an incoming task may use. Empty ⇒ **read-only**
    /// (Read tools always run; Write/Exec are denied). External callers are
    /// untrusted, so the default is the safe one.
    #[serde(default)]
    pub allow_tools: Vec<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            public_url: default_public_url(),
            name: default_name(),
            description: default_description(),
            allow_tools: Vec::new(),
        }
    }
}

fn default_bind() -> String {
    "127.0.0.1:8723".to_string()
}
fn default_public_url() -> String {
    "http://127.0.0.1:8723/".to_string()
}
fn default_name() -> String {
    "ravn".to_string()
}
fn default_description() -> String {
    "A personal-assistant AI agent (ravn) exposed over A2A.".to_string()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AuthConfig {
    pub issuer: String,
    pub jwks_url: String,
    pub audience: String,
    #[serde(default)]
    pub required_scopes: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PeerConfig {
    pub name: String,
    /// Where to fetch the peer's Agent Card.
    pub card_url: String,
    #[serde(default)]
    pub oauth: Option<PeerOAuth>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PeerOAuth {
    pub token_url: String,
    pub client_id: String,
    pub client_secret: String,
    #[serde(default)]
    pub scopes: Vec<String>,
}

impl A2aConfig {
    /// Load `a2a.toml`. A missing file yields defaults (server on
    /// 127.0.0.1:8723, no auth, no peers); a malformed file is an error.
    pub async fn load(path: &Path) -> anyhow::Result<Self> {
        match tokio::fs::read_to_string(path).await {
            Ok(text) => Ok(toml::from_str(&text)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn peer(&self, name: &str) -> Option<&PeerConfig> {
        self.peers.iter().find(|p| p.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn missing_file_is_default() {
        let dir = TempDir::new().unwrap();
        let cfg = A2aConfig::load(&dir.path().join("absent.toml"))
            .await
            .unwrap();
        assert_eq!(cfg.server.bind, "127.0.0.1:8723");
        assert!(cfg.auth.is_none());
        assert!(cfg.peers.is_empty());
        assert!(cfg.server.allow_tools.is_empty()); // read-only default
    }

    #[tokio::test]
    async fn parses_server_auth_and_peers() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a2a.toml");
        tokio::fs::write(
            &path,
            r#"
[server]
bind = "0.0.0.0:9000"
name = "ravn-prod"

[auth]
issuer = "https://idp.example.com/"
jwks_url = "https://idp.example.com/.well-known/jwks.json"
audience = "ravn-a2a"
required_scopes = ["a2a.invoke"]

[[peer]]
name = "researcher"
card_url = "https://researcher.example.com/.well-known/agent-card.json"
oauth = { token_url = "https://idp.example.com/token", client_id = "ravn", client_secret = "s3cret", scopes = ["a2a.invoke"] }
"#,
        )
        .await
        .unwrap();
        let cfg = A2aConfig::load(&path).await.unwrap();
        assert_eq!(cfg.server.bind, "0.0.0.0:9000");
        assert_eq!(cfg.server.name, "ravn-prod");
        let auth = cfg.auth.as_ref().unwrap();
        assert_eq!(auth.audience, "ravn-a2a");
        assert_eq!(auth.required_scopes, vec!["a2a.invoke"]);
        let peer = cfg.peer("researcher").unwrap();
        assert!(peer.oauth.is_some());
        assert!(cfg.peer("nope").is_none());
    }
}
