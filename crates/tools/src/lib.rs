//! Tool-Trait, Permission-Model, ToolContext, ToolRegistry.
//!
//! Architektur-Notiz: `Tool` ist object-safe (`Arc<dyn Tool>`) — Args
//! kommen als `serde_json::Value` rein, jede Tool-Impl parst typed
//! intern via `serde_json::from_value::<MyArgs>(…)`. JSON-Schema wird
//! pro Tool via `schemars::schema_for!(MyArgs)` an der Impl-Site
//! generiert (siehe Phase 1.4 für konkrete Tools).
//!
//! `ToolContext` haelt Db-Handle, Session/Trace-IDs, CancellationToken
//! und den `Approver` (D7: Inline-Modal in der TUI).

pub mod context;
pub mod native;
pub mod registry;
pub mod tool;

pub use context::{AllowAll, Approver, ApprovalDecision, DenyAll, ToolContext};
pub use registry::ToolRegistry;
pub use tool::{Permission, Tool, ToolError, ToolOutput};

#[cfg(test)]
mod native_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use ravn_persistence::Db;
    use schemars::{schema_for, JsonSchema};
    use serde::Deserialize;
    use std::sync::Arc;

    #[derive(Debug, Deserialize, JsonSchema)]
    struct EchoArgs {
        message: String,
    }

    struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &'static str {
            "echo"
        }
        fn description(&self) -> &'static str {
            "Echo the input back. Test-only."
        }
        fn permission(&self) -> Permission {
            Permission::Read
        }
        fn schema(&self) -> serde_json::Value {
            serde_json::to_value(schema_for!(EchoArgs)).unwrap_or_default()
        }
        async fn invoke(
            &self,
            args: serde_json::Value,
            _ctx: &ToolContext,
        ) -> Result<ToolOutput, ToolError> {
            let parsed: EchoArgs = serde_json::from_value(args)
                .map_err(|e| ToolError::InvalidArgs(e.to_string()))?;
            Ok(ToolOutput::ok(parsed.message))
        }
    }

    #[tokio::test]
    async fn registry_holds_and_serializes_tool() {
        let mut reg = ToolRegistry::new();
        reg.register(EchoTool);

        assert_eq!(reg.len(), 1);
        let schemas = reg.as_schemas();
        assert_eq!(schemas.len(), 1);
        assert_eq!(schemas[0].name, "echo");
        let props = schemas[0].parameters.get("properties").unwrap();
        assert!(props.get("message").is_some());
    }

    #[tokio::test]
    async fn echo_tool_round_trips_value() {
        let db = Db::open_in_memory().await.unwrap();
        let ctx = ToolContext::new(
            db,
            "sess-1",
            "trace-1",
            Arc::new(AllowAll),
        );

        let tool = EchoTool;
        let out = tool
            .invoke(serde_json::json!({"message": "hi"}), &ctx)
            .await
            .unwrap();
        assert_eq!(out.content, "hi");
        assert!(out.trustworthy);
        assert!(!out.is_error);
    }

    #[tokio::test]
    async fn invalid_args_are_rejected() {
        let db = Db::open_in_memory().await.unwrap();
        let ctx = ToolContext::new(db, "s", "t", Arc::new(AllowAll));

        let tool = EchoTool;
        let err = tool
            .invoke(serde_json::json!({"wrong": 1}), &ctx)
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[test]
    fn permission_needs_approval() {
        assert!(!Permission::Read.needs_approval());
        assert!(Permission::Write.needs_approval());
        assert!(Permission::Exec.needs_approval());
    }
}
