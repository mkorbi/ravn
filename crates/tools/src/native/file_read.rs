use async_trait::async_trait;
use schemars::{schema_for, JsonSchema};
use serde::Deserialize;

use crate::{Permission, Tool, ToolContext, ToolError, ToolOutput};

const DEFAULT_MAX_BYTES: u64 = 64 * 1024;

#[derive(Debug, Deserialize, JsonSchema)]
struct Args {
    /// Absolute filesystem path to read.
    path: String,
    /// Maximum bytes to return. Default 65536; values above 1 MiB are clamped.
    #[serde(default)]
    max_bytes: Option<u64>,
}

pub struct FileRead;

#[async_trait]
impl Tool for FileRead {
    fn name(&self) -> &'static str {
        "file_read"
    }
    fn description(&self) -> &'static str {
        "Read a UTF-8 text file from the local filesystem. Returns an error if the path is a directory or the content isn't valid UTF-8."
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
        _ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let args: Args = serde_json::from_value(args)
            .map_err(|e| ToolError::InvalidArgs(e.to_string()))?;
        let max = args
            .max_bytes
            .unwrap_or(DEFAULT_MAX_BYTES)
            .min(1024 * 1024);

        let meta = tokio::fs::metadata(&args.path)
            .await
            .map_err(|e| ToolError::Io(format!("{}: {}", args.path, e)))?;
        if meta.is_dir() {
            return Err(ToolError::InvalidArgs(format!(
                "{} is a directory",
                args.path
            )));
        }

        let bytes = tokio::fs::read(&args.path)
            .await
            .map_err(|e| ToolError::Io(format!("{}: {}", args.path, e)))?;
        let total = bytes.len();
        let truncated = (total as u64) > max;
        let slice = if truncated {
            &bytes[..max as usize]
        } else {
            &bytes[..]
        };
        let text = std::str::from_utf8(slice)
            .map_err(|e| ToolError::InvalidArgs(format!("non-UTF-8 content: {e}")))?;

        let body = if truncated {
            format!(
                "{text}\n\n[truncated: {} of {total} bytes shown]",
                slice.len()
            )
        } else {
            text.to_string()
        };
        Ok(ToolOutput::ok(body))
    }
}
