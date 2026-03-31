//! Default HuggingFace model definitions.

use super::super::types::*;

/// Default models that are available for download.
pub(super) fn default_models() -> Vec<ModelInfo> {
    vec![
        // CLIP models for image embeddings
        // LAION model is the default - same size as OpenAI base but has safetensors
        ModelInfo::new(
            "laion/CLIP-ViT-B-32-laion2B-s34B-b79K",
            "CLIP ViT-B/32 (LAION)",
            ModelType::Clip,
            vec![ModelCapability::ImageEmbedding, ModelCapability::TextEmbedding],
        )
        .with_size(605_000_000)
        .with_description("LAION's CLIP model trained on LAION-2B. Includes safetensors (recommended)."),

        ModelInfo::new(
            "openai/clip-vit-large-patch14",
            "CLIP ViT-L/14",
            ModelType::Clip,
            vec![ModelCapability::ImageEmbedding, ModelCapability::TextEmbedding],
        )
        .with_size(1_710_000_000)
        .with_description("OpenAI's larger CLIP model. Better quality, includes safetensors."),

        // BLIP models for image captioning
        // Large model is the default - has safetensors support and fastest CPU inference
        ModelInfo::new(
            "Salesforce/blip-image-captioning-large",
            "BLIP Captioning Large (Recommended)",
            ModelType::Blip,
            vec![ModelCapability::ImageCaptioning],
        )
        .with_size(1_880_000_000)
        .with_description("BLIP large model for image captioning. ~3s per image on CPU (recommended)."),

        ModelInfo::new(
            "Salesforce/blip-image-captioning-base",
            "BLIP Captioning Base",
            ModelType::Blip,
            vec![ModelCapability::ImageCaptioning],
        )
        .with_size(990_000_000)
        .with_description("BLIP base model. Note: No safetensors - requires manual conversion."),

        // Quantized BLIP - smaller size but slower on CPU (useful for memory-constrained systems)
        ModelInfo::new(
            "lmz/candle-blip",
            "BLIP Captioning (Quantized Q4K)",
            ModelType::Blip,
            vec![ModelCapability::ImageCaptioning],
        )
        .with_size(271_000_000)
        .with_description("4-bit quantized BLIP. 7x smaller (271MB), but ~3x slower on CPU. Use for memory-constrained systems.")
        .quantized("blip-image-captioning-large-q4k.gguf"),

        // Moondream models - promptable vision-language model (default captioner)
        // Note: vikhyatk/moondream2 has incompatible tensor names with candle-transformers
        // Use the quantized candle-compatible version as default
        ModelInfo::new(
            "vikhyatk/moondream2",
            "Moondream 2 (Reference)",
            ModelType::Moondream,
            vec![ModelCapability::ImageCaptioning],
        )
        .with_size(3_600_000_000)
        .with_description("Original Moondream 2 model. Note: Requires tensor name mapping for candle."),

        // Quantized Moondream - smaller and faster
        ModelInfo::new(
            "santiagomed/candle-moondream",
            "Moondream (Quantized)",
            ModelType::Moondream,
            vec![ModelCapability::ImageCaptioning],
        )
        .with_size(1_800_000_000)
        .with_description("Quantized Moondream for faster CPU inference.")
        .quantized("model-q4_0.gguf"),

        // Text embedding models
        ModelInfo::new(
            "nomic-ai/nomic-embed-text-v1.5",
            "Nomic Embed Text v1.5",
            ModelType::TextEmbedding,
            vec![ModelCapability::TextEmbedding],
        )
        .with_size(270_000_000)
        .with_description("High-quality text embedding model with 768 dimensions."),

        ModelInfo::new(
            "sentence-transformers/all-MiniLM-L6-v2",
            "MiniLM-L6",
            ModelType::TextEmbedding,
            vec![ModelCapability::TextEmbedding],
        )
        .with_size(90_000_000)
        .with_description("Fast, compact text embedding model with 384 dimensions."),
    ]
}
