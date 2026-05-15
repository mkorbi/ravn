use std::time::Duration;

use async_trait::async_trait;
use schemars::{schema_for, JsonSchema};
use serde::Deserialize;
use tokio::process::Command;

use crate::{Permission, Tool, ToolContext, ToolError, ToolOutput};

const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MAX_TIMEOUT_SECS: u64 = 300;

#[derive(Debug, Deserialize, JsonSchema)]
struct Args {
    /// Shell command, executed with `bash -c "<command>"`.
    command: String,
    /// Working directory. Defaults to the current process directory.
    #[serde(default)]
    cwd: Option<String>,
    /// Timeout in seconds. Default 30, max 300.
    #[serde(default)]
    timeout_secs: Option<u64>,
}

pub struct Shell;

#[async_trait]
impl Tool for Shell {
    fn name(&self) -> &'static str {
        "shell"
    }
    fn description(&self) -> &'static str {
        "Run a shell command via `bash -c`. Returns stdout, stderr, and exit code. Requires approval."
    }
    fn permission(&self) -> Permission {
        Permission::Exec
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
        let timeout = Duration::from_secs(
            args.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS).min(MAX_TIMEOUT_SECS),
        );

        let mut cmd = Command::new("bash");
        cmd.arg("-c").arg(&args.command);
        if let Some(dir) = &args.cwd {
            cmd.current_dir(dir);
        }
        cmd.kill_on_drop(true);

        let run = cmd.output();
        let result = tokio::select! {
            biased;
            _ = ctx.cancel.cancelled() => return Err(ToolError::Cancelled),
            r = tokio::time::timeout(timeout, run) => r,
        };

        let output = match result {
            Ok(Ok(o)) => o,
            Ok(Err(e)) => return Err(ToolError::Io(format!("spawn: {e}"))),
            Err(_) => {
                return Err(ToolError::Io(format!(
                    "timeout after {}s",
                    timeout.as_secs()
                )))
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let code = output.status.code().unwrap_or(-1);
        let body = format!("exit={code}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}");
        if code == 0 {
            Ok(ToolOutput::ok(body))
        } else {
            Ok(ToolOutput::error(body))
        }
    }
}
