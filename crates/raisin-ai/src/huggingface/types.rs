//! Types for HuggingFace model management.

use serde::{Deserialize, Serialize};

/// Type of HuggingFace model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelType {
    /// CLIP models for image embeddings
    Clip,
    /// BLIP models for image captioning
    Blip,
    /// Moondream models for promptable image captioning
    Moondream,
    /// Text embedding models (e.g., nomic-embed, sentence-transformers)
    TextEmbedding,
    /// OCR models for text extraction from images
    Ocr,
    /// Whisper models for audio transcription
    Whisper,
}

impl std::fmt::Display for ModelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelType::Clip => write!(f, "CLIP"),
            ModelType::Blip => write!(f, "BLIP"),
            ModelType::Moondream => write!(f, "Moondream"),
            ModelType::TextEmbedding => write!(f, "Text Embedding"),
            ModelType::Ocr => write!(f, "OCR"),
            ModelType::Whisper => write!(f, "Whisper"),
        }
    }
}

/// Capabilities that a model supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelCapability {
    /// Generate image embeddings
    ImageEmbedding,
    /// Generate text embeddings
    TextEmbedding,
    /// Generate image captions
    ImageCaptioning,
    /// Perform OCR on images/PDFs
    Ocr,
    /// Transcribe audio
    AudioTranscription,
}

/// Download status for a model.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadStatus {
    /// Model is not downloaded
    NotDownloaded,
    /// Model download is in progress
    Downloading {
        /// Progress as a fraction (0.0 to 1.0)
        progress: f32,
        /// Bytes downloaded so far
        downloaded_bytes: u64,
        /// Total bytes to download (if known)
        total_bytes: Option<u64>,
    },
    /// Model is fully downloaded and ready to use
    Ready,
    /// Model download failed
    Failed {
        /// Error message
        error: String,
    },
}

impl DownloadStatus {
    /// Check if the model is ready to use.
    pub fn is_ready(&self) -> bool {
        matches!(self, DownloadStatus::Ready)
    }

    /// Check if a download is in progress.
    pub fn is_downloading(&self) -> bool {
        matches!(self, DownloadStatus::Downloading { .. })
    }

    /// Get download progress as a percentage (0-100).
    pub fn progress_percent(&self) -> Option<u32> {
        match self {
            DownloadStatus::Downloading { progress, .. } => Some((progress * 100.0) as u32),
            DownloadStatus::Ready => Some(100),
            _ => None,
        }
    }
}

/// Information about a HuggingFace model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    /// HuggingFace model ID (e.g., "openai/clip-vit-base-patch32")
    pub model_id: String,

    /// Display name for the UI
    pub display_name: String,

    /// Type of model
    pub model_type: ModelType,

    /// Capabilities this model provides
    pub capabilities: Vec<ModelCapability>,

    /// Estimated size in bytes (for display)
    pub estimated_size_bytes: Option<u64>,

    /// Actual size on disk (if downloaded)
    pub actual_size_bytes: Option<u64>,

    /// Current download status
    pub status: DownloadStatus,

    /// Description of the model
    pub description: Option<String>,

    /// URL to model card on HuggingFace
    pub model_url: String,

    /// Whether this is a quantized model (uses GGUF format)
    #[serde(default)]
    pub is_quantized: bool,

    /// GGUF filename for quantized models
    #[serde(default)]
    pub gguf_filename: Option<String>,
}

impl ModelInfo {
    /// Create a new model info entry.
    pub fn new(
        model_id: impl Into<String>,
        display_name: impl Into<String>,
        model_type: ModelType,
        capabilities: Vec<ModelCapability>,
    ) -> Self {
        let model_id = model_id.into();
        let model_url = format!("https://huggingface.co/{}", model_id);
        Self {
            model_id,
            display_name: display_name.into(),
            model_type,
            capabilities,
            estimated_size_bytes: None,
            actual_size_bytes: None,
            status: DownloadStatus::NotDownloaded,
            description: None,
            model_url,
            is_quantized: false,
            gguf_filename: None,
        }
    }

    /// Mark as a quantized model with GGUF format.
    pub fn quantized(mut self, gguf_filename: impl Into<String>) -> Self {
        self.is_quantized = true;
        self.gguf_filename = Some(gguf_filename.into());
        self
    }

    /// Check if the model is downloaded and ready.
    pub fn is_downloaded(&self) -> bool {
        self.status.is_ready()
    }

    /// Set the estimated size.
    pub fn with_size(mut self, size_bytes: u64) -> Self {
        self.estimated_size_bytes = Some(size_bytes);
        self
    }

    /// Set the description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Get a human-readable size string.
    pub fn size_display(&self) -> String {
        let bytes = self.actual_size_bytes.or(self.estimated_size_bytes);
        match bytes {
            Some(b) if b >= 1_000_000_000 => format!("{:.1} GB", b as f64 / 1_000_000_000.0),
            Some(b) if b >= 1_000_000 => format!("{:.1} MB", b as f64 / 1_000_000.0),
            Some(b) if b >= 1_000 => format!("{:.1} KB", b as f64 / 1_000.0),
            Some(b) => format!("{} B", b),
            None => "Unknown".to_string(),
        }
    }
}

/// Progress callback for model downloads.
pub type ProgressCallback = Box<dyn Fn(f32) + Send + Sync>;

/// Errors that can occur during model operations.
#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("Model not found: {0}")]
    NotFound(String),

    #[error("Download failed: {0}")]
    DownloadFailed(String),

    #[error("Model already downloading: {0}")]
    AlreadyDownloading(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Cache directory not found")]
    CacheDirectoryNotFound,

    #[error("HuggingFace Hub error: {0}")]
    HubError(String),
}

/// Result type for model operations.
pub type ModelResult<T> = Result<T, ModelError>;
