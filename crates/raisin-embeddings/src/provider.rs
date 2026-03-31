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
}

/// OpenAI embedding provider
pub struct OpenAIProvider {
    api_key: String,
    model: String,
    dimensions: usize,
    client: reqwest::Client,
}

impl OpenAIProvider {
    /// Create a new OpenAI provider
    pub fn new(api_key: String, model: String) -> Result<Self> {
        let dimensions = Self::get_dimensions(&model)?;

        Ok(Self {
            api_key,
            model,
            dimensions,
            client: reqwest::Client::new(),
        })
    }

    /// Get embedding dimensions for a model
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

    /// Extract the error message from OpenAI's JSON error response
    ///
    /// OpenAI returns errors in the format:
    /// ```json
    /// {
    ///   "error": {
    ///     "message": "...",
    ///     "type": "...",
    ///     "code": "..."
    ///   }
    /// }
    /// ```
    ///
    /// This function extracts just the message field for cleaner error display.
    fn extract_error_message(error_text: &str) -> String {
        // Try to parse as JSON and extract the error message
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(error_text) {
            if let Some(error_obj) = json.get("error") {
                if let Some(message) = error_obj.get("message") {
                    if let Some(msg_str) = message.as_str() {
                        return msg_str.to_string();
                    }
                }
            }
        }

        // Fall back to returning the full error text if parsing fails
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
        // OpenAI supports batch embedding generation
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

/// Create an embedding provider based on the provider type
pub fn create_provider(
    provider: &crate::config::EmbeddingProvider,
    api_key: &str,
    model: &str,
) -> Result<Box<dyn EmbeddingProvider>> {
    match provider {
        crate::config::EmbeddingProvider::OpenAI => {
            let provider = OpenAIProvider::new(api_key.to_string(), model.to_string())?;
            Ok(Box::new(provider))
        }
        crate::config::EmbeddingProvider::Claude => Err(Error::Validation(
            "Claude provider not implemented yet".to_string(),
        )),
        crate::config::EmbeddingProvider::Ollama => Err(Error::Validation(
            "Ollama provider not implemented yet".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_dimensions() {
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
}
