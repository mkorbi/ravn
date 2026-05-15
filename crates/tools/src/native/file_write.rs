use async_trait::async_trait;
use schemars::{schema_for, JsonSchema};
use serde::Deserialize;

use crate::{Permission, Tool, ToolContext, ToolError, ToolOutput};

#[derive(Debug, Deserialize, JsonSchema)]
struct Args {
    /// Absolute filesystem path to write.
    path: String,
    /// File contents (UTF-8).
    content: String,
    /// Create missing parent directories. Default false.
    #[serde(default)]
    create_dirs: bool,
}

pub struct FileWrite;

#[async_trait]
impl Tool for FileWrite {
    fn name(&self) -> &'static str {
        "file_write"
    }
    fn description(&self) -> &'static str {
        "Write a UTF-8 text file to the local filesystem. Overwrites existing files. Requires approval."
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
        if args.create_dirs {
            if let Some(parent) = std::path::Path::new(&args.path).parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| ToolError::Io(format!("mkdir {}: {}", parent.display(), e)))?;
            }
        }
        let bytes = args.content.len();
        tokio::fs::write(&args.path, args.content.as_bytes())
            .await
            .map_err(|e| ToolError::Io(format!("{}: {}", args.path, e)))?;
        Ok(ToolOutput::ok(format!("wrote {bytes} bytes to {}", args.path)))
    }
}
