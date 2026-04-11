//! AWS Bedrock API provider implementation.
//!
//! Provides chat completions via the Converse API (unified across model families)
//! and embeddings via the InvokeModel API with Amazon Titan models.
//!
//! When the `bedrock` feature is enabled, uses the AWS SDK for actual API calls.
//! When disabled, returns `UnsupportedOperation` errors as a stub.
//!
//! Supported models:
//! - anthropic.claude-3-* (Claude 3 family via Bedrock)
//! - amazon.nova-* (Amazon Nova family)
//! - amazon.titan-embed-text-v2:0 (Titan Text Embeddings V2)

use crate::model_cache::{ModelCache, ModelCapabilities, ModelInfo};
use crate::provider::{AIProviderTrait, ProviderError, Result};
use crate::types::{CompletionRequest, CompletionResponse};
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;

#[cfg(feature = "bedrock")]
use crate::types::{FunctionCall, Message, Role, ToolCall, Usage};

const MODEL_CACHE_TTL: Duration = Duration::from_secs(3600);

// ── Provider struct ──────────────────────────────────────────────────

/// AWS Bedrock provider.
///
/// When the `bedrock` feature is enabled, this provider uses the AWS SDK
/// Converse API for completions and the InvokeModel API for embeddings.
/// Without the feature, all inference operations return `UnsupportedOperation`.
#[derive(Debug, Clone)]
pub struct BedrockProvider {
    /// AWS region (e.g., "us-east-1")
    region: String,
    /// AWS access key ID
    access_key_id: String,
    /// AWS secret access key
    secret_access_key: String,
    /// Optional session token for temporary credentials
    session_token: Option<String>,
    /// Model cache
    cache: Arc<ModelCache>,
    /// Bedrock Runtime client (for inference)
    #[cfg(feature = "bedrock")]
    runtime_client: aws_sdk_bedrockruntime::Client,
    /// Bedrock management client (for model listing)
    #[cfg(feature = "bedrock")]
    mgmt_client: aws_sdk_bedrock::Client,
}

impl BedrockProvider {
    /// Create a Bedrock provider from the tenant config credential format.
    ///
    /// In multi-tenant mode, credentials are stored as:
    /// - `api_key` = `"access_key_id:secret_access_key"` (colon-separated, encrypted at rest)
    /// - `region` = AWS region from `api_endpoint` field
    ///
    /// Returns an error if the credential string is not in the expected format.
    pub fn from_credentials_string(
        region: impl Into<String>,
        credentials: &str,
    ) -> std::result::Result<Self, ProviderError> {
        let parts: Vec<&str> = credentials.splitn(2, ':').collect();
        if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
            return Err(ProviderError::AuthenticationError(
                "Bedrock credentials must be in format 'ACCESS_KEY_ID:SECRET_ACCESS_KEY'".to_string(),
            ));
        }
        Ok(Self::new(region, parts[0], parts[1]))
    }

    /// Creates a new AWS Bedrock provider.
    ///
    /// # Arguments
    /// * `region` - AWS region (e.g., "us-east-1")
    /// * `access_key_id` - AWS access key ID
    /// * `secret_access_key` - AWS secret access key
    pub fn new(
        region: impl Into<String>,
        access_key_id: impl Into<String>,
        secret_access_key: impl Into<String>,
    ) -> Self {
        let region = region.into();
        let access_key_id = access_key_id.into();
        let secret_access_key = secret_access_key.into();

        #[cfg(feature = "bedrock")]
        let (runtime_client, mgmt_client) =
            build_clients(&region, &access_key_id, &secret_access_key, None);

        Self {
            region,
            access_key_id,
            secret_access_key,
            session_token: None,
            cache: Arc::new(ModelCache::with_ttl(MODEL_CACHE_TTL)),
            #[cfg(feature = "bedrock")]
            runtime_client,
            #[cfg(feature = "bedrock")]
            mgmt_client,
        }
    }

    /// Creates a new AWS Bedrock provider with temporary credentials.
    ///
    /// # Arguments
    /// * `region` - AWS region (e.g., "us-east-1")
    /// * `access_key_id` - AWS access key ID
    /// * `secret_access_key` - AWS secret access key
    /// * `session_token` - AWS session token for temporary credentials
    pub fn with_session_token(
        region: impl Into<String>,
        access_key_id: impl Into<String>,
        secret_access_key: impl Into<String>,
        session_token: impl Into<String>,
    ) -> Self {
        let region = region.into();
        let access_key_id = access_key_id.into();
        let secret_access_key = secret_access_key.into();
        let session_token = session_token.into();

        #[cfg(feature = "bedrock")]
        let (runtime_client, mgmt_client) = build_clients(
            &region,
            &access_key_id,
            &secret_access_key,
            Some(&session_token),
        );

        Self {
            region,
            access_key_id,
            secret_access_key,
            session_token: Some(session_token),
            cache: Arc::new(ModelCache::with_ttl(MODEL_CACHE_TTL)),
            #[cfg(feature = "bedrock")]
            runtime_client,
            #[cfg(feature = "bedrock")]
            mgmt_client,
        }
    }

    /// Returns the AWS region.
    pub fn region(&self) -> &str {
        &self.region
    }

    /// Returns a static list of known Bedrock models with capabilities.
    fn get_known_models() -> Vec<ModelInfo> {
        vec![
            // Anthropic Claude models on Bedrock
            ModelInfo::new(
                "anthropic.claude-3-sonnet-20240229",
                "Claude 3 Sonnet (Bedrock)",
            )
            .with_capabilities(ModelCapabilities::chat_with_tools())
            .with_context_window(200000)
            .with_max_output_tokens(4096)
            .with_metadata(serde_json::json!({
                "provider": "anthropic",
                "family": "claude-3",
                "tier": "sonnet",
                "platform": "bedrock"
            })),
            ModelInfo::new(
                "anthropic.claude-3-haiku-20240307",
                "Claude 3 Haiku (Bedrock)",
            )
            .with_capabilities(ModelCapabilities::chat_with_tools())
            .with_context_window(200000)
            .with_max_output_tokens(4096)
            .with_metadata(serde_json::json!({
                "provider": "anthropic",
                "family": "claude-3",
                "tier": "haiku",
                "platform": "bedrock"
            })),
            // Amazon Nova models
            ModelInfo::new("amazon.nova-pro-v1:0", "Amazon Nova Pro")
                .with_capabilities(ModelCapabilities::chat_with_tools())
                .with_context_window(300000)
                .with_max_output_tokens(5000)
                .with_metadata(serde_json::json!({
                    "provider": "amazon",
                    "family": "nova",
                    "tier": "pro",
                    "platform": "bedrock"
                })),
            ModelInfo::new("amazon.nova-lite-v1:0", "Amazon Nova Lite")
                .with_capabilities(ModelCapabilities {
                    chat: true,
                    streaming: true,
                    tools: true,
                    embeddings: false,
                    vision: false,
                })
                .with_context_window(300000)
                .with_max_output_tokens(5000)
                .with_metadata(serde_json::json!({
                    "provider": "amazon",
                    "family": "nova",
                    "tier": "lite",
                    "platform": "bedrock"
                })),
            // Amazon Titan Embedding model
            ModelInfo::new(
                "amazon.titan-embed-text-v2:0",
                "Amazon Titan Text Embeddings V2",
            )
            .with_capabilities(ModelCapabilities::embeddings())
            .with_metadata(serde_json::json!({
                "provider": "amazon",
                "family": "titan",
                "type": "embedding",
                "dimensions": 1024,
                "platform": "bedrock"
            })),
        ]
    }

    /// Validates that the model is supported by Bedrock.
    fn validate_model(model: &str) -> Result<()> {
        const SUPPORTED_PREFIXES: &[&str] = &[
            "anthropic.claude-3",
            "amazon.nova",
            "amazon.titan-embed",
            "meta.llama3",
            "cohere.command",
            "cohere.embed",
            "ai21.jamba",
        ];

        if SUPPORTED_PREFIXES
            .iter()
            .any(|prefix| model.starts_with(prefix))
        {
            Ok(())
        } else {
            Err(ProviderError::InvalidModel(format!(
                "Unsupported Bedrock model: {}. Supported prefixes: {}",
                model,
                SUPPORTED_PREFIXES.join(", ")
            )))
        }
    }
}

// ── Feature-gated SDK helpers ────────────────────────────────────────

#[cfg(feature = "bedrock")]
fn build_clients(
    region: &str,
    access_key_id: &str,
    secret_access_key: &str,
    session_token: Option<&str>,
) -> (
    aws_sdk_bedrockruntime::Client,
    aws_sdk_bedrock::Client,
) {
    use aws_sdk_bedrock as mgmt;
    use aws_sdk_bedrockruntime as rt;

    let session = session_token.map(String::from);

    let rt_conf = rt::config::Builder::new()
        .region(rt::config::Region::new(region.to_string()))
        .credentials_provider(rt::config::Credentials::new(
            access_key_id,
            secret_access_key,
            session.clone(),
            None,
            "raisindb",
        ))
        .behavior_version(rt::config::BehaviorVersion::latest())
        .build();

    let mgmt_conf = mgmt::config::Builder::new()
        .region(mgmt::config::Region::new(region.to_string()))
        .credentials_provider(mgmt::config::Credentials::new(
            access_key_id,
            secret_access_key,
            session,
            None,
            "raisindb",
        ))
        .behavior_version(mgmt::config::BehaviorVersion::latest())
        .build();

    (
        rt::Client::from_conf(rt_conf),
        mgmt::Client::from_conf(mgmt_conf),
    )
}

/// Converts `serde_json::Value` to `aws_smithy_types::Document`.
#[cfg(feature = "bedrock")]
fn json_to_document(value: serde_json::Value) -> aws_smithy_types::Document {
    match value {
        serde_json::Value::Null => aws_smithy_types::Document::Null,
        serde_json::Value::Bool(b) => aws_smithy_types::Document::Bool(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                if i >= 0 {
                    aws_smithy_types::Document::Number(aws_smithy_types::Number::PosInt(i as u64))
                } else {
                    aws_smithy_types::Document::Number(aws_smithy_types::Number::NegInt(i))
                }
            } else if let Some(f) = n.as_f64() {
                aws_smithy_types::Document::Number(aws_smithy_types::Number::Float(f))
            } else {
                aws_smithy_types::Document::Null
            }
        }
        serde_json::Value::String(s) => aws_smithy_types::Document::String(s),
        serde_json::Value::Array(arr) => {
            aws_smithy_types::Document::Array(arr.into_iter().map(json_to_document).collect())
        }
        serde_json::Value::Object(map) => aws_smithy_types::Document::Object(
            map.into_iter()
                .map(|(k, v)| (k, json_to_document(v)))
                .collect(),
        ),
    }
}

/// Converts `aws_smithy_types::Document` to `serde_json::Value`.
#[cfg(feature = "bedrock")]
fn document_to_json(doc: &aws_smithy_types::Document) -> serde_json::Value {
    match doc {
        aws_smithy_types::Document::Null => serde_json::Value::Null,
        aws_smithy_types::Document::Bool(b) => serde_json::Value::Bool(*b),
        aws_smithy_types::Document::Number(n) => match n {
            aws_smithy_types::Number::PosInt(i) => serde_json::json!(*i),
            aws_smithy_types::Number::NegInt(i) => serde_json::json!(*i),
            aws_smithy_types::Number::Float(f) => serde_json::json!(*f),
        },
        aws_smithy_types::Document::String(s) => serde_json::Value::String(s.clone()),
        aws_smithy_types::Document::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(document_to_json).collect())
        }
        aws_smithy_types::Document::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), document_to_json(v)))
                .collect(),
        ),
    }
}

// ── Real implementation (bedrock feature enabled) ────────────────────

#[cfg(feature = "bedrock")]
#[async_trait]
impl AIProviderTrait for BedrockProvider {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        use aws_sdk_bedrockruntime::types as bt;

        Self::validate_model(&request.model)?;

        let response_model = request.model.clone();
        let mut converse = self
            .runtime_client
            .converse()
            .model_id(request.model.as_str());

        // System prompt
        if let Some(ref system) = request.system {
            converse = converse.system(bt::SystemContentBlock::Text(system.clone()));
        }

        // Convert messages
        for msg in &request.messages {
            match msg.role {
                Role::System => {
                    converse =
                        converse.system(bt::SystemContentBlock::Text(msg.content.clone()));
                }
                Role::User | Role::Assistant => {
                    let role = if msg.role == Role::User {
                        bt::ConversationRole::User
                    } else {
                        bt::ConversationRole::Assistant
                    };

                    let mut blocks: Vec<bt::ContentBlock> = Vec::new();

                    if !msg.content.is_empty() {
                        blocks.push(bt::ContentBlock::Text(msg.content.clone()));
                    }

                    if let Some(tool_calls) = &msg.tool_calls {
                        for call in tool_calls {
                            let input: serde_json::Value =
                                serde_json::from_str(&call.function.arguments)
                                    .unwrap_or_default();
                            blocks.push(bt::ContentBlock::ToolUse(
                                bt::ToolUseBlock::builder()
                                    .tool_use_id(&call.id)
                                    .name(&call.function.name)
                                    .input(json_to_document(input))
                                    .build()
                                    .map_err(|e| {
                                        ProviderError::SerializationError(e.to_string())
                                    })?,
                            ));
                        }
                    }

                    let message = bt::Message::builder()
                        .role(role)
                        .set_content(Some(blocks))
                        .build()
                        .map_err(|e| ProviderError::SerializationError(e.to_string()))?;

                    converse = converse.messages(message);
                }
                Role::Tool => {
                    let tool_result = bt::ToolResultBlock::builder()
                        .tool_use_id(msg.tool_call_id.as_deref().unwrap_or(""))
                        .content(bt::ToolResultContentBlock::Text(msg.content.clone()))
                        .build()
                        .map_err(|e| ProviderError::SerializationError(e.to_string()))?;

                    let message = bt::Message::builder()
                        .role(bt::ConversationRole::User)
                        .content(bt::ContentBlock::ToolResult(tool_result))
                        .build()
                        .map_err(|e| ProviderError::SerializationError(e.to_string()))?;

                    converse = converse.messages(message);
                }
            }
        }

        // Inference configuration
        let mut inf_config = bt::InferenceConfiguration::builder();
        if let Some(temp) = request.temperature {
            inf_config = inf_config.temperature(temp);
        }
        if let Some(max_tokens) = request.max_tokens {
            inf_config = inf_config.max_tokens(max_tokens as i32);
        }
        converse = converse.inference_config(inf_config.build());

        // Tool configuration
        if let Some(tools) = &request.tools {
            if !tools.is_empty() {
                let bedrock_tools: std::result::Result<Vec<bt::Tool>, ProviderError> = tools
                    .iter()
                    .map(|t| {
                        let spec = bt::ToolSpecification::builder()
                            .name(&t.function.name)
                            .description(&t.function.description)
                            .input_schema(bt::ToolInputSchema::Json(json_to_document(
                                t.function.parameters.clone(),
                            )))
                            .build()
                            .map_err(|e| ProviderError::SerializationError(e.to_string()))?;
                        Ok(bt::Tool::ToolSpec(spec))
                    })
                    .collect();

                converse = converse.tool_config(
                    bt::ToolConfiguration::builder()
                        .set_tools(Some(bedrock_tools?))
                        .build()
                        .map_err(|e| ProviderError::SerializationError(e.to_string()))?,
                );
            }
        }

        // Send
        let response = converse
            .send()
            .await
            .map_err(|e| ProviderError::RequestFailed(e.to_string()))?;

        // Parse response
        let mut text_content = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();

        if let Some(output) = response.output() {
            match output {
                bt::ConverseOutput::Message(msg) => {
                    for block in msg.content() {
                        match block {
                            bt::ContentBlock::Text(text) => {
                                if !text_content.is_empty() {
                                    text_content.push('\n');
                                }
                                text_content.push_str(text);
                            }
                            bt::ContentBlock::ToolUse(tool_use) => {
                                tool_calls.push(ToolCall {
                                    id: tool_use.tool_use_id().to_string(),
                                    call_type: "function".to_string(),
                                    function: FunctionCall {
                                        name: tool_use.name().to_string(),
                                        arguments: serde_json::to_string(&document_to_json(
                                            tool_use.input(),
                                        ))
                                        .unwrap_or_default(),
                                    },
                                    index: None,
                                });
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }

        let stop_reason = Some(
            match response.stop_reason() {
                bt::StopReason::EndTurn => "end_turn",
                bt::StopReason::MaxTokens => "max_tokens",
                bt::StopReason::StopSequence => "stop_sequence",
                bt::StopReason::ToolUse => "tool_use",
                bt::StopReason::ContentFiltered => "content_filtered",
                bt::StopReason::GuardrailIntervened => "guardrail_intervened",
                _ => "unknown",
            }
            .to_string(),
        );

        let usage = response.usage().map(|u| Usage {
            prompt_tokens: u.input_tokens() as u32,
            completion_tokens: u.output_tokens() as u32,
            total_tokens: u.total_tokens() as u32,
        });

        Ok(CompletionResponse {
            message: Message {
                role: Role::Assistant,
                content: text_content,
                content_parts: None,
                tool_calls: if tool_calls.is_empty() {
                    None
                } else {
                    Some(tool_calls)
                },
                tool_call_id: None,
                name: None,
            },
            model: response_model,
            usage,
            stop_reason,
        })
    }

    fn provider_name(&self) -> &str {
        "bedrock"
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn supports_tools(&self) -> bool {
        true
    }

    fn available_models(&self) -> Vec<String> {
        vec![
            "anthropic.claude-3-sonnet-20240229".to_string(),
            "anthropic.claude-3-haiku-20240307".to_string(),
            "amazon.nova-pro-v1:0".to_string(),
            "amazon.nova-lite-v1:0".to_string(),
            "amazon.titan-embed-text-v2:0".to_string(),
        ]
    }

    async fn list_available_models(&self) -> Result<Vec<ModelInfo>> {
        if let Some(cached) = self.cache.get("bedrock").await {
            return Ok(cached);
        }

        // Try dynamic discovery via ListFoundationModels; fall back to static list
        let models = self
            .discover_models()
            .await
            .unwrap_or_else(|_| Self::get_known_models());

        let models = if models.is_empty() {
            Self::get_known_models()
        } else {
            models
        };

        self.cache.put("bedrock", models.clone()).await;
        Ok(models)
    }

    async fn generate_embedding(&self, text: &str, model: &str) -> Result<Vec<f32>> {
        let payload = serde_json::json!({
            "inputText": text,
            "dimensions": 1024,
            "normalize": true,
        });

        let response = self
            .runtime_client
            .invoke_model()
            .model_id(model)
            .body(aws_sdk_bedrockruntime::primitives::Blob::new(
                serde_json::to_vec(&payload)
                    .map_err(|e| ProviderError::SerializationError(e.to_string()))?,
            ))
            .content_type("application/json")
            .accept("application/json")
            .send()
            .await
            .map_err(|e| ProviderError::RequestFailed(e.to_string()))?;

        let body: serde_json::Value = serde_json::from_slice(response.body().as_ref())
            .map_err(|e| ProviderError::DeserializationError(e.to_string()))?;

        body["embedding"]
            .as_array()
            .ok_or_else(|| {
                ProviderError::DeserializationError(
                    "Missing 'embedding' array in Titan response".to_string(),
                )
            })?
            .iter()
            .map(|v| {
                v.as_f64().map(|f| f as f32).ok_or_else(|| {
                    ProviderError::DeserializationError("Invalid embedding value".to_string())
                })
            })
            .collect()
    }
}

/// Dynamic model discovery via the Bedrock management API.
#[cfg(feature = "bedrock")]
impl BedrockProvider {
    async fn discover_models(&self) -> Result<Vec<ModelInfo>> {
        use aws_sdk_bedrock::types as mt;

        let response = self
            .mgmt_client
            .list_foundation_models()
            .send()
            .await
            .map_err(|e| {
                ProviderError::RequestFailed(format!("ListFoundationModels failed: {}", e))
            })?;

        Ok(response
            .model_summaries()
            .iter()
            .filter(|m| {
                m.inference_types_supported()
                    .iter()
                    .any(|t| matches!(t, mt::InferenceType::OnDemand))
            })
            .map(|m| {
                let model_id = m.model_id().to_string();
                let model_name = m.model_name().unwrap_or(&model_id).to_string();

                let is_embed = model_id.contains("embed");
                let has_image_input = m
                    .input_modalities()
                    .iter()
                    .any(|mod_| matches!(mod_, mt::ModelModality::Image));
                let streaming = m.response_streaming_supported().unwrap_or(false);
                let supports_tools = model_id.starts_with("anthropic.claude-3")
                    || model_id.starts_with("amazon.nova");

                ModelInfo::new(&model_id, model_name)
                    .with_capabilities(ModelCapabilities {
                        chat: !is_embed,
                        embeddings: is_embed,
                        vision: has_image_input,
                        tools: supports_tools,
                        streaming,
                    })
                    .with_metadata(serde_json::json!({
                        "provider": m.provider_name().unwrap_or("unknown"),
                        "platform": "bedrock",
                    }))
            })
            .collect())
    }
}

// ── Stub implementation (bedrock feature disabled) ───────────────────

#[cfg(not(feature = "bedrock"))]
#[async_trait]
impl AIProviderTrait for BedrockProvider {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse> {
        Self::validate_model(&request.model)?;

        Err(ProviderError::UnsupportedOperation(
            "AWS Bedrock requires the 'bedrock' feature. \
             Enable it with: cargo build --features bedrock"
                .to_string(),
        ))
    }

    fn provider_name(&self) -> &str {
        "bedrock"
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn supports_tools(&self) -> bool {
        true
    }

    fn available_models(&self) -> Vec<String> {
        vec![
            "anthropic.claude-3-sonnet-20240229".to_string(),
            "anthropic.claude-3-haiku-20240307".to_string(),
            "amazon.nova-pro-v1:0".to_string(),
            "amazon.nova-lite-v1:0".to_string(),
            "amazon.titan-embed-text-v2:0".to_string(),
        ]
    }

    async fn list_available_models(&self) -> Result<Vec<ModelInfo>> {
        if let Some(cached) = self.cache.get("bedrock").await {
            return Ok(cached);
        }

        let models = Self::get_known_models();
        self.cache.put("bedrock", models.clone()).await;
        Ok(models)
    }

    async fn generate_embedding(&self, _text: &str, _model: &str) -> Result<Vec<f32>> {
        Err(ProviderError::UnsupportedOperation(
            "AWS Bedrock embeddings require the 'bedrock' feature. \
             Enable it with: cargo build --features bedrock"
                .to_string(),
        ))
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_creation() {
        let provider = BedrockProvider::new(
            "us-east-1",
            "AKIAIOSFODNN7EXAMPLE",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        );
        assert_eq!(provider.region(), "us-east-1");
        assert_eq!(provider.provider_name(), "bedrock");
    }

    #[test]
    fn test_provider_with_session_token() {
        let provider = BedrockProvider::with_session_token(
            "us-west-2",
            "AKIAIOSFODNN7EXAMPLE",
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            "FwoGZXIvYXdzEBYaDH...",
        );
        assert_eq!(provider.region(), "us-west-2");
        assert!(provider.session_token.is_some());
    }

    #[test]
    fn test_validate_model() {
        // Valid models
        assert!(BedrockProvider::validate_model("anthropic.claude-3-sonnet-20240229").is_ok());
        assert!(BedrockProvider::validate_model("amazon.nova-pro-v1:0").is_ok());
        assert!(BedrockProvider::validate_model("amazon.titan-embed-text-v2:0").is_ok());
        assert!(BedrockProvider::validate_model("meta.llama3-70b-instruct-v1:0").is_ok());

        // Invalid models
        assert!(BedrockProvider::validate_model("openai.gpt-4").is_err());
        assert!(BedrockProvider::validate_model("invalid-model").is_err());
    }

    #[test]
    fn test_provider_capabilities() {
        let provider = BedrockProvider::new("us-east-1", "test-key", "test-secret");
        assert_eq!(provider.provider_name(), "bedrock");
        assert!(provider.supports_streaming());
        assert!(provider.supports_tools());
        assert!(!provider.available_models().is_empty());
    }

    #[test]
    fn test_available_models() {
        let provider = BedrockProvider::new("us-east-1", "test-key", "test-secret");
        let models = provider.available_models();

        assert!(models.contains(&"anthropic.claude-3-sonnet-20240229".to_string()));
        assert!(models.contains(&"amazon.nova-pro-v1:0".to_string()));
        assert!(models.contains(&"amazon.titan-embed-text-v2:0".to_string()));
    }

    #[tokio::test]
    async fn test_list_available_models() {
        let provider = BedrockProvider::new("us-east-1", "test-key", "test-secret");
        let models = provider.list_available_models().await.unwrap();

        assert!(!models.is_empty());

        // Check that Claude model is present
        let claude_model = models
            .iter()
            .find(|m| m.id == "anthropic.claude-3-sonnet-20240229");
        assert!(claude_model.is_some());

        let claude = claude_model.unwrap();
        assert!(claude.capabilities.chat);
        assert!(claude.capabilities.tools);
        assert_eq!(claude.context_window, Some(200000));

        // Check embedding model is present
        let embed_model = models
            .iter()
            .find(|m| m.id == "amazon.titan-embed-text-v2:0");
        assert!(embed_model.is_some());
        assert!(embed_model.unwrap().capabilities.embeddings);
    }

    #[test]
    fn test_known_models_metadata() {
        let models = BedrockProvider::get_known_models();

        // Check Claude model metadata
        let claude = models
            .iter()
            .find(|m| m.id == "anthropic.claude-3-sonnet-20240229")
            .unwrap();
        assert_eq!(claude.metadata.as_ref().unwrap()["provider"], "anthropic");
        assert_eq!(claude.metadata.as_ref().unwrap()["platform"], "bedrock");

        // Check Nova model metadata
        let nova = models
            .iter()
            .find(|m| m.id == "amazon.nova-pro-v1:0")
            .unwrap();
        assert_eq!(nova.metadata.as_ref().unwrap()["provider"], "amazon");
        assert_eq!(nova.metadata.as_ref().unwrap()["family"], "nova");

        // Check Titan Embed model metadata
        let titan = models
            .iter()
            .find(|m| m.id == "amazon.titan-embed-text-v2:0")
            .unwrap();
        assert_eq!(titan.metadata.as_ref().unwrap()["type"], "embedding");
        assert!(titan.capabilities.embeddings);
        assert!(!titan.capabilities.chat);
    }

    // Stub-only tests (bedrock feature disabled)
    #[cfg(not(feature = "bedrock"))]
    #[tokio::test]
    async fn test_complete_returns_unsupported_error() {
        let provider = BedrockProvider::new("us-east-1", "test-key", "test-secret");
        let request = CompletionRequest::new(
            "anthropic.claude-3-sonnet-20240229".to_string(),
            vec![crate::types::Message::user("Hello")],
        );

        let result = provider.complete(request).await;
        assert!(matches!(
            result,
            Err(ProviderError::UnsupportedOperation(_))
        ));
    }

    #[cfg(not(feature = "bedrock"))]
    #[tokio::test]
    async fn test_embedding_returns_unsupported_error() {
        let provider = BedrockProvider::new("us-east-1", "test-key", "test-secret");
        let result = provider
            .generate_embedding("test", "amazon.titan-embed-text-v2:0")
            .await;

        assert!(matches!(
            result,
            Err(ProviderError::UnsupportedOperation(_))
        ));
    }

    // SDK-specific tests (bedrock feature enabled)
    #[cfg(feature = "bedrock")]
    mod sdk_tests {
        use super::super::*;

        #[test]
        fn test_json_to_document_roundtrip() {
            let json = serde_json::json!({
                "name": "test",
                "count": 42,
                "negative": -5,
                "pi": 3.14,
                "active": true,
                "items": [1, 2, 3],
                "nested": {"key": "value"},
                "empty": null,
            });

            let doc = json_to_document(json.clone());
            let back = document_to_json(&doc);

            assert_eq!(json, back);
        }

        #[test]
        fn test_json_to_document_empty_object() {
            let json = serde_json::json!({});
            let doc = json_to_document(json.clone());
            let back = document_to_json(&doc);
            assert_eq!(json, back);
        }
    }
}
