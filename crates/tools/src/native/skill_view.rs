//! `skill_view` — return the full SKILL.md body for one skill
//! (Phase 2.6, progressive-disclosure step 2).

use async_trait::async_trait;
use schemars::{schema_for, JsonSchema};
use serde::Deserialize;

use crate::{Permission, Tool, ToolContext, ToolError, ToolOutput};

#[derive(Debug, Deserialize, JsonSchema)]
struct Args {
    /// Skill name as returned by `skill_list`.
    name: String,
}

pub struct SkillView;

#[async_trait]
impl Tool for SkillView {
    fn name(&self) -> &'static str {
        "skill_view"
    }
    fn description(&self) -> &'static str {
        "Return the full SKILL.md body for one skill (call after `skill_list` to fetch details). Use the skill's instructions verbatim — don't paraphrase."
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

        let row = ravn_persistence::skills::get_by_name(&ctx.db, &args.name)
            .await
            .map_err(|e| ToolError::Internal(e.to_string()))?;
        let row = match row {
            Some(r) => r,
            None => {
                return Ok(ToolOutput::error(format!(
                    "no skill named `{}`",
                    args.name
                )))
            }
        };

        // Render description as a leading paragraph + the SKILL.md body.
        let header = format!("# {} (skill)\n\n{}\n\n---\n\n", row.name, row.description);
        Ok(ToolOutput::ok(format!("{header}{}", row.body)))
    }
}
