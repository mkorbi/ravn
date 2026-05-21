#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(String),

    #[error("config: {0}")]
    Config(String),

    #[error("rmcp service: {0}")]
    Service(String),

    #[error("transport: {0}")]
    Transport(String),

    #[error("tool not found: {0}")]
    UnknownTool(String),
}
