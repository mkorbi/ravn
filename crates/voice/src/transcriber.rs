//! Local speech-to-text via `whisper-rs` (whisper.cpp).
//!
//! The ggml model is loaded lazily on the first [`Transcriber::transcribe`]
//! call (downloading it if absent), mirroring `ravn-embeddings`' `Embedder`.
//! Both model load and inference run in `spawn_blocking`.

use std::path::PathBuf;
use std::sync::{Arc, Once};

use tokio::sync::OnceCell;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::error::Error;
use crate::model;

static LOG_HOOK: Once = Once::new();

pub struct Transcriber {
    model_path: PathBuf,
    language: Option<String>,
    ctx: OnceCell<Arc<WhisperContext>>,
}

impl Transcriber {
    /// Cheap to construct — no model is loaded until the first `transcribe`.
    pub fn new(data_dir: PathBuf) -> Self {
        // Route whisper.cpp / GGML logs into `tracing` (→ ravn.log) rather than
        // stderr, so they don't corrupt the TUI's alternate screen.
        LOG_HOOK.call_once(whisper_rs::install_logging_hooks);
        Self {
            model_path: model::resolve_model_path(&data_dir),
            language: std::env::var("RAVN_VOICE_LANG").ok(),
            ctx: OnceCell::new(),
        }
    }

    async fn context(&self) -> Result<Arc<WhisperContext>, Error> {
        self.ctx
            .get_or_try_init(|| async {
                model::ensure_model(&self.model_path).await?;
                let path = self.model_path.clone();
                let ctx = tokio::task::spawn_blocking(move || {
                    WhisperContext::new_with_params(&path, WhisperContextParameters::default())
                        .map_err(|e| Error::Model(e.to_string()))
                })
                .await
                .map_err(|e| Error::Transcribe(format!("model load join: {e}")))??;
                Ok::<_, Error>(Arc::new(ctx))
            })
            .await
            .cloned()
    }

    /// Transcribe 16 kHz mono f32 samples (produced by [`crate::resample`]).
    pub async fn transcribe(&self, samples_16k: Vec<f32>) -> Result<String, Error> {
        if samples_16k.is_empty() {
            return Ok(String::new());
        }
        let ctx = self.context().await?;
        let language = self.language.clone();
        tokio::task::spawn_blocking(move || run(&ctx, &samples_16k, language.as_deref()))
            .await
            .map_err(|e| Error::Transcribe(format!("inference join: {e}")))?
    }
}

fn run(ctx: &WhisperContext, samples: &[f32], language: Option<&str>) -> Result<String, Error> {
    let mut state = ctx
        .create_state()
        .map_err(|e| Error::Transcribe(e.to_string()))?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    // `Some("auto")` and unset both mean auto-detect.
    let lang = match language {
        Some("auto") | None => None,
        Some(l) => Some(l),
    };
    params.set_language(lang);
    params.set_print_progress(false);
    params.set_print_special(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);

    state
        .full(params, samples)
        .map_err(|e| Error::Transcribe(e.to_string()))?;

    let n = state.full_n_segments();
    let mut text = String::new();
    for i in 0..n {
        if let Some(seg) = state.get_segment(i) {
            if let Ok(s) = seg.to_str() {
                text.push_str(s);
            }
        }
    }
    Ok(text.trim().to_string())
}
