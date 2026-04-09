//! Embedding provider implementations.
//!
//! This module provides abstractions for calling external embedding APIs
//! such as OpenAI, Cohere, and HuggingFace.

use async_trait::async_trait;
use raisin_error::{Error, Result};
use serde::{Deserialize, Serialize};

/// Trait for embedding generation providers
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Generate an embedding for the given text
    async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>>;

    /// Generate embeddings for multiple texts in a single batch
    async fn generate_embeddings_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        // Default implementation: call generate_embedding for each text
        let mut embeddings = Vec::with_capacity(texts.len());
        for text in texts {
            embeddings.push(self.generate_embedding(text).await?);
        }
        Ok(embeddings)
    }

    /// Get the embedding dimensions for this provider/model
    fn dimensions(&self) -> usize;

    /// Test connectivity to the embedding provider by generating a test embedding
    async fn test_connection(&self) -> Result<usize> {
        let embedding = self.generate_embedding("test").await?;
        if embedding.len() != self.dimensions() {
            return Err(Error::Backend(format!(
                "Dimension mismatch: expected {}, got {}",
                self.dimensions(),
                embedding.len()
            )));
        }
        Ok(embedding.len())
    }
}

// ---------------------------------------------------------------------------
// OpenAI
// ---------------------------------------------------------------------------

pub struct OpenAIProvider {
    api_key: String,
    model: String,
    dimensions: usize,
    client: reqwest::Client,
}

impl OpenAIProvider {
    pub fn new(api_key: String, model: String) -> Result<Self> {
        let dimensions = Self::get_dimensions(&model)?;

        Ok(Self {
            api_key,
            model,
            dimensions,
            client: reqwest::Client::new(),
        })
    }

    fn get_dimensions(model: &str) -> Result<usize> {
        match model {
            "text-embedding-ada-002" => Ok(1536),
            "text-embedding-3-small" => Ok(1536),
            "text-embedding-3-large" => Ok(3072),
            _ => Err(Error::Validation(format!(
                "Unknown OpenAI model: {}",
                model
            ))),
        }
    }

    fn extract_error_message(error_text: &str) -> String {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(error_text) {
            if let Some(error_obj) = json.get("error") {
                if let Some(message) = error_obj.get("message") {
                    if let Some(msg_str) = message.as_str() {
                        return msg_str.to_string();
                    }
                }
            }
        }

        error_text.to_string()
    }
}

#[async_trait]
impl EmbeddingProvider for OpenAIProvider {
    async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>> {
        let request = OpenAIEmbeddingRequest {
            input: text.to_string(),
            model: self.model.clone(),
        };

        let response = self
            .client
            .post("https://api.openai.com/v1/embeddings")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| Error::Backend(format!("OpenAI API request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            let error_message = Self::extract_error_message(&error_text);
            return Err(Error::Backend(format!(
                "OpenAI API error {}: {}",
                status, error_message
            )));
        }

        let response_data: OpenAIEmbeddingResponse = response
            .json()
            .await
            .map_err(|e| Error::Backend(format!("Failed to parse OpenAI response: {}", e)))?;

        if response_data.data.is_empty() {
            return Err(Error::Backend(
                "No embeddings returned from OpenAI".to_string(),
            ));
        }

        Ok(response_data.data[0].embedding.clone())
    }

    async fn generate_embeddings_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let request = OpenAIEmbeddingBatchRequest {
            input: texts.to_vec(),
            model: self.model.clone(),
        };

        let response = self
            .client
            .post("https://api.openai.com/v1/embeddings")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| Error::Backend(format!("OpenAI API request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            let error_message = Self::extract_error_message(&error_text);
            return Err(Error::Backend(format!(
                "OpenAI API error {}: {}",
                status, error_message
            )));
        }

        let response_data: OpenAIEmbeddingResponse = response
            .json()
            .await
            .map_err(|e| Error::Backend(format!("Failed to parse OpenAI response: {}", e)))?;

        if response_data.data.len() != texts.len() {
            return Err(Error::Backend(format!(
                "Expected {} embeddings, got {}",
                texts.len(),
                response_data.data.len()
            )));
        }

        Ok(response_data
            .data
            .into_iter()
            .map(|d| d.embedding)
            .collect())
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }
}

#[derive(Debug, Serialize)]
struct OpenAIEmbeddingRequest {
    input: String,
    model: String,
}

#[derive(Debug, Serialize)]
struct OpenAIEmbeddingBatchRequest {
    input: Vec<String>,
    model: String,
}

#[derive(Debug, Deserialize)]
struct OpenAIEmbeddingResponse {
    data: Vec<OpenAIEmbeddingData>,
}

#[derive(Debug, Deserialize)]
struct OpenAIEmbeddingData {
    embedding: Vec<f32>,
}

// ---------------------------------------------------------------------------
// Voyage AI (Claude variant)
// ---------------------------------------------------------------------------

pub struct VoyageProvider {
    api_key: String,
    model: String,
    dimensions: usize,
    client: reqwest::Client,
}

impl VoyageProvider {
    pub fn new(api_key: String, model: String) -> Result<Self> {
        let dimensions = Self::get_dimensions(&model)?;

        Ok(Self {
            api_key,
            model,
            dimensions,
            client: reqwest::Client::new(),
        })
    }

    fn get_dimensions(model: &str) -> Result<usize> {
        match model {
            "voyage-large-2-instruct" => Ok(1024),
            "voyage-code-2" => Ok(1536),
            "voyage-3" => Ok(1024),
            "voyage-3-lite" => Ok(512),
            _ => Err(Error::Validation(format!(
                "Unknown Voyage AI model: {}",
                model
            ))),
        }
    }

    fn extract_error_message(error_text: &str) -> String {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(error_text) {
            if let Some(detail) = json.get("detail") {
                if let Some(msg_str) = detail.as_str() {
                    return msg_str.to_string();
                }
            }
            if let Some(error_obj) = json.get("error") {
                if let Some(message) = error_obj.get("message") {
                    if let Some(msg_str) = message.as_str() {
                        return msg_str.to_string();
                    }
                }
            }
        }

        error_text.to_string()
    }
}

#[async_trait]
impl EmbeddingProvider for VoyageProvider {
    async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>> {
        let request = VoyageEmbeddingRequest {
            input: vec![text.to_string()],
            model: self.model.clone(),
        };

        let response = self
            .client
            .post("https://api.voyageai.com/v1/embeddings")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| Error::Backend(format!("Voyage AI API request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            let error_message = Self::extract_error_message(&error_text);
            return Err(Error::Backend(format!(
                "Voyage AI API error {}: {}",
                status, error_message
            )));
        }

        let response_data: VoyageEmbeddingResponse = response
            .json()
            .await
            .map_err(|e| Error::Backend(format!("Failed to parse Voyage AI response: {}", e)))?;

        if response_data.data.is_empty() {
            return Err(Error::Backend(
                "No embeddings returned from Voyage AI".to_string(),
            ));
        }

        Ok(response_data.data[0].embedding.clone())
    }

    async fn generate_embeddings_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let request = VoyageEmbeddingRequest {
            input: texts.to_vec(),
            model: self.model.clone(),
        };

        let response = self
            .client
            .post("https://api.voyageai.com/v1/embeddings")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| Error::Backend(format!("Voyage AI API request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            let error_message = Self::extract_error_message(&error_text);
            return Err(Error::Backend(format!(
                "Voyage AI API error {}: {}",
                status, error_message
            )));
        }

        let response_data: VoyageEmbeddingResponse = response
            .json()
            .await
            .map_err(|e| Error::Backend(format!("Failed to parse Voyage AI response: {}", e)))?;

        if response_data.data.len() != texts.len() {
            return Err(Error::Backend(format!(
                "Expected {} embeddings, got {}",
                texts.len(),
                response_data.data.len()
            )));
        }

        Ok(response_data
            .data
            .into_iter()
            .map(|d| d.embedding)
            .collect())
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }
}

#[derive(Debug, Serialize)]
struct VoyageEmbeddingRequest {
    input: Vec<String>,
    model: String,
}

#[derive(Debug, Deserialize)]
struct VoyageEmbeddingResponse {
    data: Vec<VoyageEmbeddingData>,
}

#[derive(Debug, Deserialize)]
struct VoyageEmbeddingData {
    embedding: Vec<f32>,
}

// ---------------------------------------------------------------------------
// Ollama
// ---------------------------------------------------------------------------

pub struct OllamaProvider {
    base_url: String,
    model: String,
    dimensions: usize,
    client: reqwest::Client,
}

impl OllamaProvider {
    pub fn new(model: String) -> Result<Self> {
        Self::with_base_url("http://localhost:11434".to_string(), model)
    }

    pub fn with_base_url(base_url: String, model: String) -> Result<Self> {
        let dimensions = Self::get_dimensions(&model)?;

        Ok(Self {
            base_url,
            model,
            dimensions,
            client: reqwest::Client::new(),
        })
    }

    fn get_dimensions(model: &str) -> Result<usize> {
        match model {
            "nomic-embed-text" => Ok(768),
            "all-minilm" => Ok(384),
            "mxbai-embed-large" => Ok(1024),
            "snowflake-arctic-embed" => Ok(1024),
            _ => Err(Error::Validation(format!(
                "Unknown Ollama model: {}",
                model
            ))),
        }
    }

    fn extract_error_message(error_text: &str) -> String {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(error_text) {
            if let Some(error) = json.get("error") {
                if let Some(msg_str) = error.as_str() {
                    return msg_str.to_string();
                }
            }
        }

        error_text.to_string()
    }
}

#[async_trait]
impl EmbeddingProvider for OllamaProvider {
    async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>> {
        let request = OllamaEmbedRequest {
            model: self.model.clone(),
            input: vec![text.to_string()],
        };

        let url = format!("{}/api/embed", self.base_url);

        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| Error::Backend(format!("Ollama API request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            let error_message = Self::extract_error_message(&error_text);
            return Err(Error::Backend(format!(
                "Ollama API error {}: {}",
                status, error_message
            )));
        }

        let response_data: OllamaEmbedResponse = response
            .json()
            .await
            .map_err(|e| Error::Backend(format!("Failed to parse Ollama response: {}", e)))?;

        if response_data.embeddings.is_empty() {
            return Err(Error::Backend(
                "No embeddings returned from Ollama".to_string(),
            ));
        }

        Ok(response_data.embeddings[0].clone())
    }

    async fn generate_embeddings_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let request = OllamaEmbedRequest {
            model: self.model.clone(),
            input: texts.to_vec(),
        };

        let url = format!("{}/api/embed", self.base_url);

        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| Error::Backend(format!("Ollama API request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            let error_message = Self::extract_error_message(&error_text);
            return Err(Error::Backend(format!(
                "Ollama API error {}: {}",
                status, error_message
            )));
        }

        let response_data: OllamaEmbedResponse = response
            .json()
            .await
            .map_err(|e| Error::Backend(format!("Failed to parse Ollama response: {}", e)))?;

        if response_data.embeddings.len() != texts.len() {
            return Err(Error::Backend(format!(
                "Expected {} embeddings, got {}",
                texts.len(),
                response_data.embeddings.len()
            )));
        }

        Ok(response_data.embeddings)
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }
}

#[derive(Debug, Serialize)]
struct OllamaEmbedRequest {
    model: String,
    input: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct OllamaEmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Create an embedding provider based on the provider type.
///
/// For Ollama, `api_key` is ignored (optional, local service).
/// Use `create_provider_with_url` to specify a custom Ollama base URL.
pub fn create_provider(
    provider: &crate::config::EmbeddingProvider,
    api_key: &str,
    model: &str,
) -> Result<Box<dyn EmbeddingProvider>> {
    create_provider_with_url(provider, api_key, model, None)
}

/// Create an embedding provider with optional base URL override.
///
/// The `base_url` parameter is used by Ollama to connect to a remote instance.
/// For other providers, it is ignored.
pub fn create_provider_with_url(
    provider: &crate::config::EmbeddingProvider,
    api_key: &str,
    model: &str,
    base_url: Option<&str>,
) -> Result<Box<dyn EmbeddingProvider>> {
    match provider {
        crate::config::EmbeddingProvider::OpenAI => {
            let provider = OpenAIProvider::new(api_key.to_string(), model.to_string())?;
            Ok(Box::new(provider))
        }
        crate::config::EmbeddingProvider::Claude => {
            let provider = VoyageProvider::new(api_key.to_string(), model.to_string())?;
            Ok(Box::new(provider))
        }
        crate::config::EmbeddingProvider::Ollama => {
            let provider = if let Some(url) = base_url {
                OllamaProvider::with_base_url(url.to_string(), model.to_string())?
            } else {
                OllamaProvider::new(model.to_string())?
            };
            Ok(Box::new(provider))
        }
        crate::config::EmbeddingProvider::HuggingFace => Err(Error::Validation(
            "HuggingFace local embeddings require the 'candle' feature".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openai_get_dimensions() {
        assert_eq!(
            OpenAIProvider::get_dimensions("text-embedding-ada-002").unwrap(),
            1536
        );
        assert_eq!(
            OpenAIProvider::get_dimensions("text-embedding-3-small").unwrap(),
            1536
        );
        assert_eq!(
            OpenAIProvider::get_dimensions("text-embedding-3-large").unwrap(),
            3072
        );
        assert!(OpenAIProvider::get_dimensions("unknown-model").is_err());
    }

    #[test]
    fn test_voyage_get_dimensions() {
        assert_eq!(
            VoyageProvider::get_dimensions("voyage-large-2-instruct").unwrap(),
            1024
        );
        assert_eq!(
            VoyageProvider::get_dimensions("voyage-code-2").unwrap(),
            1536
        );
        assert_eq!(
            VoyageProvider::get_dimensions("voyage-3").unwrap(),
            1024
        );
        assert_eq!(
            VoyageProvider::get_dimensions("voyage-3-lite").unwrap(),
            512
        );
        assert!(VoyageProvider::get_dimensions("unknown-model").is_err());
    }

    #[test]
    fn test_ollama_get_dimensions() {
        assert_eq!(
            OllamaProvider::get_dimensions("nomic-embed-text").unwrap(),
            768
        );
        assert_eq!(
            OllamaProvider::get_dimensions("all-minilm").unwrap(),
            384
        );
        assert_eq!(
            OllamaProvider::get_dimensions("mxbai-embed-large").unwrap(),
            1024
        );
        assert_eq!(
            OllamaProvider::get_dimensions("snowflake-arctic-embed").unwrap(),
            1024
        );
        assert!(OllamaProvider::get_dimensions("unknown-model").is_err());
    }

    #[test]
    fn test_create_provider_openai() {
        let provider = create_provider(
            &crate::config::EmbeddingProvider::OpenAI,
            "test-key",
            "text-embedding-3-small",
        );
        assert!(provider.is_ok());
        assert_eq!(provider.unwrap().dimensions(), 1536);
    }

    #[test]
    fn test_create_provider_voyage() {
        let provider = create_provider(
            &crate::config::EmbeddingProvider::Claude,
            "test-key",
            "voyage-3",
        );
        assert!(provider.is_ok());
        assert_eq!(provider.unwrap().dimensions(), 1024);
    }

    #[test]
    fn test_create_provider_ollama() {
        let provider = create_provider(
            &crate::config::EmbeddingProvider::Ollama,
            "",
            "nomic-embed-text",
        );
        assert!(provider.is_ok());
        assert_eq!(provider.unwrap().dimensions(), 768);
    }

    #[test]
    fn test_create_provider_huggingface_errors() {
        let result = create_provider(
            &crate::config::EmbeddingProvider::HuggingFace,
            "",
            "some-model",
        );
        match result {
            Err(e) => assert!(e.to_string().contains("candle")),
            Ok(_) => panic!("Expected error for HuggingFace provider"),
        }
    }
}
