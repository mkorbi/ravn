//! `world_write` — update the persistent world state (Phase 4.11).
//!
//! The agent always sees the current world state under a `# World State`
//! heading in its prompt (injected by `ravn_core::Agent::run`), so this tool
//! takes the **complete** desired state and replaces the stored value rather
//! than applying a patch.

use async_trait::async_trait;
use ravn_persistence::WorldState;
use schemars::schema_for;

use crate::{Permission, Tool, ToolContext, ToolError, ToolOutput};

pub struct WorldWrite;

#[async_trait]
impl Tool for WorldWrite {
    fn name(&self) -> &'static str {
        "world_write"
    }
    fn description(&self) -> &'static str {
        "Replace the persistent world state (projects, open_tabs, watch_targets) shown under the \"# World State\" heading in your context. Pass the COMPLETE desired state — this overwrites, it does not merge — so include everything you want to keep. Use it to add, update, or remove projects, open tabs, and watch targets. Requires approval."
    }
    fn permission(&self) -> Permission {
        Permission::Write
    }
    fn schema(&self) -> serde_json::Value {
        serde_json::to_value(schema_for!(WorldState)).unwrap_or_default()
    }

    async fn invoke(
        &self,
        args: serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let state: WorldState =
            serde_json::from_value(args).map_err(|e| ToolError::InvalidArgs(e.to_string()))?;
        ravn_persistence::world::save(&ctx.db, &state)
            .await
            .map_err(|e| ToolError::Internal(e.to_string()))?;
        Ok(ToolOutput::ok(format!(
            "world state updated: {} project(s), {} tab(s), {} watch target(s)",
            state.projects.len(),
            state.open_tabs.len(),
            state.watch_targets.len()
        )))
    }
}
