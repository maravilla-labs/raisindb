//! Ollama API request and response types.
//!
//! Contains all serde-serializable types used for communicating with the Ollama HTTP API.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Ollama API chat request format
#[derive(Debug, Serialize)]
pub(crate) struct OllamaChatRequest {
    pub model: String,
    pub messages: Vec<OllamaMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<OllamaTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<OllamaOptions>,
    /// Response format: "json" for JSON mode, or a schema object for structured output
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub(crate) struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_predict: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct OllamaMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<OllamaToolCall>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct OllamaToolCall {
    pub function: OllamaFunctionCall,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct OllamaFunctionCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub(crate) struct OllamaTool {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: OllamaFunctionDefinition,
}

#[derive(Debug, Serialize)]
pub(crate) struct OllamaFunctionDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Ollama API chat response format
#[derive(Debug, Deserialize)]
pub(crate) struct OllamaChatResponse {
    pub model: String,
    pub message: OllamaMessage,
    pub done: bool,
    #[serde(default)]
    pub total_duration: Option<u64>,
    #[serde(default)]
    pub load_duration: Option<u64>,
    #[serde(default)]
    pub prompt_eval_count: Option<u32>,
    #[serde(default)]
    pub eval_count: Option<u32>,
}

/// Ollama error response
#[derive(Debug, Deserialize)]
pub(crate) struct OllamaError {
    pub error: String,
}

/// Ollama tags (models list) response
#[derive(Debug, Deserialize)]
pub(crate) struct OllamaTagsResponse {
    pub models: Vec<OllamaModelInfo>,
}

/// Information about an Ollama model
#[derive(Debug, Deserialize)]
pub(crate) struct OllamaModelInfo {
    pub name: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub digest: String,
    #[serde(default)]
    pub modified_at: String,
    #[serde(default)]
    pub details: Option<OllamaModelDetails>,
}

/// Detailed information about an Ollama model
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct OllamaModelDetails {
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub family: String,
    #[serde(default)]
    pub families: Option<Vec<String>>,
    #[serde(default)]
    pub parameter_size: Option<String>,
    #[serde(default)]
    pub quantization_level: Option<String>,
}

/// Response from Ollama's /api/show endpoint
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct OllamaShowResponse {
    #[serde(default)]
    pub details: Option<OllamaModelDetails>,
    /// Model info contains architecture, embedding_length, pooling_type, etc.
    /// Keys are like "general.architecture", "bert.embedding_length", "bert.pooling_type"
    #[serde(default)]
    pub model_info: Option<HashMap<String, serde_json::Value>>,
}

/// Ollama embeddings request format
#[derive(Debug, Serialize)]
pub(crate) struct OllamaEmbeddingRequest {
    pub model: String,
    pub input: String,
}

/// Ollama embeddings response format
#[derive(Debug, Deserialize)]
pub(crate) struct OllamaEmbeddingResponse {
    pub model: String,
    pub embeddings: Vec<Vec<f32>>,
}
