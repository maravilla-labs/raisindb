//! Ollama model discovery and conversion.
//!
//! Handles fetching model information from the Ollama API and converting
//! Ollama model metadata into the internal ModelInfo format.

use super::api_types::*;
use super::OllamaProvider;
use crate::model_cache::{ModelCapabilities, ModelInfo};
use crate::provider::{ProviderError, Result};

impl OllamaProvider {
    /// Fetches detailed model info from Ollama's /api/show endpoint
    pub(crate) async fn fetch_model_show(&self, model_name: &str) -> Result<OllamaShowResponse> {
        let request = self
            .client
            .post(format!("{}/show", self.base_url))
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({ "name": model_name }));
        let request = self.add_auth_header(request);
        let response = request.send().await.map_err(|e| {
            ProviderError::NetworkError(format!("Failed to fetch model info: {}", e))
        })?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(ProviderError::RequestFailed(format!(
                "Failed to fetch model info for '{}': HTTP {}: {}",
                model_name, status, error_text
            )));
        }

        response
            .json()
            .await
            .map_err(|e| ProviderError::DeserializationError(e.to_string()))
    }

    /// Determines if a model is an embedding model based on /api/show response
    /// Checks for:
    /// 1. BERT-family architectures (bert, nomic-bert, xlm, roberta)
    /// 2. Presence of pooling_type field (indicates pooled embeddings)
    pub(crate) fn is_embedding_model(show: &OllamaShowResponse) -> bool {
        if let Some(model_info) = &show.model_info {
            // Check architecture for known embedding model families
            if let Some(arch) = model_info.get("general.architecture") {
                let arch_str = arch.as_str().unwrap_or("").to_lowercase();
                // BERT family (all-minilm, nomic-embed-text, etc.)
                // XLM-RoBERTa family (multilingual embeddings)
                if arch_str.contains("bert")
                    || arch_str.contains("xlm")
                    || arch_str.contains("roberta")
                {
                    return true;
                }
            }
            // Check for pooling_type (embedding indicator)
            for key in model_info.keys() {
                if key.ends_with(".pooling_type") {
                    return true;
                }
            }
        }
        false
    }

    /// Fetches the list of locally installed models from Ollama
    /// For each model, calls /api/show to get detailed info for capability detection
    pub(crate) async fn fetch_models(&self) -> Result<Vec<ModelInfo>> {
        let request = self.client.get(format!("{}/tags", self.base_url));
        let request = self.add_auth_header(request);
        let response = request.send().await.map_err(|e| {
            ProviderError::ProviderNotAvailable(format!(
                "Failed to connect to Ollama: {}. Is Ollama running?",
                e
            ))
        })?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(ProviderError::RequestFailed(format!(
                "Failed to fetch Ollama models: HTTP {}: {}",
                status, error_text
            )));
        }

        let tags_response: OllamaTagsResponse = response
            .json()
            .await
            .map_err(|e| ProviderError::DeserializationError(e.to_string()))?;

        // Convert Ollama models to our ModelInfo format
        // Fetch /api/show for each model to detect embedding capability
        let mut models = Vec::with_capacity(tags_response.models.len());
        for model in tags_response.models {
            // Fetch detailed model info for capability detection
            let show_response = self.fetch_model_show(&model.name).await.ok();
            let model_info = self.convert_ollama_model_with_show(model, show_response.as_ref());
            models.push(model_info);
        }

        Ok(models)
    }

    /// Converts an Ollama model to our ModelInfo format with show response for capability detection
    fn convert_ollama_model_with_show(
        &self,
        model: OllamaModelInfo,
        show: Option<&OllamaShowResponse>,
    ) -> ModelInfo {
        // Check if model supports tools based on name patterns
        let supports_tools = Self::model_supports_tools_by_name(&model.name);

        // Detect embedding capability from /api/show response
        let is_embedding = show.map(Self::is_embedding_model).unwrap_or(false);

        // Extract architecture from show response
        let architecture = show
            .and_then(|s| s.model_info.as_ref())
            .and_then(|m| m.get("general.architecture"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Extract embedding dimensions if available
        let embedding_length = show
            .and_then(|s| s.model_info.as_ref())
            .and_then(|m| {
                // Try various architecture prefixes for embedding_length
                m.get("bert.embedding_length")
                    .or_else(|| m.get("nomic-bert.embedding_length"))
                    .or_else(|| m.get("xlm-roberta.embedding_length"))
            })
            .and_then(|v| v.as_u64());

        let capabilities = ModelCapabilities {
            // Embedding models are NOT chat models
            chat: !is_embedding,
            embeddings: is_embedding,
            vision: model.name.contains("vision") || model.name.contains("llava"),
            // Embedding models don't support tools
            tools: supports_tools && !is_embedding,
            // Embedding models don't support streaming
            streaming: !is_embedding,
        };

        // Extract context window from details if available
        let context_window = model
            .details
            .as_ref()
            .and_then(|d| d.parameter_size.as_ref())
            .map(|size| {
                // Estimate based on parameter size
                if size.contains("70B")
                    || size.contains("70b")
                    || size.contains("7B")
                    || size.contains("7b")
                {
                    4096
                } else {
                    2048
                }
            });

        // Build metadata with raw show response for debugging
        let mut metadata = serde_json::json!({
            "size": model.size,
            "modified_at": model.modified_at,
            "digest": model.digest,
            "details": model.details,
        });

        // Add architecture info if available
        if let Some(arch) = architecture {
            metadata["architecture"] = serde_json::json!(arch);
        }
        if let Some(emb_len) = embedding_length {
            metadata["embedding_length"] = serde_json::json!(emb_len);
        }
        // Store raw model_info for debugging
        if let Some(show) = show {
            if let Some(model_info) = &show.model_info {
                metadata["ollama_model_info"] = serde_json::json!(model_info);
            }
        }

        ModelInfo::new(model.name.clone(), model.name)
            .with_capabilities(capabilities)
            .with_context_window(context_window.unwrap_or(2048))
            .with_metadata(metadata)
    }

    /// Converts an Ollama model to our ModelInfo format (legacy, without show response)
    #[allow(dead_code)]
    fn convert_ollama_model(&self, model: OllamaModelInfo) -> ModelInfo {
        self.convert_ollama_model_with_show(model, None)
    }
}
