#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("llm: {0}")]
    Llm(#[from] ravn_llm::Error),

    #[error("persistence: {0}")]
    Persistence(#[from] ravn_persistence::Error),

    #[error("tool: {0}")]
    Tool(#[from] ravn_tools::ToolError),

    #[error("loop cancelled")]
    Cancelled,

    #[error("budget exceeded: {0}")]
    BudgetExceeded(String),

    #[error("internal: {0}")]
    Internal(String),
}
