#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(String),

    #[error("config: {0}")]
    Config(String),

    #[error("scheduler: {0}")]
    Scheduler(String),
}
