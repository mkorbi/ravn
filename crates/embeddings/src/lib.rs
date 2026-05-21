//! Text-Embeddings via fastembed-rs (D12 revised 2026-05-20:
//! `EmbeddingGemma-300M`, 768 dim, multilingual).
//!
//! `Embedder` is cheap to construct — the actual ~300 MB ONNX model is
//! only downloaded + loaded into memory on the first call to
//! [`Embedder::embed`]. Loading and inference run inside
//! `tokio::task::spawn_blocking` because fastembed's `embed()` takes
//! `&mut self`; concurrent callers serialize through a `parking_lot::Mutex`.
//!
//! D12 history: initially picked `Qwen3-Embedding-0.6B` (1024 dim, candle
//! backend, ~1.2 GB). Revised on 2026-05-20 because the download +
//! 3 GB RAM footprint were too heavy for the personal-assistant
//! workload; EmbeddingGemma-300M is ~30× smaller and still multilingual
//! enough for session-search + skill-matching.

use std::sync::Arc;

use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use parking_lot::Mutex;
use tokio::sync::OnceCell;

#[derive(Debug, Clone)]
pub struct EmbedderConfig {
    /// Max token length per input. Inputs longer than this get
    /// truncated by the tokenizer.
    pub max_length: usize,
    /// Show the Hugging Face download progress bar on the first load.
    pub show_download_progress: bool,
}

impl Default for EmbedderConfig {
    fn default() -> Self {
        Self {
            max_length: 512,
            show_download_progress: true,
        }
    }
}

/// Output dimension of the default model. Hardcoded here because the
/// `sqlite-vec` `vec0` table needs the dim at migration time.
pub const EMBEDDING_DIM: usize = 768;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("init: {0}")]
    Init(String),
    #[error("embed: {0}")]
    Embed(String),
    #[error("wrong embedding dim: expected {expected}, got {actual}")]
    WrongDim { expected: usize, actual: usize },
}

pub struct Embedder {
    config: EmbedderConfig,
    inner: OnceCell<Arc<Inner>>,
}

struct Inner {
    /// fastembed's `embed()` takes `&mut self`. We serialize concurrent
    /// callers through a Mutex — embedding is CPU-bound, so single-
    /// threaded sequencing is fine.
    model: Mutex<TextEmbedding>,
}

impl Embedder {
    pub fn new(config: EmbedderConfig) -> Self {
        Self {
            config,
            inner: OnceCell::new(),
        }
    }

    pub fn default_gemma() -> Self {
        Self::new(EmbedderConfig::default())
    }

    /// Force the model to load now (otherwise it loads lazily on the
    /// first [`embed`]). Useful at startup if you want the
    /// model-download progress bar to appear immediately.
    pub async fn warmup(&self) -> Result<(), Error> {
        let _ = self.ensure_loaded().await?;
        Ok(())
    }

    /// Encode a batch of texts. First call downloads + loads the model
    /// (~300 MB on disk); subsequent calls reuse it. Returns one
    /// `Vec<f32>` of length [`EMBEDDING_DIM`] per input.
    pub async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, Error> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let inner = self.ensure_loaded().await?;
        let inner_clone = inner.clone();
        let vecs = tokio::task::spawn_blocking(move || {
            let mut guard = inner_clone.model.lock();
            guard
                .embed(texts, None)
                .map_err(|e| Error::Embed(e.to_string()))
        })
        .await
        .map_err(|e| Error::Embed(format!("join: {e}")))??;

        for v in &vecs {
            if v.len() != EMBEDDING_DIM {
                return Err(Error::WrongDim {
                    expected: EMBEDDING_DIM,
                    actual: v.len(),
                });
            }
        }
        Ok(vecs)
    }

    async fn ensure_loaded(&self) -> Result<Arc<Inner>, Error> {
        let config = self.config.clone();
        let loaded = self
            .inner
            .get_or_try_init(|| async move {
                tracing::info!(
                    model = "EmbeddingGemma300M",
                    "loading embedding model"
                );
                let inner = tokio::task::spawn_blocking(move || {
                    let model = TextEmbedding::try_new(
                        TextInitOptions::new(EmbeddingModel::EmbeddingGemma300M)
                            .with_max_length(config.max_length)
                            .with_show_download_progress(config.show_download_progress),
                    )
                    .map_err(|e| Error::Init(e.to_string()))?;
                    Ok::<_, Error>(Arc::new(Inner {
                        model: Mutex::new(model),
                    }))
                })
                .await
                .map_err(|e| Error::Init(format!("join: {e}")))??;
                Ok::<Arc<Inner>, Error>(inner)
            })
            .await?;
        Ok(loaded.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_input_returns_empty_output() {
        let embedder = Embedder::default_gemma();
        let result = embedder.embed(Vec::new()).await.unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn config_defaults_match_d12_revised() {
        let cfg = EmbedderConfig::default();
        assert_eq!(cfg.max_length, 512);
        assert!(cfg.show_download_progress);
        assert_eq!(EMBEDDING_DIM, 768);
    }

    /// Real model call. Ignored by default — downloads ~300 MB on first
    /// run. Run explicitly with
    /// `cargo test -p ravn-embeddings -- --ignored embeds_real_text`.
    #[tokio::test]
    #[ignore]
    async fn embeds_real_text() {
        let embedder = Embedder::default_gemma();
        let vecs = embedder
            .embed(vec![
                "the quick brown fox".into(),
                "der schnelle braune Fuchs".into(),
            ])
            .await
            .expect("embed");
        assert_eq!(vecs.len(), 2);
        assert_eq!(vecs[0].len(), EMBEDDING_DIM);
        let dot: f32 = vecs[0].iter().zip(&vecs[1]).map(|(a, b)| a * b).sum();
        assert!(dot > 0.3, "expected positive cosine sim, got {dot}");
    }
}
