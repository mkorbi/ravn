//! Locating (and, on first use, downloading) the ggml Whisper model.
//!
//! Resolution order: `RAVN_WHISPER_MODEL` env var → `<data_dir>/whisper/
//! ggml-base.bin`. A missing default model is downloaded from Hugging Face on
//! first use (mirrors how `ravn-embeddings` fetches its ONNX model lazily).

use std::path::{Path, PathBuf};

use crate::error::Error;

/// Multilingual base model (~142 MB) — a good speed/quality balance for a
/// personal-assistant workload, matching the multilingual embedding choice.
const MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin";
const MODEL_FILE: &str = "ggml-base.bin";

/// Resolve the model path: `RAVN_WHISPER_MODEL` override, else the default
/// under the data dir.
pub fn resolve_model_path(data_dir: &Path) -> PathBuf {
    if let Ok(p) = std::env::var("RAVN_WHISPER_MODEL") {
        return PathBuf::from(p);
    }
    data_dir.join("whisper").join(MODEL_FILE)
}

/// Ensure the model exists at `path`, downloading the default model if it is
/// missing. A user-supplied `RAVN_WHISPER_MODEL` that doesn't exist is an error
/// (we only auto-download the known default URL).
pub async fn ensure_model(path: &Path) -> Result<(), Error> {
    if path.exists() {
        return Ok(());
    }
    if std::env::var_os("RAVN_WHISPER_MODEL").is_some() {
        return Err(Error::Model(format!(
            "RAVN_WHISPER_MODEL points at a missing file: {}",
            path.display()
        )));
    }
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|e| Error::Io(e.to_string()))?;
    }
    tracing::info!(
        url = MODEL_URL,
        dest = %path.display(),
        "downloading whisper model (~142 MB, first run only)"
    );
    let resp = reqwest::get(MODEL_URL)
        .await
        .map_err(|e| Error::Model(format!("download request: {e}")))?;
    if !resp.status().is_success() {
        return Err(Error::Model(format!("download HTTP {}", resp.status())));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| Error::Model(format!("download body: {e}")))?;
    // Write to a temp file then rename, so an interrupted download never
    // leaves a half-written model that looks valid.
    let tmp = path.with_extension("part");
    tokio::fs::write(&tmp, &bytes)
        .await
        .map_err(|e| Error::Io(e.to_string()))?;
    tokio::fs::rename(&tmp, path)
        .await
        .map_err(|e| Error::Io(e.to_string()))?;
    tracing::info!(dest = %path.display(), bytes = bytes.len(), "whisper model ready");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // One test (not two) because both paths mutate the same process-global
    // env var, which would race under cargo's parallel test threads.
    #[test]
    fn resolves_default_then_env_override() {
        std::env::remove_var("RAVN_WHISPER_MODEL");
        let default = resolve_model_path(Path::new("/data/ravn"));
        assert!(default.ends_with("whisper/ggml-base.bin"));

        std::env::set_var("RAVN_WHISPER_MODEL", "/models/custom.bin");
        assert_eq!(
            resolve_model_path(Path::new("/data/ravn")),
            PathBuf::from("/models/custom.bin")
        );
        std::env::remove_var("RAVN_WHISPER_MODEL");
    }
}
