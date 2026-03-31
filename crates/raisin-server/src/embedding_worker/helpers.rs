//! Helper functions for embedding worker.

use raisin_ai::{AIProvider, EmbeddingSettings};
use raisin_embeddings::config::EmbeddingProvider;
use raisin_error::{Error, Result};
use raisin_models::nodes::Node;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Extract embeddable content from node based on settings
pub fn extract_embeddable_content(node: &Node, settings: &EmbeddingSettings) -> Result<String> {
    let mut parts = Vec::new();

    // 1. Include node name
    if settings.include_name {
        parts.push(node.name.clone());
    }

    // 2. Include node path
    if settings.include_path {
        parts.push(node.path.clone());
    }

    // Note: Per-node-type property selection is now handled via NodeType schema
    // This legacy worker extracts all string properties as a fallback
    for (_, prop_value) in &node.properties {
        if let Some(text) = property_value_to_text(prop_value) {
            parts.push(text);
        }
    }

    // Join all parts with newlines
    Ok(parts.join("\n"))
}

/// Convert property value to text for embedding
pub fn property_value_to_text(
    value: &raisin_models::nodes::properties::PropertyValue,
) -> Option<String> {
    use raisin_models::nodes::properties::PropertyValue;

    match value {
        PropertyValue::String(s) => Some(s.clone()),
        PropertyValue::Integer(i) => Some(i.to_string()),
        PropertyValue::Float(f) => Some(f.to_string()),
        PropertyValue::Boolean(b) => Some(b.to_string()),
        PropertyValue::Date(d) => Some(d.to_string()),
        PropertyValue::Url(u) => Some(u.url.clone()),
        PropertyValue::Array(arr) => {
            let texts: Vec<String> = arr.iter().filter_map(property_value_to_text).collect();
            if texts.is_empty() {
                None
            } else {
                Some(texts.join(", "))
            }
        }
        PropertyValue::Object(obj) => {
            // Convert object to JSON string for embedding
            serde_json::to_string(obj).ok()
        }
        PropertyValue::Composite(bc) => {
            // Extract text from blocks
            let texts: Vec<String> = bc
                .items
                .iter()
                .filter_map(|block| {
                    // Try to extract text from block content
                    if let Some(text_prop) = block.content.get("text") {
                        property_value_to_text(text_prop)
                    } else {
                        None
                    }
                })
                .collect();
            if texts.is_empty() {
                None
            } else {
                Some(texts.join("\n"))
            }
        }
        PropertyValue::Element(block) => {
            // Extract text from single block
            if let Some(text_prop) = block.content.get("text") {
                property_value_to_text(text_prop)
            } else {
                None
            }
        }
        // Skip Reference and Resource types - they're not textual content
        _ => None,
    }
}

/// Hash text for detecting changes
pub fn hash_text(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// Map AI provider enum to embedding provider enum.
///
/// This is a temporary function until raisin-ai fully supports embeddings.
/// Currently we use the new TenantAIConfig for configuration but still use
/// the old raisin-embeddings crate for actual embedding generation.
pub fn map_ai_provider_to_embedding_provider(provider: &AIProvider) -> Result<EmbeddingProvider> {
    match provider {
        AIProvider::OpenAI => Ok(EmbeddingProvider::OpenAI),
        AIProvider::Ollama => Ok(EmbeddingProvider::Ollama),
        AIProvider::Anthropic
        | AIProvider::Google
        | AIProvider::AzureOpenAI
        | AIProvider::Groq
        | AIProvider::OpenRouter
        | AIProvider::Bedrock
        | AIProvider::Local
        | AIProvider::Custom => Err(Error::Validation(format!(
            "Provider {:?} does not support embeddings yet",
            provider
        ))),
    }
}
