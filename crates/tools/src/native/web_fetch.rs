use std::time::Duration;

use async_trait::async_trait;
use schemars::{schema_for, JsonSchema};
use serde::Deserialize;

use crate::{Permission, Tool, ToolContext, ToolError, ToolOutput};

const DEFAULT_TIMEOUT_SECS: u64 = 20;
const DEFAULT_MAX_BYTES: usize = 200_000;

#[derive(Debug, Deserialize, JsonSchema)]
struct Args {
    /// Absolute URL to fetch (http or https).
    url: String,
    /// Output format: "markdown" (default), "text", or "html".
    #[serde(default)]
    format: Option<Format>,
    /// Timeout in seconds. Default 20.
    #[serde(default)]
    timeout_secs: Option<u64>,
    /// Maximum bytes of response body to return. Default 200 000.
    #[serde(default)]
    max_bytes: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
enum Format {
    #[default]
    Markdown,
    Text,
    Html,
}

pub struct WebFetch {
    client: reqwest::Client,
}

impl Default for WebFetch {
    fn default() -> Self {
        Self::new()
    }
}

impl WebFetch {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent(concat!("ravn/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .expect("reqwest client");
        Self { client }
    }
}

#[async_trait]
impl Tool for WebFetch {
    fn name(&self) -> &'static str {
        "web_fetch"
    }
    fn description(&self) -> &'static str {
        "Fetch a URL and return its body as Markdown, text, or raw HTML. Output is marked as untrusted (web content)."
    }
    fn permission(&self) -> Permission {
        Permission::Read
    }
    fn schema(&self) -> serde_json::Value {
        serde_json::to_value(schema_for!(Args)).unwrap_or_default()
    }

    async fn invoke(
        &self,
        args: serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let args: Args = serde_json::from_value(args)
            .map_err(|e| ToolError::InvalidArgs(e.to_string()))?;
        if !(args.url.starts_with("http://") || args.url.starts_with("https://")) {
            return Err(ToolError::InvalidArgs(
                "url must start with http:// or https://".into(),
            ));
        }
        let timeout = Duration::from_secs(args.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS));
        let max_bytes = args.max_bytes.unwrap_or(DEFAULT_MAX_BYTES);
        let format = args.format.unwrap_or_default();

        let req = self.client.get(&args.url).timeout(timeout).build()
            .map_err(|e| ToolError::Transport(e.to_string()))?;

        let resp = tokio::select! {
            biased;
            _ = ctx.cancel.cancelled() => return Err(ToolError::Cancelled),
            r = self.client.execute(req) => r,
        }
        .map_err(|e| ToolError::Transport(e.to_string()))?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| ToolError::Transport(e.to_string()))?;
        let truncated = body.len() > max_bytes;
        let slice = if truncated { &body[..max_bytes] } else { &body[..] };

        let mut rendered = match format {
            Format::Html => slice.to_string(),
            Format::Text | Format::Markdown => {
                // html2text yields readable text/markdown-ish; for `format=text`
                // we keep it as-is, for `format=markdown` we keep the same output
                // (it preserves headings + bullets in a markdown-compatible way).
                html2text::from_read(slice.as_bytes(), 100)
                    .unwrap_or_else(|_| slice.to_string())
            }
        };
        if truncated {
            rendered.push_str(&format!(
                "\n\n[truncated: {} of {} bytes]",
                max_bytes,
                body.len()
            ));
        }
        let header = format!("HTTP {} {}\n\n", status.as_u16(), args.url);
        Ok(ToolOutput::untrusted(format!("{header}{rendered}")))
    }
}
