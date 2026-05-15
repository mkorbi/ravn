//! `skill_list` — list all skills with name + description (Phase 2.6).
//!
//! Progressive Disclosure step 1: the LLM sees only the cheap metadata
//! (~100 tokens per skill) up front; full SKILL.md bodies are pulled in
//! on demand via [`crate::native::skill_view`].

use async_trait::async_trait;
use schemars::{schema_for, JsonSchema};
use serde::Deserialize;

use crate::{Permission, Tool, ToolContext, ToolError, ToolOutput};

#[derive(Debug, Deserialize, JsonSchema)]
struct Args {
    /// Optional FTS5 query to narrow down skills. Empty = list all.
    #[serde(default)]
    query: Option<String>,
    /// Max skills to return. Default 20.
    #[serde(default)]
    limit: Option<i64>,
}

pub struct SkillList;

#[async_trait]
impl Tool for SkillList {
    fn name(&self) -> &'static str {
        "skill_list"
    }
    fn description(&self) -> &'static str {
        "List available skills (each with a one-line description). Pass `query` to narrow by topic. Call `skill_view <name>` to read the full SKILL.md for any skill."
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
        let limit = args.limit.unwrap_or(20).clamp(1, 100);

        let rows = match args.query.as_deref() {
            Some(q) if !q.trim().is_empty() => {
                ravn_persistence::skills::search(&ctx.db, q, limit)
                    .await
                    .map_err(|e| ToolError::Internal(e.to_string()))?
            }
            _ => {
                let mut all = ravn_persistence::skills::list_all(&ctx.db)
                    .await
                    .map_err(|e| ToolError::Internal(e.to_string()))?;
                all.truncate(limit as usize);
                all
            }
        };

        if rows.is_empty() {
            return Ok(ToolOutput::ok("no skills registered"));
        }

        let mut buf = format!("{} skill(s):\n\n", rows.len());
        for s in rows {
            buf.push_str(&format!("- {}: {}\n", s.name, summarize(&s.description)));
        }
        Ok(ToolOutput::ok(buf))
    }
}

fn summarize(s: &str) -> String {
    let one_line: String = s.replace('\n', " ");
    if one_line.len() > 200 {
        format!("{}…", &one_line[..200])
    } else {
        one_line
    }
}
