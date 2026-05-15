use std::path::PathBuf;

use async_trait::async_trait;
use chrono::Local;
use ravn_memory::Slot;
use schemars::{schema_for, JsonSchema};
use serde::Deserialize;

use crate::{Permission, Tool, ToolContext, ToolError, ToolOutput};

#[derive(Debug, Deserialize, JsonSchema)]
struct Args {
    /// Which memory file to update.
    slot: SlotArg,
    /// Body text to write (Markdown).
    body: String,
    /// Section heading to use when appending. Defaults to today's date.
    #[serde(default)]
    section: Option<String>,
    /// "append" (default) or "replace".
    #[serde(default)]
    mode: Option<Mode>,
}

#[derive(Debug, Deserialize, JsonSchema, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum SlotArg {
    Soul,
    Memory,
    User,
}

impl From<SlotArg> for Slot {
    fn from(value: SlotArg) -> Self {
        match value {
            SlotArg::Soul => Slot::Soul,
            SlotArg::Memory => Slot::Memory,
            SlotArg::User => Slot::User,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
enum Mode {
    #[default]
    Append,
    Replace,
}

/// Tool needs to know where memory files live. The cli constructs one
/// of these with the user's data dir at startup.
pub struct MemorySave {
    pub data_dir: PathBuf,
}

#[async_trait]
impl Tool for MemorySave {
    fn name(&self) -> &'static str {
        "memory_save"
    }
    fn description(&self) -> &'static str {
        "Persist content into soul.md / memory.md / user.md under the agent's data dir. Default mode appends under a `## <section>` heading (today's date if unspecified). Requires approval."
    }
    fn permission(&self) -> Permission {
        Permission::Write
    }
    fn schema(&self) -> serde_json::Value {
        serde_json::to_value(schema_for!(Args)).unwrap_or_default()
    }

    async fn invoke(
        &self,
        args: serde_json::Value,
        _ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let args: Args = serde_json::from_value(args)
            .map_err(|e| ToolError::InvalidArgs(e.to_string()))?;
        let slot: Slot = args.slot.into();
        let mode = args.mode.unwrap_or_default();

        match mode {
            Mode::Replace => {
                ravn_memory::write_slot(&self.data_dir, slot, &args.body)
                    .await
                    .map_err(|e| ToolError::Io(e.to_string()))?;
                Ok(ToolOutput::ok(format!(
                    "replaced {} ({} bytes)",
                    slot.filename(),
                    args.body.len()
                )))
            }
            Mode::Append => {
                let section = args
                    .section
                    .unwrap_or_else(|| Local::now().format("%Y-%m-%d").to_string());
                ravn_memory::append_section(&self.data_dir, slot, &section, &args.body)
                    .await
                    .map_err(|e| ToolError::Io(e.to_string()))?;
                Ok(ToolOutput::ok(format!(
                    "appended {}-byte section `{}` to {}",
                    args.body.len(),
                    section,
                    slot.filename()
                )))
            }
        }
    }
}
