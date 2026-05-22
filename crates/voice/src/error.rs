#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no microphone input device available")]
    NoDevice,

    #[error("audio: {0}")]
    Audio(String),

    #[error("unsupported sample format: {0}")]
    UnsupportedFormat(String),

    #[error("model: {0}")]
    Model(String),

    #[error("transcription: {0}")]
    Transcribe(String),

    #[error("io: {0}")]
    Io(String),
}
