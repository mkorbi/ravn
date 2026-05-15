//! Text-Embeddings via fastembed-rs (D12: `Qwen3-Embedding-0.6B`,
//! 1024 dim, multilingual).
//!
//! `Embedder` is cheap to construct — the actual 1.2 GB model is only
//! downloaded + loaded into memory on the first call to [`Embedder::embed`].
//! Loading and inference run inside `tokio::task::spawn_blocking`
//! because candle is sync; embedding calls beyond the first take a
//! `parking_lot::Mutex` so concurrent callers serialize through the
//! single CPU-bound model.
//!
//! Phase 2 only exposes the concrete `Embedder` struct. A trait
//! abstraction will appear once we add a second backend (BGE-Small for
//! tests, OpenAI embeddings, etc.).

use std::sync::Arc;

use candle_core::{DType, Device};
use fastembed::Qwen3TextEmbedding;
use parking_lot::Mutex;
use tokio::sync::OnceCell;

#[derive(Debug, Clone)]
pub struct EmbedderConfig {
    /// Hugging Face repo, e.g. `"Qwen/Qwen3-Embedding-0.6B"`.
    pub model_id: String,
    /// Max token length per input. Inputs longer than this get
    /// truncated by the tokenizer; recommended 512 for short messages,
    /// 2048 for whole SKILL.md bodies.
    pub max_length: usize,
}

impl Default for EmbedderConfig {
    fn default() -> Self {
        Self {
            model_id: "Qwen/Qwen3-Embedding-0.6B".into(),
            max_length: 512,
        }
    }
}

/// Output dimension of the default model. Hardcoded here because the
/// `sqlite-vec` `vec0` table needs the dim at migration time.
pub const EMBEDDING_DIM: usize = 1024;

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
    /// candle's `Qwen3TextEmbedding` mutates internal state during
    /// `.embed()` (intermediate tensors); serialize concurrent calls.
    model: Mutex<Qwen3TextEmbedding>,
}

impl Embedder {
    pub fn new(config: EmbedderConfig) -> Self {
        Self {
            config,
            inner: OnceCell::new(),
        }
    }

    pub fn default_qwen3() -> Self {
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
    /// (~1.2 GB on disk; ~3 GB RAM on CPU); subsequent calls reuse it.
    /// Returns one `Vec<f32>` of length [`EMBEDDING_DIM`] per input.
    pub async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, Error> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let inner = self.ensure_loaded().await?;
        let inner_clone = inner.clone();
        let vecs = tokio::task::spawn_blocking(move || {
            let guard = inner_clone.model.lock();
            guard
                .embed(&texts)
                .map_err(|e| Error::Embed(e.to_string()))
        })
        .await
        .map_err(|e| Error::Embed(format!("join: {e}")))??;

        for (i, v) in vecs.iter().enumerate() {
            if v.len() != EMBEDDING_DIM {
                return Err(Error::WrongDim {
                    expected: EMBEDDING_DIM,
                    actual: v.len(),
                });
            }
            let _ = i;
        }
        Ok(vecs)
    }

    async fn ensure_loaded(&self) -> Result<Arc<Inner>, Error> {
        let config = self.config.clone();
        let loaded = self
            .inner
            .get_or_try_init(|| async move {
                tracing::info!(model = %config.model_id, "loading embedding model");
                let model_id = config.model_id.clone();
                let max_length = config.max_length;
                let inner = tokio::task::spawn_blocking(move || {
                    let device = Device::Cpu;
                    let dtype = DType::F32;
                    let model = Qwen3TextEmbedding::from_hf(
                        &model_id,
                        &device,
                        dtype,
                        max_length,
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
        let embedder = Embedder::default_qwen3();
        let result = embedder.embed(Vec::new()).await.unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn config_defaults_match_d12() {
        let cfg = EmbedderConfig::default();
        assert_eq!(cfg.model_id, "Qwen/Qwen3-Embedding-0.6B");
        assert_eq!(cfg.max_length, 512);
        assert_eq!(EMBEDDING_DIM, 1024);
    }

    /// Real model call. Ignored by default — downloads ~1.2 GB on first
    /// run and needs ~3 GB RAM. Run explicitly with
    /// `cargo test -p ravn-embeddings -- --ignored embeds_real_text`.
    #[tokio::test]
    #[ignore]
    async fn embeds_real_text() {
        let embedder = Embedder::default_qwen3();
        let vecs = embedder
            .embed(vec![
                "the quick brown fox".into(),
                "der schnelle braune Fuchs".into(),
            ])
            .await
            .expect("embed");
        assert_eq!(vecs.len(), 2);
        assert_eq!(vecs[0].len(), EMBEDDING_DIM);
        // The German and English versions of the same sentence should be
        // close in cosine distance (multilingual model).
        let dot: f32 = vecs[0].iter().zip(&vecs[1]).map(|(a, b)| a * b).sum();
        assert!(dot > 0.5, "expected high cosine sim, got {dot}");
    }
}
