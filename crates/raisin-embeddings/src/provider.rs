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

/// OpenAI's own embeddings host. Any OpenAI-compatible endpoint — a self-hosted server,
/// a gateway, Azure — can be substituted via [`OpenAIProvider::with_base_url`].
const OPENAI_API_BASE: &str = "https://api.openai.com/v1";

pub struct OpenAIProvider {
    api_key: String,
    model: String,
    dimensions: usize,
    base_url: String,
    client: reqwest::Client,
}

impl OpenAIProvider {
    pub fn new(api_key: String, model: String) -> Result<Self> {
        Self::build(api_key, model, None, None)
    }

    /// Point the provider at an OpenAI-compatible endpoint other than OpenAI's own.
    pub fn with_base_url(api_key: String, model: String, base_url: String) -> Result<Self> {
        Self::build(api_key, model, Some(base_url), None)
    }

    /// Full constructor.
    ///
    /// `dimensions` is needed because [`Self::get_dimensions`] only recognises OpenAI's
    /// own model names. Against any other OpenAI-compatible endpoint the model is named
    /// by whoever runs it, so the width has to come from configuration — without this,
    /// a custom `base_url` is unusable no matter what it points at.
    pub fn build(
        api_key: String,
        model: String,
        base_url: Option<String>,
        dimensions: Option<usize>,
    ) -> Result<Self> {
        let dimensions = match dimensions {
            Some(d) if d > 0 => d,
            _ => Self::get_dimensions(&model)?,
        };

        Ok(Self {
            api_key,
            model,
            dimensions,
            base_url: base_url
                .map(|u| u.trim_end_matches('/').to_string())
                .unwrap_or_else(|| OPENAI_API_BASE.to_string()),
            client: reqwest::Client::new(),
        })
    }

    fn endpoint(&self) -> String {
        format!("{}/embeddings", self.base_url)
    }

    fn get_dimensions(model: &str) -> Result<usize> {
        match model {
            "text-embedding-ada-002" => Ok(1536),
            "text-embedding-3-small" => Ok(1536),
            "text-embedding-3-large" => Ok(3072),
            _ => Err(Error::Validation(format!(
                "Unknown OpenAI model `{model}`: set `dimensions` in the embedding config \
                 when using a model this build does not know"
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
            .post(self.endpoint())
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
            .post(self.endpoint())
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

        // Sort by index before dropping it. The API does not guarantee `data` comes back
        // in request order, and the caller matches vectors to inputs positionally — so an
        // unsorted response silently attaches every embedding to the wrong text.
        let mut rows = response_data.data;
        rows.sort_by_key(|d| d.index);
        Ok(rows.into_iter().map(|d| d.embedding).collect())
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
    /// Position in the request. Load-bearing for batches — see
    /// `generate_embeddings_batch`. Defaulted so a server that omits it still parses.
    #[serde(default)]
    index: usize,
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
        Self::build(base_url, model, None)
    }

    /// Full constructor.
    ///
    /// `dimensions` overrides the built-in table. Ollama serves whatever model the
    /// operator pulled, so a fixed list of four names cannot keep up — and an unknown
    /// name is a hard error without this.
    pub fn build(base_url: String, model: String, dimensions: Option<usize>) -> Result<Self> {
        let dimensions = match dimensions {
            Some(d) if d > 0 => d,
            _ => Self::get_dimensions(&model)?,
        };

        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
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
                "Unknown Ollama model `{model}`: set `dimensions` in the embedding config \
                 when using a model this build does not know"
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

/// Create an embedding provider with an optional base URL override.
///
/// Honoured by OpenAI (and anything OpenAI-compatible) as well as Ollama.
pub fn create_provider_with_url(
    provider: &crate::config::EmbeddingProvider,
    api_key: &str,
    model: &str,
    base_url: Option<&str>,
) -> Result<Box<dyn EmbeddingProvider>> {
    create_provider_full(provider, api_key, model, base_url, None)
}

/// Create an embedding provider with a base URL *and* an explicit dimension.
///
/// Both overrides exist for the same reason: against a non-first-party endpoint, neither
/// the host nor the model name is something this crate can know in advance. The built-in
/// dimension tables only cover the vendors' own model names, so a custom endpoint needs
/// the width supplied from the tenant's config.
///
/// The dimension is a **one-way door** — it is hashed into the embedder identity and the
/// vector index is built at that width, so changing it later means reindexing everything
/// that used it.
pub fn create_provider_full(
    provider: &crate::config::EmbeddingProvider,
    api_key: &str,
    model: &str,
    base_url: Option<&str>,
    dimensions: Option<usize>,
) -> Result<Box<dyn EmbeddingProvider>> {
    match provider {
        crate::config::EmbeddingProvider::OpenAI => Ok(Box::new(OpenAIProvider::build(
            api_key.to_string(),
            model.to_string(),
            base_url.map(str::to_string),
            dimensions,
        )?)),
        crate::config::EmbeddingProvider::Claude => {
            let provider = VoyageProvider::new(api_key.to_string(), model.to_string())?;
            Ok(Box::new(provider))
        }
        crate::config::EmbeddingProvider::Ollama => Ok(Box::new(OllamaProvider::build(
            base_url.unwrap_or("http://localhost:11434").to_string(),
            model.to_string(),
            dimensions,
        )?)),
        crate::config::EmbeddingProvider::HuggingFace => Err(Error::Validation(
            "HuggingFace local embeddings require the 'candle' feature".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── base_url and dimension overrides ────────────────────────────────────────
    //
    // Both exist so a non-vendor endpoint is reachable at all. Before them, a tenant
    // could configure a custom endpoint, have it accepted, and still have every single
    // embedding request go to api.openai.com.

    #[test]
    fn openai_defaults_to_openais_own_host() {
        let p = OpenAIProvider::new("k".into(), "text-embedding-3-small".into()).unwrap();
        assert_eq!(p.endpoint(), "https://api.openai.com/v1/embeddings");
    }

    #[test]
    fn openai_honours_a_custom_base_url() {
        let p = OpenAIProvider::with_base_url(
            "k".into(),
            "text-embedding-3-small".into(),
            "https://marvel.maravilla.cloud/v1".into(),
        )
        .unwrap();
        assert_eq!(p.endpoint(), "https://marvel.maravilla.cloud/v1/embeddings");
    }

    #[test]
    fn a_trailing_slash_does_not_produce_a_double_slash() {
        let p = OpenAIProvider::with_base_url(
            "k".into(),
            "text-embedding-3-small".into(),
            "https://host/v1/".into(),
        )
        .unwrap();
        assert_eq!(p.endpoint(), "https://host/v1/embeddings");
    }

    /// The point of the override: a gateway names its own models, so the built-in table
    /// cannot possibly know them.
    #[test]
    fn an_unknown_model_works_when_dimensions_are_supplied() {
        let p = OpenAIProvider::build(
            "k".into(),
            "maravilla/embed".into(),
            Some("https://marvel.maravilla.cloud/v1".into()),
            Some(1536),
        )
        .unwrap();
        assert_eq!(p.dimensions(), 1536);
    }

    #[test]
    fn an_unknown_model_without_dimensions_still_fails_loudly() {
        // `.err()` rather than `unwrap_err()`: the latter needs `Debug` on the provider,
        // and deriving that would put the API key one `{:?}` away from a log line.
        let err = OpenAIProvider::build("k".into(), "maravilla/embed".into(), None, None)
            .err()
            .expect("an unknown model with no dimensions must fail")
            .to_string();
        assert!(
            err.contains("dimensions"),
            "the error must say how to fix it: {err}"
        );
    }

    /// A zero is a missing value, not a legitimate width — fall back rather than build a
    /// provider that reports zero-dimensional vectors.
    #[test]
    fn a_zero_dimension_override_is_ignored() {
        let p = OpenAIProvider::build("k".into(), "text-embedding-3-small".into(), None, Some(0))
            .unwrap();
        assert_eq!(p.dimensions(), 1536);
    }

    #[test]
    fn ollama_accepts_an_unknown_model_with_dimensions() {
        let p = OllamaProvider::build(
            "http://host:11434".into(),
            "some-new-model".into(),
            Some(1024),
        )
        .unwrap();
        assert_eq!(p.dimensions(), 1024);
        assert!(
            OllamaProvider::build("http://host:11434".into(), "some-new-model".into(), None)
                .is_err()
        );
    }

    /// Batch embeddings are matched to inputs by position, so an out-of-order response
    /// would attach every vector to the wrong text — silently.
    #[test]
    fn batch_rows_are_sorted_by_index_before_the_index_is_dropped() {
        let mut rows = vec![
            OpenAIEmbeddingData {
                embedding: vec![3.0],
                index: 2,
            },
            OpenAIEmbeddingData {
                embedding: vec![1.0],
                index: 0,
            },
            OpenAIEmbeddingData {
                embedding: vec![2.0],
                index: 1,
            },
        ];
        rows.sort_by_key(|d| d.index);
        let ordered: Vec<Vec<f32>> = rows.into_iter().map(|d| d.embedding).collect();
        assert_eq!(ordered, vec![vec![1.0], vec![2.0], vec![3.0]]);
    }

    #[test]
    fn the_full_factory_threads_both_overrides_through() {
        let p = create_provider_full(
            &crate::config::EmbeddingProvider::OpenAI,
            "k",
            "maravilla/embed",
            Some("https://marvel.maravilla.cloud/v1"),
            Some(1536),
        )
        .unwrap();
        assert_eq!(p.dimensions(), 1536);
    }

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
        assert_eq!(VoyageProvider::get_dimensions("voyage-3").unwrap(), 1024);
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
        assert_eq!(OllamaProvider::get_dimensions("all-minilm").unwrap(), 384);
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
