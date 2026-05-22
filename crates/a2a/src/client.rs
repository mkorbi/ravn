//! A2A client (Phase 5.7): discover a peer's Agent Card and send it a task.

use anyhow::{anyhow, Context};
use serde_json::json;
use uuid::Uuid;

use crate::config::{PeerConfig, PeerOAuth};
use crate::types::{AgentCard, Part, Task};

pub struct A2aClient {
    http: reqwest::Client,
}

impl Default for A2aClient {
    fn default() -> Self {
        Self::new()
    }
}

impl A2aClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }

    /// Fetch a peer's Agent Card.
    pub async fn discover(&self, card_url: &str) -> anyhow::Result<AgentCard> {
        self.http
            .get(card_url)
            .send()
            .await?
            .error_for_status()?
            .json::<AgentCard>()
            .await
            .context("parse agent card")
    }

    /// Send a one-shot text message to a peer (`message/send`) and return the
    /// reply text. Obtains an OAuth2 client-credentials token first if the
    /// peer is configured with one.
    pub async fn call_peer(&self, peer: &PeerConfig, text: &str) -> anyhow::Result<String> {
        let card = self
            .discover(&peer.card_url)
            .await
            .with_context(|| format!("discover peer '{}'", peer.name))?;

        let token = match &peer.oauth {
            Some(o) => Some(fetch_token(&self.http, o).await.context("oauth token")?),
            None => None,
        };

        let req = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "message/send",
            "params": { "message": {
                "role": "user",
                "parts": [{ "kind": "text", "text": text }],
                "messageId": Uuid::new_v4().to_string(),
                "kind": "message"
            }}
        });

        let mut rb = self.http.post(&card.url).json(&req);
        if let Some(t) = &token {
            rb = rb.bearer_auth(t);
        }
        let resp: serde_json::Value = rb
            .send()
            .await?
            .error_for_status()
            .context("peer returned error status")?
            .json()
            .await?;

        if let Some(err) = resp.get("error") {
            return Err(anyhow!("peer JSON-RPC error: {err}"));
        }
        let task: Task = serde_json::from_value(resp.get("result").cloned().unwrap_or_default())
            .context("parse task result")?;
        Ok(extract_reply(&task))
    }
}

/// Pull the reply text out of a returned Task (artifacts first, then the final
/// status message).
fn extract_reply(task: &Task) -> String {
    let mut out = String::new();
    for a in &task.artifacts {
        for p in &a.parts {
            if let Part::Text { text } = p {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(text);
            }
        }
    }
    if out.is_empty() {
        if let Some(m) = &task.status.message {
            out = m.text();
        }
    }
    if out.is_empty() {
        out = format!("[task {} ended in state {:?}]", task.id, task.status.state);
    }
    out
}

/// OAuth2 client-credentials grant (a plain form POST — no extra crate needed).
async fn fetch_token(http: &reqwest::Client, oauth: &PeerOAuth) -> anyhow::Result<String> {
    let scope = oauth.scopes.join(" ");
    let mut form = vec![
        ("grant_type", "client_credentials".to_string()),
        ("client_id", oauth.client_id.clone()),
        ("client_secret", oauth.client_secret.clone()),
    ];
    if !scope.is_empty() {
        form.push(("scope", scope));
    }
    let resp: serde_json::Value = http
        .post(&oauth.token_url)
        .form(&form)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    resp["access_token"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| anyhow!("token response missing access_token"))
}
