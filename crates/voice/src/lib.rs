//! Voice input (Phase 4.7): microphone capture + local Whisper STT.
//!
//! - [`Recorder`] captures from the default mic on a dedicated thread.
//! - [`resample`] converts the captured audio to whisper.cpp's 16 kHz mono f32.
//! - `Transcriber` (added in step 2) runs `whisper-rs` on a lazily-downloaded
//!   ggml model, mirroring `ravn-embeddings`' lazy-load pattern.

pub mod error;
pub mod model;
pub mod recorder;
pub mod resample;
pub mod transcriber;

pub use error::Error;
pub use recorder::{RecordResult, Recorder};
pub use transcriber::Transcriber;
