//! Text embeddings for search by meaning. The model runs on this Mac; no message text
//! leaves the machine.

use crate::error::Result;

/// Blocking: call from the blocking pool, never from an async task.
pub trait Embedder: Send + Sync {
    /// Identifies the vectors' space; stored with each vector.
    fn model(&self) -> &str;
    /// The model can be used: its files are present and loading has not failed.
    fn available(&self) -> bool;
    /// Loaded and usable. Search skips meaning until then rather than wait.
    fn ready(&self) -> bool;
    /// Loads the model (idempotent). Errors leave search by meaning off.
    fn load(&self) -> Result<()>;
    /// L2-normalised vectors, one per text.
    fn passages(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
    fn query(&self, text: &str) -> Result<Vec<f32>>;
}

pub fn to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

pub fn from_blob(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

pub fn normalise(mut v: Vec<f32>) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        v.iter_mut().for_each(|x| *x /= n);
    }
    v
}

#[cfg(feature = "semantic")]
pub use e5::E5;

#[cfg(feature = "semantic")]
mod e5 {
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};

    use fastembed::{
        InitOptionsUserDefined, Pooling, QuantizationMode, TextEmbedding, TokenizerFiles, UserDefinedEmbeddingModel,
    };

    use super::{Embedder, normalise};
    use crate::error::{Error, Result};

    /// intfloat/multilingual-e5-small, int8-quantised ONNX (Xenova export). Dutch and
    /// English, 384 dimensions. Files ship inside the app bundle; see `just model`.
    pub struct E5 {
        dir: PathBuf,
        model: OnceLock<Mutex<TextEmbedding>>,
        failed: OnceLock<String>,
        loading: Mutex<()>,
    }

    pub const FILES: [&str; 5] = [
        "model_quantized.onnx",
        "tokenizer.json",
        "config.json",
        "special_tokens_map.json",
        "tokenizer_config.json",
    ];

    impl E5 {
        pub fn new(dir: PathBuf) -> Self {
            Self {
                dir,
                model: OnceLock::new(),
                failed: OnceLock::new(),
                loading: Mutex::new(()),
            }
        }

        fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
            self.load()?;
            let m = self
                .model
                .get()
                .ok_or_else(|| Error::Other("model not loaded".into()))?;
            let mut m = m.lock().map_err(|_| Error::Other("model mutex poisoned".into()))?;
            let out = m
                .embed(texts, None)
                .map_err(|e| Error::Other(format!("embedding: {e}")))?;
            Ok(out.into_iter().map(normalise).collect())
        }
    }

    impl Embedder for E5 {
        fn model(&self) -> &str {
            "multilingual-e5-small-q8"
        }

        fn available(&self) -> bool {
            self.failed.get().is_none() && FILES.iter().all(|f| self.dir.join(f).is_file())
        }

        fn ready(&self) -> bool {
            self.model.get().is_some()
        }

        fn load(&self) -> Result<()> {
            if self.model.get().is_some() {
                return Ok(());
            }
            // One load at a time; the loser of a race finds the model ready.
            let _g = self
                .loading
                .lock()
                .map_err(|_| Error::Other("model mutex poisoned".into()))?;
            if self.model.get().is_some() {
                return Ok(());
            }
            if let Some(e) = self.failed.get() {
                return Err(Error::Other(e.clone()));
            }
            let read = |f: &str| std::fs::read(self.dir.join(f));
            let built = (|| -> std::result::Result<TextEmbedding, String> {
                let model = UserDefinedEmbeddingModel::new(
                    read(FILES[0]).map_err(|e| e.to_string())?,
                    TokenizerFiles {
                        tokenizer_file: read(FILES[1]).map_err(|e| e.to_string())?,
                        config_file: read(FILES[2]).map_err(|e| e.to_string())?,
                        special_tokens_map_file: read(FILES[3]).map_err(|e| e.to_string())?,
                        tokenizer_config_file: read(FILES[4]).map_err(|e| e.to_string())?,
                    },
                )
                .with_pooling(Pooling::Mean)
                .with_quantization(QuantizationMode::Dynamic);
                // Half the cores: indexing runs in the background and must not starve the UI.
                let threads = std::thread::available_parallelism().map_or(2, |n| (n.get() / 2).max(1));
                TextEmbedding::try_new_from_user_defined(
                    model,
                    InitOptionsUserDefined::new()
                        .with_max_length(512)
                        .with_intra_threads(threads),
                )
                .map_err(|e| e.to_string())
            })();
            match built {
                Ok(m) => {
                    let _ = self.model.set(Mutex::new(m));
                    tracing::info!("search model loaded");
                    Ok(())
                }
                Err(e) => {
                    tracing::warn!("search model unavailable: {e}");
                    let _ = self.failed.set(e.clone());
                    Err(Error::Other(e))
                }
            }
        }

        fn passages(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            self.embed(texts.iter().map(|t| format!("passage: {t}")).collect())
        }

        fn query(&self, text: &str) -> Result<Vec<f32>> {
            Ok(self.embed(vec![format!("query: {text}")])?.remove(0))
        }
    }
}
