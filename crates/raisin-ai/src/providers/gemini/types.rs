//! API request and response types for Gemini provider.

use serde::{Deserialize, Serialize};

/// Gemini models list response
#[derive(Debug, Deserialize)]
pub(super) struct GeminiModelsResponse {
    pub models: Vec<GeminiModel>,
}

/// Gemini model object
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GeminiModel {
    pub name: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub input_token_limit: Option<u64>,
    #[serde(default)]
    pub output_token_limit: Option<u64>,
    #[serde(default)]
    pub supported_generation_methods: Vec<String>,
}

/// Gemini generateContent request
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GeminiGenerateRequest {
    pub contents: Vec<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_instruction: Option<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation_config: Option<GeminiGenerationConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<GeminiTool>>,
}

/// Gemini content (message)
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct GeminiContent {
    pub role: String,
    pub parts: Vec<GeminiPart>,
}

/// Gemini content part
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub(super) enum GeminiPart {
    Text {
        text: String,
    },
    FunctionCall {
        #[serde(rename = "functionCall")]
        function_call: GeminiFunctionCall,
    },
    FunctionResponse {
        #[serde(rename = "functionResponse")]
        function_response: GeminiFunctionResponse,
    },
}

/// Gemini function call
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct GeminiFunctionCall {
    pub name: String,
    pub args: serde_json::Value,
}

/// Gemini function response
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct GeminiFunctionResponse {
    pub name: String,
    pub response: serde_json::Value,
}

/// Gemini generation config
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GeminiGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
}

/// Gemini tool declaration
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GeminiTool {
    pub function_declarations: Vec<GeminiFunctionDeclaration>,
}

/// Gemini function declaration
#[derive(Debug, Serialize)]
pub(super) struct GeminiFunctionDeclaration {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Gemini generateContent response
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GeminiGenerateResponse {
    pub candidates: Vec<GeminiCandidate>,
    #[serde(default)]
    pub usage_metadata: Option<GeminiUsageMetadata>,
    #[serde(default)]
    pub model_version: Option<String>,
}

/// Gemini response candidate
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GeminiCandidate {
    pub content: GeminiContent,
    #[serde(default)]
    pub finish_reason: Option<String>,
    #[serde(default)]
    pub safety_ratings: Option<Vec<serde_json::Value>>,
}

/// Gemini usage metadata
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GeminiUsageMetadata {
    pub prompt_token_count: u32,
    pub candidates_token_count: u32,
    pub total_token_count: u32,
}

/// Gemini API error response
#[derive(Debug, Deserialize)]
pub(super) struct GeminiError {
    pub error: GeminiErrorDetail,
}

#[derive(Debug, Deserialize)]
pub(super) struct GeminiErrorDetail {
    pub message: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub code: Option<i32>,
}
