//! Embedder module for generating code embeddings.
//!
//! When the `semantic` feature is enabled, uses the `ort` ONNX runtime with
//! [Nomic Embed Text v1](https://huggingface.co/nomic-ai/nomic-embed-text-v1)
//! (`onnx/model_quantized.onnx`) to generate 768-dimensional embeddings.
//!
//! **Why not `nomic-ai/nomic-embed-code`?** That repository ships Transformers /
//! safetensors weights only (no official ONNX export on Hugging Face), and its
//! sentence-transformers head uses **last-token pooling** with **3584**-dim
//! outputs-different from this module’s mean pooling and fixed 768-dim storage.
//! When disabled, all operations return `EmbedError::NotEnabled`.

use std::path::{Path, PathBuf};

/// Embedding dimension (matches Nomic Embed Text v1 ONNX export).
pub const EMBEDDING_DIM: usize = 768;

/// Hugging Face repo that provides both the ONNX model and `tokenizer.json`.
const HF_MODEL_REPO: &str = "nomic-ai/nomic-embed-text-v1";

/// Remote path within the repo for the int8 ONNX graph (same artifact we cache locally).
const MODEL_REMOTE_PATH: &str = "onnx/model_quantized.onnx";

/// Remote tokenizer path (repo root).
const TOKENIZER_REMOTE_PATH: &str = "tokenizer.json";

/// Cached ONNX file name (same as `onnx/model_quantized.onnx` on Hugging Face).
const MODEL_FILENAME: &str = "model_quantized.onnx";

/// Cached tokenizer file name (same as repo-root `tokenizer.json` on Hugging Face).
const TOKENIZER_FILENAME: &str = "tokenizer.json";

/// Build a `.../resolve/main/...` Hugging Face download URL.
fn hf_resolve_url(repo: &str, file_path: &str) -> String {
    format!("https://huggingface.co/{repo}/resolve/main/{file_path}")
}

/// Error type for embedding operations.
#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    /// Semantic search is not enabled (model not downloaded or feature not compiled).
    #[error(
        "semantic search is not enabled. Run `cortex semantic enable` to download the model (requires building with --features semantic)."
    )]
    NotEnabled,

    /// Model loading failed.
    #[error("failed to load embedding model: {0}")]
    ModelLoadFailed(String),

    /// Embedding generation failed.
    #[error("embedding generation failed: {0}")]
    GenerationFailed(String),

    /// Download failed.
    #[error("model download failed: {0}")]
    DownloadFailed(String),
}

// ===========================================================================
// Feature-gated implementation
// ===========================================================================

#[cfg(feature = "semantic")]
mod inner {
    use super::*;
    use ndarray::{Array1, Array2, ArrayView1};
    use ort::session::Session;
    use tokenizers::Tokenizer;

    /// Embedder struct that generates code embeddings using ONNX runtime.
    pub struct Embedder {
        session: Session,
        tokenizer: Tokenizer,
    }

    impl Embedder {
        /// Create a new Embedder by loading the ONNX model and tokenizer.
        ///
        /// Returns `EmbedError::NotEnabled` if model files are not present.
        pub fn new(data_dir: &Path) -> Result<Self, EmbedError> {
            let model_path = model_path(data_dir);
            let tokenizer_path = tokenizer_path(data_dir);

            if !model_path.exists() || !tokenizer_path.exists() {
                return Err(EmbedError::NotEnabled);
            }

            let session = Session::builder()
                .map_err(|e| EmbedError::ModelLoadFailed(format!("session builder: {e}")))?
                .with_intra_threads(1)
                .map_err(|e| EmbedError::ModelLoadFailed(format!("set threads: {e}")))?
                .commit_from_file(&model_path)
                .map_err(|e| {
                    EmbedError::ModelLoadFailed(format!(
                        "load model '{}': {e}",
                        model_path.display()
                    ))
                })?;

            let tokenizer = Tokenizer::from_file(&tokenizer_path).map_err(|e| {
                EmbedError::ModelLoadFailed(format!(
                    "load tokenizer '{}': {e}",
                    tokenizer_path.display()
                ))
            })?;

            Ok(Self { session, tokenizer })
        }

        /// Check if the embedder is enabled (always true if constructed successfully).
        pub fn is_enabled(&self) -> bool {
            true
        }

        /// Generate an embedding for a code snippet.
        ///
        /// Tokenizes the input, runs ONNX inference, and returns a normalized
        /// 768-dimensional f32 vector.
        pub fn generate_embedding(&self, code_snippet: &str) -> Result<Vec<f32>, EmbedError> {
            // Tokenize
            let encoding = self
                .tokenizer
                .encode(code_snippet, true)
                .map_err(|e| EmbedError::GenerationFailed(format!("tokenization: {e}")))?;

            let input_ids: Vec<i64> = encoding.get_ids().iter().map(|&id| id as i64).collect();
            let attention_mask: Vec<i64> = encoding
                .get_attention_mask()
                .iter()
                .map(|&m| m as i64)
                .collect();
            let token_type_ids: Vec<i64> =
                encoding.get_type_ids().iter().map(|&t| t as i64).collect();

            let seq_len = input_ids.len();

            // Create 2D arrays [1, seq_len]
            let input_ids_arr = Array2::from_shape_vec((1, seq_len), input_ids)
                .map_err(|e| EmbedError::GenerationFailed(format!("shape input_ids: {e}")))?;
            let attention_mask_arr = Array2::from_shape_vec((1, seq_len), attention_mask.clone())
                .map_err(|e| {
                EmbedError::GenerationFailed(format!("shape attention_mask: {e}"))
            })?;
            let token_type_ids_arr = Array2::from_shape_vec((1, seq_len), token_type_ids)
                .map_err(|e| EmbedError::GenerationFailed(format!("shape token_type_ids: {e}")))?;

            // Run inference
            let outputs = self
                .session
                .run(ort::inputs![
                    "input_ids" => input_ids_arr,
                    "attention_mask" => attention_mask_arr,
                    "token_type_ids" => token_type_ids_arr,
                ])
                .map_err(|e| EmbedError::GenerationFailed(format!("inference: {e}")))?;

            // Extract the output tensor - shape [1, seq_len, 768]
            let output_tensor = outputs[0]
                .try_extract_tensor::<f32>()
                .map_err(|e| EmbedError::GenerationFailed(format!("extract tensor: {e}")))?;

            // Mean pooling with attention mask
            let output_view = output_tensor.view();
            let shape = output_view.shape();

            if shape.len() < 2 {
                return Err(EmbedError::GenerationFailed(
                    "unexpected output shape".to_string(),
                ));
            }

            let hidden_size = shape[shape.len() - 1];
            let embedding = if shape.len() == 3 {
                // [1, seq_len, hidden_size] - mean pool over seq_len with attention mask
                let seq_len_out = shape[1];
                let mut pooled = Array1::<f32>::zeros(hidden_size);
                let mut mask_sum = 0.0f32;

                for i in 0..seq_len_out {
                    let mask_val = if i < attention_mask.len() {
                        attention_mask[i] as f32
                    } else {
                        0.0
                    };
                    mask_sum += mask_val;
                    for j in 0..hidden_size {
                        pooled[j] += output_view[[0, i, j]] * mask_val;
                    }
                }

                if mask_sum > 0.0 {
                    pooled /= mask_sum;
                }
                pooled
            } else {
                // [1, hidden_size] - already pooled
                let mut pooled = Array1::<f32>::zeros(hidden_size);
                for j in 0..hidden_size {
                    pooled[j] = output_view[[0, j]];
                }
                pooled
            };

            // L2 normalize
            let embedding = l2_normalize(embedding.view());

            Ok(embedding.to_vec())
        }

        /// Generate embeddings for multiple code snippets (batched for efficiency).
        pub fn generate_embeddings(&self, snippets: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError> {
            // For simplicity, process one at a time. Batching can be added later.
            snippets
                .iter()
                .map(|s| self.generate_embedding(s))
                .collect()
        }
    }

    /// L2 normalize a vector.
    fn l2_normalize(v: ArrayView1<f32>) -> Array1<f32> {
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 1e-12 {
            v.to_owned() / norm
        } else {
            v.to_owned()
        }
    }
}

#[cfg(not(feature = "semantic"))]
mod inner {
    use super::*;

    /// Stub Embedder when the `semantic` feature is not enabled.
    #[derive(Debug)]
    pub struct Embedder {
        _enabled: bool,
    }

    impl Embedder {
        /// Create a new Embedder (stub - always disabled).
        pub fn new(_data_dir: &Path) -> Result<Self, EmbedError> {
            Err(EmbedError::NotEnabled)
        }

        /// Check if the embedder is enabled (always false without feature).
        pub fn is_enabled(&self) -> bool {
            false
        }

        /// Generate an embedding (stub - always returns NotEnabled).
        pub fn generate_embedding(&self, _code_snippet: &str) -> Result<Vec<f32>, EmbedError> {
            Err(EmbedError::NotEnabled)
        }

        /// Generate embeddings for multiple snippets (stub).
        pub fn generate_embeddings(&self, _snippets: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError> {
            Err(EmbedError::NotEnabled)
        }
    }
}

// Re-export the Embedder from the appropriate inner module
pub use inner::Embedder;

// ===========================================================================
// Shared functions (available regardless of feature)
// ===========================================================================

/// Get the path where the ONNX model should be stored.
pub fn model_path(data_dir: &Path) -> PathBuf {
    data_dir.join("models").join(MODEL_FILENAME)
}

/// Get the path where the tokenizer should be stored.
pub fn tokenizer_path(data_dir: &Path) -> PathBuf {
    data_dir.join("models").join(TOKENIZER_FILENAME)
}

/// Check if the semantic search model is available.
pub fn is_model_available(data_dir: &Path) -> bool {
    model_path(data_dir).exists() && tokenizer_path(data_dir).exists()
}

/// Enable semantic search by downloading the model and tokenizer.
///
/// Downloads Nomic Embed Text v1 ONNX (`model_quantized.onnx`, ~138MB) and
/// `tokenizer.json` from `nomic-ai/nomic-embed-text-v1` into the data directory.
pub fn enable(data_dir: &Path) -> Result<(), EmbedError> {
    let model_dir = data_dir.join("models");
    std::fs::create_dir_all(&model_dir).map_err(|e| {
        EmbedError::ModelLoadFailed(format!("failed to create model directory: {e}"))
    })?;

    let mp = model_path(data_dir);
    let tp = tokenizer_path(data_dir);

    if mp.exists() && tp.exists() {
        println!("Semantic search: model already downloaded.");
        return Ok(());
    }

    let model_url = hf_resolve_url(HF_MODEL_REPO, MODEL_REMOTE_PATH);
    let tokenizer_url = hf_resolve_url(HF_MODEL_REPO, TOKENIZER_REMOTE_PATH);

    println!("Downloading Nomic Embed Text v1 ONNX ({MODEL_FILENAME}, ~138MB)...");
    println!("  Repo: {HF_MODEL_REPO}");
    println!("  Model URL: {model_url}");
    println!("  Tokenizer URL: {tokenizer_url}");

    // Download tokenizer
    if !tp.exists() {
        download_file(&tokenizer_url, &tp)?;
        println!("  ✓ Tokenizer downloaded to {}", tp.display());
    }

    // Download model
    if !mp.exists() {
        download_file(&model_url, &mp)?;
        println!("  ✓ Model downloaded to {}", mp.display());
    }

    println!("Semantic search enabled successfully.");
    Ok(())
}

/// Disable semantic search by removing the model files.
pub fn disable(data_dir: &Path) -> Result<(), EmbedError> {
    let mp = model_path(data_dir);
    let tp = tokenizer_path(data_dir);

    if mp.exists() {
        std::fs::remove_file(&mp)
            .map_err(|e| EmbedError::ModelLoadFailed(format!("failed to remove model: {e}")))?;
    }
    if tp.exists() {
        std::fs::remove_file(&tp)
            .map_err(|e| EmbedError::ModelLoadFailed(format!("failed to remove tokenizer: {e}")))?;
    }

    println!("Semantic search disabled.");
    Ok(())
}

/// Get the status of semantic search.
pub fn status(data_dir: &Path) -> String {
    if is_model_available(data_dir) {
        "Semantic search: enabled (model present)".to_string()
    } else {
        "Semantic search: disabled (model not present). Run `cortex semantic enable` to download."
            .to_string()
    }
}

/// Download a file from a URL to a local path.
fn download_file(url: &str, dest: &Path) -> Result<(), EmbedError> {
    // Use a simple synchronous HTTP download via std::process::Command (curl/wget)
    // This avoids adding a heavy HTTP client dependency.
    let status = std::process::Command::new("curl")
        .args(["-fSL", "--progress-bar", "-o"])
        .arg(dest.as_os_str())
        .arg(url)
        .status()
        .map_err(|e| {
            EmbedError::DownloadFailed(format!("failed to execute curl (is curl installed?): {e}"))
        })?;

    if !status.success() {
        // Clean up partial download
        let _ = std::fs::remove_file(dest);
        return Err(EmbedError::DownloadFailed(format!(
            "curl exited with status {status} for URL: {url}"
        )));
    }

    Ok(())
}

/// Compute cosine similarity between two vectors.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a < 1e-12 || norm_b < 1e-12 {
        return 0.0;
    }

    dot / (norm_a * norm_b)
}

/// Prepare a text representation of a node for embedding.
///
/// Combines the FQN with the first few lines of code to create a meaningful
/// text for embedding generation.
pub fn prepare_node_text(fqn: &str, code_snippet: Option<&str>) -> String {
    match code_snippet {
        Some(code) => {
            // Take first 512 chars of code to keep embedding input manageable
            let truncated: String = code.chars().take(512).collect();
            format!("{}\n{}", fqn, truncated)
        }
        None => fqn.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_model_not_present_returns_not_enabled() {
        let tmp = TempDir::new().unwrap();
        let result = Embedder::new(tmp.path());
        assert!(result.is_err());
        match result.unwrap_err() {
            EmbedError::NotEnabled => {} // expected
            other => panic!("expected NotEnabled, got: {other}"),
        }
    }

    #[test]
    fn test_status_when_disabled() {
        let tmp = TempDir::new().unwrap();
        let s = status(tmp.path());
        assert!(s.contains("disabled"));
    }

    #[test]
    fn test_is_model_available_false() {
        let tmp = TempDir::new().unwrap();
        assert!(!is_model_available(tmp.path()));
    }

    #[test]
    fn test_cosine_similarity_identical() {
        let v = vec![1.0, 0.0, 0.0];
        let sim = cosine_similarity(&v, &v);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![-1.0, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim + 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_empty() {
        let a: Vec<f32> = vec![];
        let b: Vec<f32> = vec![];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    #[test]
    fn test_cosine_similarity_different_lengths() {
        let a = vec![1.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    #[test]
    fn test_prepare_node_text_with_code() {
        let text = prepare_node_text(
            "src/auth.rs::validate_user",
            Some("fn validate_user(input: &str) -> bool {\n    !input.is_empty()\n}"),
        );
        assert!(text.contains("src/auth.rs::validate_user"));
        assert!(text.contains("fn validate_user"));
    }

    #[test]
    fn test_prepare_node_text_without_code() {
        let text = prepare_node_text("src/auth.rs::validate_user", None);
        assert_eq!(text, "src/auth.rs::validate_user");
    }

    #[test]
    fn test_enable_creates_model_directory() {
        let tmp = TempDir::new().unwrap();
        // We can't actually download in tests, but we can verify the directory is created
        // The download will fail (no network in CI), but the directory should exist
        let _ = enable(tmp.path());
        assert!(tmp.path().join("models").exists());
    }

    #[test]
    fn test_disable_when_no_model() {
        let tmp = TempDir::new().unwrap();
        let result = disable(tmp.path());
        assert!(result.is_ok());
    }
}
