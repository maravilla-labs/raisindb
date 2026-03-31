//! Local model type identifier and capabilities.

/// Local model type identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalModel {
    /// Moondream vision-language model (default)
    Moondream,
    /// Quantized Moondream (faster CPU inference)
    MoondreamQuantized,
    /// BLIP captioner
    Blip,
    /// Quantized BLIP (fastest CPU inference)
    BlipQuantized,
    /// CLIP embedder
    Clip,
}

impl LocalModel {
    /// Parse a model ID into a LocalModel.
    ///
    /// Accepts both with and without `local:` prefix.
    pub fn from_model_id(model_id: &str) -> Option<Self> {
        let name = model_id.strip_prefix("local:").unwrap_or(model_id);
        let lower = name.to_lowercase();

        match lower.as_str() {
            "moondream" | "moondream2" => Some(LocalModel::Moondream),
            "moondream-quantized" | "moondream-q4" | "moondream-gguf" => {
                Some(LocalModel::MoondreamQuantized)
            }
            "blip" | "blip-large" => Some(LocalModel::Blip),
            "blip-quantized" | "blip-q4k" | "blip-gguf" => Some(LocalModel::BlipQuantized),
            "clip" | "clip-vit-b-32" => Some(LocalModel::Clip),
            _ => {
                // Try fuzzy matching
                if lower.contains("moondream") {
                    if lower.contains("quantized") || lower.contains("gguf") || lower.contains("q4")
                    {
                        Some(LocalModel::MoondreamQuantized)
                    } else {
                        Some(LocalModel::Moondream)
                    }
                } else if lower.contains("blip") {
                    if lower.contains("quantized") || lower.contains("gguf") || lower.contains("q4")
                    {
                        Some(LocalModel::BlipQuantized)
                    } else {
                        Some(LocalModel::Blip)
                    }
                } else if lower.contains("clip") {
                    Some(LocalModel::Clip)
                } else {
                    None
                }
            }
        }
    }

    /// Get the HuggingFace model ID for this model.
    #[cfg(feature = "candle")]
    pub fn hf_model_id(&self) -> &'static str {
        use crate::candle::{
            DEFAULT_BLIP_MODEL, DEFAULT_MOONDREAM_MODEL, QUANTIZED_BLIP_MODEL,
            QUANTIZED_MOONDREAM_MODEL,
        };
        match self {
            LocalModel::Moondream => DEFAULT_MOONDREAM_MODEL,
            LocalModel::MoondreamQuantized => QUANTIZED_MOONDREAM_MODEL,
            LocalModel::Blip => DEFAULT_BLIP_MODEL,
            LocalModel::BlipQuantized => QUANTIZED_BLIP_MODEL,
            LocalModel::Clip => crate::candle::clip::DEFAULT_CLIP_MODEL,
        }
    }

    /// Get the simple model name.
    pub fn name(&self) -> &'static str {
        match self {
            LocalModel::Moondream => "moondream",
            LocalModel::MoondreamQuantized => "moondream-quantized",
            LocalModel::Blip => "blip",
            LocalModel::BlipQuantized => "blip-quantized",
            LocalModel::Clip => "clip",
        }
    }

    /// Whether this model supports vision/image input.
    pub fn supports_vision(&self) -> bool {
        matches!(
            self,
            LocalModel::Moondream
                | LocalModel::MoondreamQuantized
                | LocalModel::Blip
                | LocalModel::BlipQuantized
                | LocalModel::Clip
        )
    }

    /// Whether this model supports embeddings.
    pub fn supports_embeddings(&self) -> bool {
        matches!(self, LocalModel::Clip)
    }

    /// Whether this model is promptable (supports custom prompts).
    pub fn is_promptable(&self) -> bool {
        matches!(self, LocalModel::Moondream | LocalModel::MoondreamQuantized)
    }
}
