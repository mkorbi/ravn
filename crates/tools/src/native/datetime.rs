use async_trait::async_trait;
use chrono::{Local, Utc};
use schemars::{schema_for, JsonSchema};
use serde::Deserialize;

use crate::{Permission, Tool, ToolContext, ToolError, ToolOutput};

#[derive(Debug, Deserialize, JsonSchema)]
struct Args {
    /// Timezone: "local" (default) or "utc".
    #[serde(default)]
    timezone: Option<Zone>,
    /// Optional strftime format. Default: RFC 3339.
    #[serde(default)]
    format: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
enum Zone {
    #[default]
    Local,
    Utc,
}

pub struct DateTime;

#[async_trait]
impl Tool for DateTime {
    fn name(&self) -> &'static str {
        "datetime"
    }
    fn description(&self) -> &'static str {
        "Return the current date and time. Use this whenever the user references 'now', 'today', or relative dates — the model's training cutoff makes its own clock unreliable."
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
        let zone = args.timezone.unwrap_or_default();
        let text = match (zone, args.format) {
            (Zone::Local, None) => Local::now().to_rfc3339(),
            (Zone::Utc, None) => Utc::now().to_rfc3339(),
            (Zone::Local, Some(f)) => Local::now().format(&f).to_string(),
            (Zone::Utc, Some(f)) => Utc::now().format(&f).to_string(),
        };
        Ok(ToolOutput::ok(text))
    }
}
