use async_trait::async_trait;
use schemars::{schema_for, JsonSchema};
use serde::Deserialize;

use crate::{Permission, Tool, ToolContext, ToolError, ToolOutput};

const DEFAULT_LIMIT: i64 = 10;

#[derive(Debug, Deserialize, JsonSchema)]
struct Args {
    /// FTS5 match query (supports `AND`, `OR`, `NOT`, `"phrase"`, prefix `term*`).
    query: String,
    /// Maximum hits to return. Default 10, capped at 50.
    #[serde(default)]
    limit: Option<i64>,
}

pub struct SessionSearch;

#[async_trait]
impl Tool for SessionSearch {
    fn name(&self) -> &'static str {
        "session_search"
    }
    fn description(&self) -> &'static str {
        "Full-text search across messages in every past session. Returns BM25-ranked hits with session ID, role, and a content excerpt."
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
        let limit = args.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 50);

        let hits = ravn_persistence::messages::search(&ctx.db, &args.query, limit)
            .await
            .map_err(|e| ToolError::Internal(e.to_string()))?;

        if hits.is_empty() {
            return Ok(ToolOutput::ok(format!("no hits for `{}`", args.query)));
        }
        let mut buf = format!("{} hit(s) for `{}`:\n\n", hits.len(), args.query);
        for h in hits {
            let excerpt = h.content.chars().take(240).collect::<String>();
            buf.push_str(&format!(
                "- session={} role={} id={}\n  {}\n",
                &h.session_id[..h.session_id.len().min(8)],
                h.role,
                h.id,
                excerpt
            ));
        }
        Ok(ToolOutput::ok(buf))
    }
}
